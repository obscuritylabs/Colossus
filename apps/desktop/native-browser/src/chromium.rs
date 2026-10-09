//! Direct CEF guest presentation behind an explicit native feasibility gate.
//!
//! CEF application installation occurs before Tauri/AppKit. A missing component
//! never falls back to another engine under a Chromium label.

#[cfg(colossus_cef_linked)]
mod callbacks;
#[cfg(colossus_cef_linked)]
mod ffi;
#[cfg(colossus_cef_linked)]
mod linked;

#[cfg(colossus_cef_linked)]
pub use linked::{Surface, bootstrap, pump, shutdown};

use crate::BrowserError;

/// Result of the real application-entry bootstrap, before any Tauri UI exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bootstrap {
    Ready,
    SubprocessExit(i32),
    Unavailable(BrowserError),
}

/// Actual availability established by early bootstrap; never a compile-time guess.
///
/// # Errors
/// Reports missing bootstrap, component, or native platform acceptance.
pub fn readiness() -> Result<(), BrowserError> {
    #[cfg(colossus_cef_linked)]
    return linked::readiness();
    #[cfg(not(colossus_cef_linked))]
    Err(BrowserError::ComponentMissing)
}

#[cfg(not(colossus_cef_linked))]
/// Missing-component implementation keeps optional preview builds inspectable.
pub struct Surface;

#[cfg(not(colossus_cef_linked))]
impl Clone for Surface {
    fn clone(&self) -> Self {
        Self
    }
}

#[cfg(not(colossus_cef_linked))]
#[allow(clippy::unused_async)] // Preserve the same asynchronous linked lifecycle API.
impl Surface {
    /// Create the same native guest later controlled by the runtime bridge.
    ///
    /// # Errors
    /// The unlinked build reports the missing verified component explicitly.
    pub async fn create(
        _: &tauri::Window,
        _: &str,
        _: u64,
        _: std::path::PathBuf,
        _: crate::NavigationPolicy,
        _: crate::EventSink,
    ) -> Result<Self, BrowserError> {
        Err(BrowserError::ComponentMissing)
    }

    /// # Errors
    /// Chromium is absent in this build.
    pub fn navigate(&self, _: &url::Url) -> Result<(), BrowserError> {
        readiness()
    }
    /// # Errors
    /// Chromium is absent in this build.
    pub fn hide(&self) -> Result<(), BrowserError> {
        readiness()
    }
    /// # Errors
    /// Chromium is absent in this build.
    pub fn show(&self) -> Result<(), BrowserError> {
        readiness()
    }
    /// # Errors
    /// Chromium is absent in this build.
    pub fn set_bounds(&self, _: tauri::Rect) -> Result<(), BrowserError> {
        readiness()
    }
    /// # Errors
    /// Chromium is absent in this build.
    pub async fn close(&self) -> Result<(), BrowserError> {
        readiness()
    }
    /// # Errors
    /// Chromium is absent in this build.
    pub async fn inspect(&self) -> Result<crate::PageState, BrowserError> {
        Err(BrowserError::ComponentMissing)
    }
    /// # Errors
    /// Chromium is absent in this build.
    pub async fn control(&self, _: crate::NavigationAction) -> Result<(), BrowserError> {
        readiness()
    }
}

#[cfg(not(colossus_cef_linked))]
/// Initialize before the native application is created.
#[must_use]
pub fn bootstrap(_: &std::path::Path, _: &std::path::Path) -> Bootstrap {
    Bootstrap::Unavailable(BrowserError::ComponentMissing)
}

#[cfg(not(colossus_cef_linked))]
/// Pump callback is inert when no component is linked.
pub fn pump() {}

#[cfg(not(colossus_cef_linked))]
/// No native component needs teardown in an unlinked build.
pub fn shutdown() {}
