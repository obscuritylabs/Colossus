use super::types::*;
use crate::Runtime;
use colossus_browser::BrowserError;
use colossus_contracts::BrowserSessionId;
use std::sync::Arc;

pub(super) struct WorkerGuard {
    browser: Arc<super::super::RuntimeBrowserTools>,
    entry: Arc<Entry>,
    armed: bool,
}
impl WorkerGuard {
    pub(super) fn new(browser: Arc<super::super::RuntimeBrowserTools>, entry: Arc<Entry>) -> Self {
        Self {
            browser,
            entry,
            armed: true,
        }
    }
    pub(super) fn finish(&mut self) {
        self.armed = false;
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.armed {
            fence_entry(&self.entry);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let browser = self.browser.clone();
                let entry = self.entry.clone();
                handle.spawn(async move {
                    let _ = close_entry(&browser, &entry).await;
                });
            }
        }
    }
}

pub(in crate::browser_tools) fn entry(
    browser: &super::super::RuntimeBrowserTools,
    role: &RuntimeNativeBrowserAuthority,
    session: &BrowserSessionId,
) -> Result<Arc<Entry>, RuntimeNativeBrowserError> {
    authority(browser, role)?;
    let entries = browser
        .native
        .entries
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
    entries
        .values()
        .find(|entry| {
            entry.instance == role.instance
                && entry.binding.application_id == role.application_id
                && entry.request.lock().is_ok_and(|request| {
                    request
                        .as_ref()
                        .is_some_and(|request| &request.session_id == session)
                })
        })
        .cloned()
        .ok_or(RuntimeNativeBrowserError::Denied)
}

pub(in crate::browser_tools) fn fence_entry(entry: &Entry) {
    entry.control.cancel();
    let mut view = entry.view.lock().unwrap_or_else(|error| error.into_inner());
    view.fenced = true;
    view.closed = true;
    if let Some(client) = &view.client {
        client.disconnect();
    }
}

pub(in crate::browser_tools) async fn close_entry(
    browser: &super::super::RuntimeBrowserTools,
    entry: &Arc<Entry>,
) -> Result<(), RuntimeNativeBrowserError> {
    fence_entry(entry);
    let _operation = entry.operation.lock().await;
    let original = entry
        .request
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .clone();
    {
        let mut view = entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        view.fenced = true;
        view.closed = true;
        if let Some(client) = &view.client {
            client.disconnect();
        }
    }
    if let Some(original) = original {
        match browser
            .coordinator
            .close(&entry.binding, &original.session_id)
            .await
        {
            Ok(()) => {}
            Err(BrowserError::NotFound) => {
                browser
                    .native
                    .driver
                    .close_session(&original.session_id)
                    .await?
            }
            Err(error) => return Err(error.into()),
        }
    }
    entry
        .view
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .retired = true;
    browser
        .native
        .entries
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .remove(&entry.id);
    Ok(())
}

pub(in crate::browser_tools) async fn drain(browser: &super::super::RuntimeBrowserTools) {
    let entries = browser
        .native
        .entries
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for entry in entries {
        let _ = close_entry(browser, &entry).await;
    }
}

impl Runtime {
    /// Explicit native user close reaps the entire owned browser context.
    /// The cleanup task survives loss of its caller and retains uncertain ownership.
    pub async fn close_native_browser(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        session: &BrowserSessionId,
    ) -> Result<(), RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?
            .clone();
        let owned = entry(&browser, role, session)?;
        fence_entry(&owned);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = close_entry(&browser, &owned).await;
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
    }

    /// Native viewer loss closes a human context. After transfer it hides only the
    /// viewer lease, retaining the private host channel and active agent context.
    pub async fn detach_native_browser(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        session: &BrowserSessionId,
    ) -> Result<(), RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?
            .clone();
        let owned = entry(&browser, role, session)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let operation = owned.operation.lock().await;
            let state = owned
                .view
                .lock()
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)
                .map(|view| {
                    (
                        view.agent_generation,
                        view.client.clone(),
                        view.lease,
                        view.closed,
                    )
                });
            let result = match state {
                Ok((Some(_), Some(client), Some(lease), false)) => {
                    client.hide(lease).await.map_err(Into::into)
                }
                Ok((Some(_), _, _, false)) => Err(RuntimeNativeBrowserError::Stale),
                Ok(_) => {
                    drop(operation);
                    close_entry(&browser, &owned).await
                }
                Err(error) => Err(error),
            };
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
    }
}
