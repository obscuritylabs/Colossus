//! Classify explicit close from the canonical feed, without renderer authority.
use colossus_sdk::{
    Colossus, Run, RunStatus, RunUpdate, RunUpdateKind, ToolActivity, ToolActivityState,
    WatchRunRequest,
};
use futures_util::{Stream, StreamExt as _};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::Instant;

const MAX_UPDATES: u64 = 4096;
const REPLAY_TIMEOUT: Duration = Duration::from_secs(2);

/// `false` requires a complete canonical replay proving no matching explicit close.
/// Unavailable, malformed, incomplete or over-budget evidence is never absence.
pub(super) async fn explicitly_closed(
    client: &Colossus,
    run: &Run,
    session_id: &str,
) -> Option<bool> {
    if session_id.is_empty()
        || run.run_id.is_empty()
        || run.terminal.is_none()
        || !matches!(
            run.status,
            RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::Interrupted
                | RunStatus::OutcomeUnknown
        )
        || !(1..=MAX_UPDATES).contains(&run.last_sequence)
    {
        return None;
    }
    let deadline = Instant::now() + REPLAY_TIMEOUT;
    // The SDK rejects cross-run updates and sequence gaps. Dropping this read-only
    // watcher at the deadline disconnects observation; it never cancels a run.
    let mut updates = tokio::time::timeout_at(
        deadline,
        client.watch_run(WatchRunRequest {
            run_id: run.run_id.clone(),
            after_sequence: 0,
        }),
    )
    .await
    .ok()?
    .ok()?;
    classify(
        &mut updates,
        &run.run_id,
        run.last_sequence,
        session_id,
        deadline,
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseReceipt<'a> {
    session_id: &'a str,
    closed: bool,
}

fn matching_close(activity: &ToolActivity, session_id: &str) -> Option<bool> {
    if activity.tool_name != "browser.close" || activity.state != ToolActivityState::Completed {
        return Some(false);
    }
    let receipt: CloseReceipt<'_> = serde_json::from_str(activity.preview.as_deref()?).ok()?;
    Some(receipt.closed && receipt.session_id == session_id)
}

async fn classify<S, E>(
    updates: &mut S,
    run_id: &str,
    last_sequence: u64,
    session_id: &str,
    deadline: Instant,
) -> Option<bool>
where
    S: Stream<Item = Result<RunUpdate, E>> + Unpin,
{
    if !(1..=MAX_UPDATES).contains(&last_sequence) {
        return None;
    }
    tokio::time::timeout_at(deadline, async {
        let mut closed = false;
        for sequence in 1..=last_sequence {
            // Ready streams can otherwise process many items without yielding to
            // the timeout future. Bound that synchronous work by the same deadline.
            if Instant::now() >= deadline {
                return None;
            }
            let update = updates.next().await?.ok()?;
            if update.run_id != run_id || update.sequence != sequence {
                return None;
            }
            if let RunUpdateKind::ToolActivity(activity) = update.update {
                closed |= matching_close(&activity, session_id)?;
            }
        }
        // Do not classify a suffix, a truncated replay, or a feed that advanced
        // beyond the exact terminal GetRun summary as proven absence.
        if updates.next().await.is_some() {
            return None;
        }
        Some(closed)
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn activity(tool: &str, state: ToolActivityState, preview: Option<&str>) -> ToolActivity {
        ToolActivity {
            call_id: "call".into(),
            tool_name: tool.into(),
            state,
            summary: String::new(),
            input: None,
            preview: preview.map(str::to_owned),
        }
    }

    fn update(sequence: u64, activity: Option<ToolActivity>) -> RunUpdate {
        RunUpdate {
            run_id: "run".into(),
            sequence,
            created_at: String::new(),
            update: activity.map_or(
                RunUpdateKind::State(RunStatus::Completed),
                RunUpdateKind::ToolActivity,
            ),
        }
    }

    fn close(sequence: u64) -> RunUpdate {
        update(
            sequence,
            Some(activity(
                "browser.close",
                ToolActivityState::Completed,
                Some(r#"{"session_id":"owned","closed":true}"#),
            )),
        )
    }

    async fn replay(items: Vec<Result<RunUpdate, ()>>, last_sequence: u64) -> Option<bool> {
        classify(
            &mut stream::iter(items),
            "run",
            last_sequence,
            "owned",
            Instant::now() + REPLAY_TIMEOUT,
        )
        .await
    }

    #[test]
    fn matches_only_successful_exact_session_receipts() {
        let receipt = r#"{"session_id":"owned","closed":true}"#;
        let completed = ToolActivityState::Completed;
        assert_eq!(
            matching_close(
                &activity("browser.close", completed, Some(receipt)),
                "owned"
            ),
            Some(true)
        );
        assert_eq!(
            matching_close(
                &activity("browser.close", completed, Some(receipt)),
                "other"
            ),
            Some(false)
        );
        assert_eq!(
            matching_close(
                &activity("browser.tab.close", completed, Some(receipt)),
                "owned"
            ),
            Some(false)
        );
        assert_eq!(
            matching_close(
                &activity("browser.close", ToolActivityState::Started, Some(receipt)),
                "owned"
            ),
            Some(false)
        );
        assert_eq!(
            matching_close(
                &activity("browser.close", ToolActivityState::Failed, Some(receipt)),
                "owned"
            ),
            Some(false)
        );
        assert_eq!(
            matching_close(
                &activity(
                    "browser.close",
                    completed,
                    Some(r#"{"session_id":"owned","closed":false}"#)
                ),
                "owned"
            ),
            Some(false)
        );
    }

    #[test]
    fn malformed_or_missing_success_receipts_are_not_absence() {
        for preview in [
            None,
            Some(""),
            Some("preview truncated"),
            Some(r#"{"session_id":"owned","closed":"true"}"#),
            Some(r#"{"session_id":"owned","closed":true,"extra":1}"#),
            Some(r#"{"session_id":"owned","closed":false,"closed":true}"#),
        ] {
            assert_eq!(
                matching_close(
                    &activity("browser.close", ToolActivityState::Completed, preview),
                    "owned"
                ),
                None
            );
        }
    }

    #[tokio::test]
    async fn complete_replays_distinguish_explicit_and_automatic_cleanup() {
        assert_eq!(
            replay(vec![Ok(close(1)), Ok(update(2, None))], 2).await,
            Some(true)
        );
        assert_eq!(
            replay(vec![Ok(update(1, None)), Ok(update(2, None))], 2).await,
            Some(false)
        );
        let maximum = (1..=MAX_UPDATES)
            .map(|sequence| Ok(update(sequence, None)))
            .collect();
        assert_eq!(replay(maximum, MAX_UPDATES).await, Some(false));
    }

    #[tokio::test]
    async fn incomplete_or_inconsistent_replays_never_prove_absence() {
        assert_eq!(replay(vec![Ok(close(1))], 2).await, None);
        assert_eq!(
            replay(vec![Ok(update(1, None)), Ok(update(2, None))], 1).await,
            None
        );
        assert_eq!(replay(vec![Ok(update(2, None))], 1).await, None);
        let mut foreign = update(1, None);
        foreign.run_id = "foreign".into();
        assert_eq!(replay(vec![Ok(foreign)], 1).await, None);
        assert_eq!(replay(vec![Err(())], 1).await, None);
        assert_eq!(replay(vec![], MAX_UPDATES + 1).await, None);
        assert_eq!(
            replay(
                vec![Ok(update(
                    1,
                    Some(activity(
                        "browser.close",
                        ToolActivityState::Completed,
                        None
                    )),
                ))],
                1,
            )
            .await,
            None
        );
    }

    #[tokio::test]
    async fn replay_including_wait_for_exhaustion_is_time_bounded() {
        let mut ready = stream::iter([Ok::<_, ()>(update(1, None))]);
        assert_eq!(
            classify(&mut ready, "run", 1, "owned", Instant::now()).await,
            None
        );
        let mut stalled = stream::pending::<Result<RunUpdate, ()>>();
        assert_eq!(
            classify(
                &mut stalled,
                "run",
                1,
                "owned",
                Instant::now() + Duration::from_millis(10)
            )
            .await,
            None
        );
        let first = stream::iter([Ok::<_, ()>(update(1, None))]);
        let mut stalled_end = first.chain(stream::pending());
        assert_eq!(
            classify(
                &mut stalled_end,
                "run",
                1,
                "owned",
                Instant::now() + Duration::from_millis(10)
            )
            .await,
            None
        );
    }
}
