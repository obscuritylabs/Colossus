use super::*;

/// Native-only postgrant state; the same GUI stream becomes a read-only viewer.
pub struct NativeBrowserHandoff {
    pub(super) lease: BrowserControlLease,
    pub(super) configure: Configure,
    pub(super) enrollment_digest: [u8; 32],
}
impl NativeBrowserHandoff {
    /// Runtime-held run writer after gateway authorization and the irreversible native fence.
    pub fn lease(&self) -> &BrowserControlLease {
        &self.lease
    }
    /// Next fresh GUI Configure after Runtime acknowledged a read-only native viewport.
    pub fn configure(&self) -> Configure {
        self.configure.clone()
    }
    /// Unchanged authenticated host enrollment for the retained native relay.
    pub fn enrollment_digest(&self) -> [u8; 32] {
        self.enrollment_digest
    }
}

/// Native-only attachment to the exact admitted page and its retained cleanup owner.
pub struct NativeBrowserAttachment {
    pub(super) conversation_id: String,
    pub(super) presentation: PresentationClient,
    pub(super) configure: Configure,
    pub(super) enrollment_digest: [u8; 32],
    pub(super) owner: Arc<NativeBrowserSessionOwner>,
}
impl NativeBrowserAttachment {
    /// Exact canonical conversation returned by the managed Runtime.
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    /// Retained private presentation client; never place it in renderer state.
    pub fn presentation(&self) -> PresentationClient {
        self.presentation.clone()
    }
    /// Runtime-derived opaque placement and ownership.
    pub fn configure(&self) -> Configure {
        self.configure.clone()
    }
    /// Exact host enrollment for authenticated native pixels.
    pub fn enrollment_digest(&self) -> [u8; 32] {
        self.enrollment_digest
    }
    /// Retained full-context cleanup owner for native tab/window teardown.
    pub fn owner(&self) -> Arc<NativeBrowserSessionOwner> {
        Arc::clone(&self.owner)
    }
}
