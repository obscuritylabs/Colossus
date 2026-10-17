use super::*;
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

pub(super) async fn open(
    inner: Arc<Inner>,
    entry: Arc<Entry>,
    sender: oneshot::Sender<Result<BrowserTabSummary, BrowserDriverError>>,
) {
    let operation = entry.operation.lock().await;
    let control = BrowserDriverControl::new(RunControl::default(), entry.authority.clone());
    let result = allocate(&inner, &entry, &control).await;
    drop(operation);
    let result = if result.is_err() || entry.authority.is_cancelled() {
        match cleanup::start(Arc::clone(&inner), Arc::clone(&entry)) {
            Ok(receiver) => match cleanup::acknowledged(receiver).await {
                Ok(()) => {
                    if result
                        .as_ref()
                        .is_err_and(|error| *error != BrowserDriverError::OutcomeUnknown)
                        && !entry.admission_fenced.load(Ordering::Acquire)
                        && let Ok(mut entries) = inner.entries.lock()
                        && entries
                            .get(&entry.request.session_id)
                            .is_some_and(|current| Arc::ptr_eq(current, &entry))
                    {
                        entries.remove(&entry.request.session_id);
                    }
                    result.and(Err(BrowserDriverError::OutcomeUnknown))
                }
                Err(_) => Err(BrowserDriverError::OutcomeUnknown),
            },
            Err(_) => Err(BrowserDriverError::OutcomeUnknown),
        }
    } else {
        result
    };
    let _ = sender.send(result);
}

async fn allocate(
    inner: &Inner,
    entry: &Entry,
    control: &BrowserDriverControl,
) -> Result<BrowserTabSummary, BrowserDriverError> {
    if control.is_cancelled() {
        entry
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .launch = LaunchState::Absent;
        return Err(BrowserDriverError::Cancelled);
    }
    let deadline =
        Duration::from_millis(u64::from(inner.capabilities.limits.navigation_timeout_ms));
    let result = timeout(deadline, inner.factory.launch(&entry.request, control))
        .await
        .unwrap_or(Err(BrowserDriverError::OutcomeUnknown));
    let driver = {
        let mut state = entry
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        match result {
            Ok(driver) => {
                state.launch = LaunchState::Ready;
                state.driver = Some(Arc::clone(&driver));
                driver
            }
            Err(error) => {
                state.launch = if error == BrowserDriverError::OutcomeUnknown {
                    LaunchState::Unknown
                } else {
                    LaunchState::Absent
                };
                return Err(error);
            }
        }
    };
    if control.is_cancelled() {
        return Err(BrowserDriverError::Cancelled);
    }
    let tab = timeout(
        deadline,
        driver.open_session(entry.request.clone(), control),
    )
    .await
    .unwrap_or(Err(BrowserDriverError::OutcomeUnknown))?;
    if tab.tab_id != entry.request.tab_id
        || tab.document_id != entry.request.document_id
        || tab.title.len() > 1024
        || tab
            .origin
            .as_ref()
            .is_some_and(|origin| !entry.request.options.allowed_origins.contains(origin))
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(tab)
}

pub(super) async fn execute(
    inner: Arc<Inner>,
    entry: Arc<Entry>,
    command: BrowserDriverCommand,
    _permit: OwnedSemaphorePermit,
    sender: oneshot::Sender<Result<BrowserObservation, BrowserDriverError>>,
) {
    let _operation = entry.operation.lock().await;
    let retired = super::transfer::retire(&entry);
    let driver = entry
        .state
        .lock()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)
        .and_then(|state| {
            if state.terminal {
                Err(BrowserDriverError::Stale)
            } else {
                state.driver.clone().ok_or(BrowserDriverError::Stale)
            }
        });
    let result = match retired.and(driver) {
        Ok(driver) => timeout(
            Duration::from_millis(u64::from(inner.capabilities.limits.action_timeout_ms)),
            driver.execute(
                command,
                &BrowserDriverControl::new(RunControl::default(), entry.authority.clone()),
            ),
        )
        .await
        .unwrap_or(Err(BrowserDriverError::OutcomeUnknown)),
        Err(error) => Err(error),
    };
    let _ = sender.send(result);
}
