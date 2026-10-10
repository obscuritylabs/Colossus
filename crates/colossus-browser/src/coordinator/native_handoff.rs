use super::*;
use crate::state::Session;

/// Native-only receipt for transferring a fresh human page to one authenticated run.
///
/// Trusted composition constructs this only after the private native host has
/// acknowledged an irreversible human-input fence and observed that same page.
/// It has no wire or model serialization. A failed transfer leaves the host fenced;
/// the caller retains cleanup ownership and must close or explicitly reconcile it.
pub struct NativeBrowserHandoff {
    /// Coordinator generation captured before the native input fence.
    pub expected_control_generation: u64,
    /// Exact coordinator tab/document captured before the native input fence.
    pub expected_target: BrowserTarget,
    /// Actual document returned by the authenticated, fenced native host.
    pub confirmed_tab: BrowserTabSummary,
}

impl NativeBrowserHandoff {
    pub(super) fn validate(&self, session: &Session) -> Result<(), BrowserError> {
        if session.summary.control != BrowserControlState::Human
            || session.summary.control_generation != self.expected_control_generation
            || session.lease.is_some()
            || session.last_run.is_some()
        {
            return Err(BrowserError::StaleControl);
        }
        if session.summary.selected_tab_id.as_ref() != Some(&self.expected_target.tab_id)
            || self.confirmed_tab.tab_id != self.expected_target.tab_id
            || !session.summary.tabs.iter().any(|tab| {
                tab.tab_id == self.expected_target.tab_id
                    && tab.document_id == self.expected_target.document_id
            })
        {
            return Err(BrowserError::StaleDocument);
        }
        validation::tab(&self.confirmed_tab, &session.options)
    }
}

impl BrowserCoordinator {
    /// Adopt the fenced native document and grant its first writer atomically.
    ///
    /// This is a trusted composition entry point, following ordinary application/run
    /// authorization. The receipt must come directly from the retained authenticated
    /// native host; callers must never translate a renderer or application claim into
    /// a receipt. Stale bindings, target/generation changes, concurrent effects, foreign
    /// destinations and duplicate transfers fail before coordinator state changes.
    pub fn grant_control_from_native_handoff(
        &self,
        actor: &BrowserActor,
        id: &BrowserSessionId,
        lease_ms: u32,
        handoff: NativeBrowserHandoff,
    ) -> Result<BrowserControlLease, BrowserError> {
        self.grant_control_inner(actor, id, lease_ms, Some(&handoff))
    }
}
