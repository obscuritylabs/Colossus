use std::{future::Future, time::Duration};

use colossus_native_browser::{BrowserError, PageState};
use futures_util::{StreamExt, stream::FuturesUnordered};

pub(super) const SNAPSHOT_BUDGET: Duration = Duration::from_secs(1);

/// Read all views concurrently within one budget, retaining completed observations.
/// Dropping pending futures on timeout leaves no inspection tasks holding the UI lock.
pub(super) async fn collect_pages<F>(
    inspections: impl IntoIterator<Item = F>,
    budget: Duration,
) -> Vec<(String, PageState)>
where
    F: Future<Output = (String, Result<PageState, BrowserError>)>,
{
    let mut pending: FuturesUnordered<_> = inspections.into_iter().collect();
    let mut pages = Vec::new();
    let _ = tokio::time::timeout(budget, async {
        while let Some((id, result)) = pending.next().await {
            if let Ok(page) = result {
                pages.push((id, page));
            }
        }
    })
    .await;
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn hung_views_share_one_deadline_and_do_not_hide_ready_views() {
        let mut senders = Vec::new();
        let mut inspections = Vec::new();
        for index in 0..super::super::registry::MAX_TABS {
            let (send, receive) = oneshot::channel();
            inspections.push(async move { (index.to_string(), receive.await.unwrap()) });
            senders.push(send);
        }
        let ready = senders.pop().unwrap();
        ready.send(Ok(PageState::default())).unwrap();
        let pages = tokio::time::timeout(
            Duration::from_secs(1),
            collect_pages(inspections, Duration::from_millis(20)),
        )
        .await
        .expect("one shared deadline must release the browser operation");
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0].0,
            (super::super::registry::MAX_TABS - 1).to_string()
        );
        assert!(senders.iter().all(oneshot::Sender::is_closed));
    }

    #[tokio::test]
    async fn failed_views_do_not_discard_other_observations() {
        let pages = collect_pages(
            [false, true].map(|ready| async move {
                (
                    ready.to_string(),
                    if ready {
                        Ok(PageState::default())
                    } else {
                        Err(BrowserError::Closed)
                    },
                )
            }),
            SNAPSHOT_BUDGET,
        )
        .await;
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].0, "true");
    }
}
