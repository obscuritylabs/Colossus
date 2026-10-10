use super::*;
use std::{future::Future, pin::Pin, time::Duration};

type Operation<T> = Pin<Box<dyn Future<Output = Result<T, BrowserDriverError>> + Send>>;

impl BrowserHostPool {
    pub(super) async fn private_operation<T: Send + 'static>(
        &self,
        entry: Arc<Entry>,
        control: &BrowserDriverControl,
        action: impl FnOnce(Arc<dyn BrowserDriver>, BrowserDriverControl) -> Operation<T>
        + Send
        + 'static,
    ) -> Result<T, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let permit = Arc::clone(&self.inner.actions)
            .try_acquire_owned()
            .map_err(|_| BrowserDriverError::LimitExceeded)?;
        let (sender, receiver) = oneshot::channel();
        let retained = Arc::clone(&entry);
        let deadline_ms = self.inner.capabilities.limits.action_timeout_ms;
        tokio::spawn(async move {
            let _permit = permit;
            let _operation = retained.operation.lock().await;
            let driver = retained
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)
                .and_then(|state| {
                    if state.terminal || retained.admission_fenced.load(Ordering::Acquire) {
                        Err(BrowserDriverError::Stale)
                    } else {
                        state.driver.clone().ok_or(BrowserDriverError::Stale)
                    }
                });
            let result = match driver {
                Ok(driver) => tokio::time::timeout(
                    Duration::from_millis(u64::from(deadline_ms)),
                    action(
                        driver,
                        BrowserDriverControl::new(
                            RunControl::default(),
                            retained.authority.clone(),
                        ),
                    ),
                )
                .await
                .unwrap_or(Err(BrowserDriverError::OutcomeUnknown)),
                Err(error) => Err(error),
            };
            let _ = sender.send(result);
        });
        wait::response(
            Arc::clone(&self.inner),
            entry,
            receiver,
            control,
            deadline_ms,
        )
        .await
    }

    pub(super) async fn capture_private(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        let entry = self
            .entry(&command.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::command_for_open(&command, &entry.request, &self.inner.capabilities)?;
        if !matches!(
            command.action,
            colossus_contracts::BrowserAction::Screenshot { .. }
        ) {
            return Err(BrowserDriverError::Unsupported);
        }
        let retained = entry.clone();
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                super::transfer::retire(&retained)?;
                let descriptor = driver.capture(command.clone(), &control).await?;
                crate::validation::screenshot_descriptor(&descriptor, &command)?;
                Ok(descriptor)
            })
        })
        .await
    }

    pub(super) async fn read_screenshot_private(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        let entry = self
            .entry(&request.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::screenshot_read(&request, &entry.request)?;
        let offset = request.offset;
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                let chunk = driver.read_screenshot_chunk(request, &control).await?;
                crate::validation::screenshot_chunk(
                    &chunk,
                    offset,
                    colossus_ports::MAX_BROWSER_SCREENSHOT_BYTES,
                )?;
                Ok(chunk)
            })
        })
        .await
    }

    pub(super) async fn confirm_handoff_private(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        let entry = self
            .entry(&request.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::native_handoff(&request, &entry.request)?;
        let origins = entry.request.options.allowed_origins.clone();
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                let tab = driver
                    .confirm_native_handoff(request.clone(), &control)
                    .await?;
                if tab.tab_id != request.confirmed_target.tab_id
                    || tab.document_id != request.confirmed_target.document_id
                    || tab.title.len() > 1024
                    || tab
                        .origin
                        .as_ref()
                        .is_some_and(|origin| !origins.contains(origin))
                {
                    return Err(BrowserDriverError::OutcomeUnknown);
                }
                Ok(tab)
            })
        })
        .await
    }
}
