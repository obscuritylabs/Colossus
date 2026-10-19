//! Closed Windows source-fixture probes for the owned remote presenter.
use super::RemoteSurface;
use crate::{BrowserError, presentation::WindowsScreenEvidence};

impl RemoteSurface {
    /// Capture OS compositor evidence for the source-only synthetic acceptance page.
    ///
    /// # Errors
    /// Requires a live native presenter, its owning UI thread and foreground parent.
    pub async fn acceptance_windows_screen(&self) -> Result<WindowsScreenEvidence, BrowserError> {
        let surface = self
            .0
            .state
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .surface
            .clone()
            .ok_or(BrowserError::Unavailable)?;
        surface.screen().await
    }

    /// Dispatch the closed native key/Unicode sequence to the synthetic field.
    ///
    /// # Errors
    /// Human control must remain active; the ordinary input guard rechecks every event.
    pub async fn acceptance_windows_input(&self) -> Result<(), BrowserError> {
        if !self.human_control_available() {
            return Err(BrowserError::Unavailable);
        }
        let surface = self
            .0
            .state
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .surface
            .clone()
            .ok_or(BrowserError::Unavailable)?;
        surface.fixture_input().await
    }

    /// Exercise reentrant Return messages in this presenter's own Tao parent.
    ///
    /// # Errors
    /// Requires human control, the exact foreground parent, an observed nested
    /// callback and a subsequent main-thread acknowledgement. The external native
    /// acceptance runner must bound its disposable process if Tao self-deadlocks.
    pub async fn acceptance_windows_keyboard_reentrancy(&self) -> Result<(), BrowserError> {
        if !self.human_control_available() {
            return Err(BrowserError::Unavailable);
        }
        let surface = self
            .0
            .state
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .surface
            .clone()
            .ok_or(BrowserError::Unavailable)?;
        surface.fixture_keyboard_reentrancy().await
    }
}
