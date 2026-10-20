use super::{Owner, RemoteHostOwner, State};
use crate::{BrowserError, presentation::PresentationSurface};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) struct Pending(pub Option<Arc<dyn RemoteHostOwner>>);
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(native) = self.0.take() {
            tauri::async_runtime::spawn(async move {
                let _ = native.close().await;
            });
        }
    }
}
pub(super) fn hide(owner: &Arc<Owner>) -> Result<(), BrowserError> {
    let (surface, lease, revision) = {
        let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        let revision = owner
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        state.visible = false;
        state.applied = None;
        (state.surface.clone(), state.lease, revision)
    };
    let result = surface.as_ref().map_or(Ok(()), |surface| surface.hide());
    let owner = Arc::clone(owner);
    tauri::async_runtime::spawn(async move {
        let _operation = owner.operation.lock().await;
        if current_hide_failed(
            &owner.state,
            &owner.revision,
            revision,
            lease,
            hide_host(&owner, lease),
        )
        .await
        {
            owner.client.disconnect();
        }
    });
    result
}

async fn current_hide_failed(
    state: &Mutex<State>,
    revision: &AtomicU64,
    expected: u64,
    lease: colossus_browser_presentation::Lease,
    hide: impl Future<Output = Result<(), BrowserError>>,
) -> bool {
    let current = || {
        state.lock().is_ok_and(|state| {
            state.lease == lease && !state.visible && revision.load(Ordering::Acquire) == expected
        })
    };
    // Local pixels/input were already revoked synchronously. A later show owns
    // fresh configuration; this obsolete queued hide cannot revoke that revision.
    if !current() {
        return false;
    }
    hide.await.is_err() && current()
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

pub(super) async fn close(owner: &Arc<Owner>) -> Result<(), BrowserError> {
    owner.closing.store(true, Ordering::Release);
    #[cfg(windows)]
    owner.completed.clear();
    hide(owner)?;
    let _operation = owner.operation.lock().await;
    let (surface, lease) = {
        let state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        (state.surface.clone(), state.lease)
    };
    if let Some(surface) = surface {
        surface.hide_confirmed().await?;
    }
    let _ = hide_host(owner, lease).await;
    owner.client.disconnect();
    // The separate native owner must positively acknowledge detach/cleanup.
    // That receipt also reconciles an already ended frame stream: an active
    // writer's view is confirmed hidden, or Core has confirmed host shutdown.
    owner.native.close().await?;
    owner
        .state
        .lock()
        .map_err(|_| BrowserError::Closed)?
        .surface
        .take();
    Ok(())
}
pub(super) async fn hide_host(
    owner: &Owner,
    lease: colossus_browser_presentation::Lease,
) -> Result<(), BrowserError> {
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    let started = (
        std::time::Instant::now(),
        owner.revision.load(Ordering::Acquire),
    );
    match owner.client.hide(lease).await {
        Ok(())
        | Err(
            colossus_browser_presentation::PresentationError::Stale
            | colossus_browser_presentation::PresentationError::Hidden,
        ) => Ok(()),
        Err(error) => {
            #[cfg(all(
                any(windows, target_os = "macos"),
                feature = "native-test-driver",
                debug_assertions
            ))]
            super::trace_owner_failure(
                owner,
                "hide_host",
                super::presentation_failure_token(error),
                lease,
                started.1,
                started.0.elapsed(),
            );
            #[cfg(not(all(
                any(windows, target_os = "macos"),
                feature = "native-test-driver",
                debug_assertions
            )))]
            let _ = error;
            Err(BrowserError::Closed)
        }
    }
}
pub(super) async fn hide_surface(
    surface: Option<&Arc<PresentationSurface>>,
) -> Result<(), BrowserError> {
    if let Some(surface) = surface {
        surface.hide_confirmed().await?;
    }
    Ok(())
}
