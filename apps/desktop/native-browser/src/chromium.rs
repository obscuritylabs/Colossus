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
pub use linked::{Surface, bind_quit_handler, bootstrap, pump, shutdown};

use crate::BrowserError;

/// Result of the real application-entry bootstrap, before any Tauri UI exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bootstrap {
    Ready,
    SubprocessExit(i32),
    Unavailable(BrowserError),
}

/// Closed native acceptance evidence for one actual Chromium child view.
///
/// Pixel counts sample a fixed PNG capture from the same CEF browser. `AppKit`
/// bounds and visibility are read from its native child, independently of CDP.
#[cfg(feature = "native-test-driver")]
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
// Independent observed native facts are serialized evidence, not state flags.
#[allow(clippy::struct_excessive_bools)]
pub struct AcceptanceProbe {
    pub visible: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub magenta_pixels: u32,
    pub green_pixels: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub bits_per_sample: u32,
    pub samples_per_pixel: u32,
    pub alpha_first: bool,
    pub magenta_sample: Vec<u32>,
    pub green_sample: Vec<u32>,
    pub child_width: f64,
    pub child_height: f64,
    pub child_class: String,
    pub parent_layer: bool,
    pub view_autoresizes_subviews: bool,
    pub child_autoresizing_mask: u32,
    pub cef_application: bool,
    pub tauri_event_loop: bool,
    pub parent_attached: bool,
    pub app_active: bool,
    pub activation_policy: i32,
    pub window_key: bool,
    pub window_visible: bool,
    pub window_can_become_key: bool,
    pub delegate_class: String,
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

    /// # Errors
    /// A linked, accepted native Chromium component is required.
    #[cfg(feature = "native-test-driver")]
    pub async fn acceptance_probe(&self) -> Result<AcceptanceProbe, BrowserError> {
        Err(BrowserError::ComponentMissing)
    }

    /// # Errors
    /// A linked native Chromium component and owning window are required.
    #[cfg(feature = "native-test-driver")]
    pub async fn acceptance_activate_parent(&self) -> Result<(), BrowserError> {
        Err(BrowserError::ComponentMissing)
    }

    /// # Errors
    /// A linked native Chromium component and owning window are required.
    #[cfg(feature = "native-test-driver")]
    pub async fn acceptance_terminate_application(&self) -> Result<(), BrowserError> {
        Err(BrowserError::ComponentMissing)
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
/// Bind the trusted native application's Quit route.
///
/// # Errors
/// Returns the missing-component error for an unlinked preview build.
pub fn bind_quit_handler(_: impl Fn() + Send + Sync + 'static) -> Result<(), BrowserError> {
    Err(BrowserError::ComponentMissing)
}

#[cfg(not(colossus_cef_linked))]
/// Report absent native teardown evidence for an unlinked preview build.
///
/// # Errors
/// Returns the missing-component error because no Chromium host initialized.
pub fn shutdown() -> Result<(), BrowserError> {
    Err(BrowserError::ComponentMissing)
}
