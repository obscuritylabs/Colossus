use super::*;

pub(super) fn start(
    inner: Arc<Inner>,
    entry: Arc<Entry>,
) -> Result<watch::Receiver<CleanupResult>, BrowserDriverError> {
    entry.authority.cancel();
    let mut state = entry
        .state
        .lock()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    state.terminal = true;
    if let Some(receiver) = &state.cleanup
        && (receiver.borrow().is_none() || state.cleaned)
    {
        return Ok(receiver.clone());
    }
    tokio::runtime::Handle::try_current().map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let (sender, receiver) = watch::channel(None);
    state.cleanup = Some(receiver.clone());
    drop(state);
    // One owned cleanup worker per entry; callers never own its cancellation.
    tokio::spawn(async move {
        let deadline = std::time::Duration::from_millis(
            u64::from(inner.capabilities.limits.navigation_timeout_ms) * 3,
        );
        let result = tokio::time::timeout(deadline, reap(&inner, &entry))
            .await
            .unwrap_or(Err(BrowserDriverError::OutcomeUnknown));
        if result.is_ok()
            && let Ok(mut state) = entry.state.lock()
        {
            state.cleaned = true;
        }
        let _ = sender.send(Some(result));
    });
    Ok(receiver)
}

pub(super) async fn acknowledged(
    mut receiver: watch::Receiver<CleanupResult>,
) -> Result<(), BrowserDriverError> {
    loop {
        if let Some(result) = *receiver.borrow_and_update() {
            return result;
        }
        receiver
            .changed()
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    }
}

async fn reap(inner: &Inner, entry: &Entry) -> Result<(), BrowserDriverError> {
    let pending_driver = entry
        .state
        .lock()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        .driver
        .clone();
    // Quiescence must reach native dispatch independently of the action lock.
    if let Some(driver) = &pending_driver {
        let _ = driver.cancel_session(&entry.request.session_id).await;
    }
    let _operation = entry.operation.lock().await;
    let (driver, launch) = {
        let state = entry
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        (state.driver.clone(), state.launch)
    };
    match (driver, launch) {
        (Some(driver), _) => {
            if pending_driver.is_none() {
                let _ = driver.cancel_session(&entry.request.session_id).await;
            }
            // Full teardown can prove cleanup even when preliminary cancellation failed.
            driver.close_session(&entry.request.session_id).await
        }
        (None, LaunchState::Absent) => Ok(()),
        (None, _) => inner.factory.reap_failed_launch(&entry.request).await,
    }
}
