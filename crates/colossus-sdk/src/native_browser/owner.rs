use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Native viewer owner; explicit close/detach is acknowledged and dropped owners retain retry.
pub struct NativeBrowserSessionOwner {
    client: NativeBrowserClient,
    session: BrowserSessionId,
    closed: AtomicBool,
    detached: AtomicBool,
    cleanup: tokio::sync::Mutex<()>,
}
impl NativeBrowserSessionOwner {
    pub(super) fn new(client: NativeBrowserClient, session: BrowserSessionId) -> Self {
        Self {
            client,
            session,
            closed: AtomicBool::new(false),
            detached: AtomicBool::new(false),
            cleanup: tokio::sync::Mutex::new(()),
        }
    }
    /// Exact native session whose positively admitted cleanup belongs to this owner.
    pub fn session_id(&self) -> &BrowserSessionId {
        &self.session
    }
    /// Await full host/profile cleanup. An uncertain result never marks the owner complete.
    pub async fn close(&self) -> SdkResult<()> {
        let _cleanup = self.cleanup.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.client.close_session(&self.session).await?;
        self.closed.store(true, Ordering::Release);
        self.detached.store(true, Ordering::Release);
        Ok(())
    }
    /// Release this viewer. Human contexts are reaped; active agent pages stay run-owned.
    /// Native window teardown uses this operation rather than canceling another writer.
    pub async fn detach(&self) -> SdkResult<()> {
        let _cleanup = self.cleanup.lock().await;
        if self.detached.load(Ordering::Acquire) {
            return Ok(());
        }
        self.client.detach_session(&self.session).await?;
        self.detached.store(true, Ordering::Release);
        Ok(())
    }
}
impl Drop for NativeBrowserSessionOwner {
    fn drop(&mut self) {
        if self.detached.load(Ordering::Acquire) {
            return;
        }
        let client = self.client.clone();
        let session = self.session.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                // Viewer detach is idempotent and preserves an active run writer.
                // Retain exact ownership until the worker positively acknowledges it.
                while client.detach_session(&session).await.is_err() {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });
        }
    }
}
