use crate::Runtime;

impl Runtime {
    /// Credential-free installed browser capability evidence. Missing native components
    /// never fall back to a personal browser or an unverified engine.
    pub fn browser_capabilities(&self) -> colossus_contracts::BrowserCapabilities {
        self.browser.as_ref().map_or_else(
            colossus_contracts::BrowserCapabilities::unavailable,
            |browser| browser.coordinator.capabilities(),
        )
    }

    /// Fail-closed active-work fact for trusted worker eviction and configuration drain.
    /// Allocation and uncertain cleanup pin the runtime; no background browser grant
    /// survives an ended run in this initial ephemeral-session implementation.
    pub fn browser_active_work(&self) -> bool {
        let Some(browser) = &self.browser else {
            return false;
        };
        let Ok(runs) = browser.runs.lock() else {
            return true;
        };
        runs.values().any(|run| {
            browser
                .coordinator
                .list(&run.actor.binding)
                .map_or(true, |sessions| !sessions.is_empty())
        })
    }

    /// Revoke run-owned writers synchronously during trusted shutdown or drain.
    pub fn stop_browser_sessions(&self) {
        if let Some(browser) = &self.browser {
            browser.cancel_all();
        }
    }

    /// Quiesce and close all run-owned ephemeral browser contexts during shutdown.
    pub async fn drain_browser_sessions(&self) {
        if let Some(browser) = &self.browser {
            browser.drain().await;
        }
    }
}
