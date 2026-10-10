//! Trusted composition seam for Desktop's independently supervised human guests.
use crate::{BrowserError, BrowserView, EventSink, NavigationPolicy};
use async_trait::async_trait;
/// Closed native placement type shared with the managed sidecar adapter.
pub use colossus_browser_presentation::Configure as NativeConfigure;
use std::sync::Arc;
use tauri::{AppHandle, Manager as _, Window};
use url::Url;

/// Native attachment shape. Selecting a role does not grant runtime ownership;
/// composition must separately admit the exact session and private channel.
#[derive(Clone, Copy)]
pub enum PresentationRole {
    /// Fresh dedicated human session, before agent ownership begins.
    Human,
    /// View the exact current agent page without human input or navigation.
    ReadOnly,
}
impl PresentationRole {
    /// Check the native controller generation expected by this admitted role.
    #[must_use]
    pub const fn accepts_generation(self, generation: u64) -> bool {
        match self {
            Self::Human => generation == 0,
            Self::ReadOnly => generation > 0,
        }
    }
}

/// A native Desktop request after controller/workspace generation checks.
/// These inputs do not nominate an executable, profile, endpoint or grant.
pub struct ContainedGuestRequest {
    /// Existing Desktop display identity; the factory creates private runtime identities.
    pub id: String,
    /// Trusted current controller generation.
    pub generation: u64,
    /// Native-selected workspace scope, checked again by composition.
    pub scope: String,
    /// Nominated canonical conversation, whose ownership is checked by Runtime.
    pub conversation_id: Option<String>,
    /// Requested human address; composition derives the immutable origin envelope.
    pub initial_url: Option<Url>,
    /// Closed Desktop navigation checks, additional to native containment policy.
    pub policy: NavigationPolicy,
    /// Trusted generation-bound Desktop event sink.
    pub sink: EventSink,
}
/// Native bootstrap supplies the verified installed host and complete cleanup owner.
/// Factories obtain runtime-owned human admission before launching any browser process.
#[async_trait]
pub trait ContainedGuestFactory: Send + Sync {
    /// Query accepted embedded placement before any owned allocation is attempted.
    async fn available(
        &self,
        window: &Window,
        request: &ContainedGuestRequest,
    ) -> Result<bool, BrowserError>;
    /// Create the actual independently contained native pixel guest.
    ///
    /// # Errors
    /// Missing platform proof or uncertain startup fails closed and retains cleanup.
    async fn create(
        &self,
        window: &Window,
        request: ContainedGuestRequest,
    ) -> Result<BrowserView, BrowserError>;
}

/// Retained native context owner. No renderer can create or serialize this authority.
#[async_trait]
pub trait RemoteHostOwner: Send + Sync {
    /// Canonical conversation admitted by the owning runtime.
    fn conversation_id(&self) -> &str;
    /// Exact browser context admitted by the owning runtime.
    fn session_id(&self) -> &str;
    /// Detach the view; human contexts additionally require complete owned cleanup.
    async fn close(&self) -> Result<(), BrowserError>;
    /// Transfer to a registered run and return only server-derived native placement.
    async fn handoff(
        &self,
        _run_id: &str,
    ) -> Result<colossus_browser_presentation::Configure, BrowserError> {
        Err(BrowserError::Unavailable)
    }
}

/// Whether this build includes the native pixel compositor, independent of host acceptance.
#[must_use]
pub const fn native_presentation_available() -> bool {
    cfg!(all(colossus_cef_linked, any(windows, target_os = "macos")))
}

/// Attach independently admitted pixels through the platform compositor.
///
/// # Errors
/// Missing compositor or failed private/native admission fails closed.
pub async fn attach(
    window: &Window,
    client: colossus_browser_presentation::PresentationClient,
    configure: colossus_browser_presentation::Configure,
    digest: [u8; 32],
    owner: Arc<dyn RemoteHostOwner>,
    sink: EventSink,
) -> Result<BrowserView, BrowserError> {
    #[cfg(all(colossus_cef_linked, any(windows, target_os = "macos")))]
    {
        crate::remote::RemoteSurface::create(window, client, configure, digest, owner, sink)
            .await
            .map(BrowserView::Remote)
    }
    #[cfg(not(all(colossus_cef_linked, any(windows, target_os = "macos"))))]
    {
        let _ = (window, client, configure, digest, sink);
        owner.close().await?;
        Err(BrowserError::Unavailable)
    }
}
/// Native-only application state. No web IPC command installs or replaces this factory.
pub struct ContainedBrowserComposition(pub Arc<dyn ContainedGuestFactory>);
impl ContainedBrowserComposition {
    /// Install once from trusted native bootstrap before browser requests are admitted.
    ///
    /// # Errors
    /// Refuses replacement of an existing native owner.
    pub fn install(
        app: &AppHandle,
        factory: Arc<dyn ContainedGuestFactory>,
    ) -> Result<(), BrowserError> {
        app.manage(Self(factory))
            .then_some(())
            .ok_or(BrowserError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::PresentationRole;

    #[test]
    fn human_attachment_cannot_claim_an_agent_generation() {
        assert!(PresentationRole::Human.accepts_generation(0));
        assert!(!PresentationRole::Human.accepts_generation(1));
        assert!(!PresentationRole::Human.accepts_generation(u64::MAX));
    }

    #[test]
    fn read_only_viewer_cannot_claim_initial_human_ownership() {
        assert!(!PresentationRole::ReadOnly.accepts_generation(0));
        assert!(PresentationRole::ReadOnly.accepts_generation(1));
        assert!(PresentationRole::ReadOnly.accepts_generation(u64::MAX));
    }
}
