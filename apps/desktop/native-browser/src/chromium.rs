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

#[cfg(all(windows, colossus_cef_linked))]
pub use linked::run_windows_client;
#[cfg(colossus_cef_linked)]
pub use linked::{Surface, bind_quit_handler, bootstrap, pump, shutdown};

/// A Windows preview without native linkage cannot accept a bootstrap entry.
///
/// # Safety
/// Matches the linked entry's borrowed bootstrap ABI; no pointer is inspected.
#[cfg(all(windows, not(colossus_cef_linked)))]
pub unsafe fn run_windows_client(
    _: usize,
    _: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
    _: unsafe extern "C" fn() -> i32,
) -> i32 {
    1
}

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
/// macOS pixel counts sample a fixed PNG from the CEF browser; Windows counts
/// sample the actual screen compositor at the guest HWND. Native child bounds
/// and visibility are read independently of CDP. Platform-specific fields carry
/// no evidence on other operating systems.
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
    #[serde(default)]
    pub windows_owning_thread: bool,
    #[serde(default)]
    pub os_compositor_capture: bool,
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
    /// # Errors
    /// Chromium is absent in this build.
    pub fn pending_client_identity(
        &self,
    ) -> Result<Option<crate::pki::IdentityRequest>, BrowserError> {
        Err(BrowserError::ComponentMissing)
    }

    /// An unlinked component owns no pending native identity request.
    pub fn revoke_client_identity(&self) {}

    /// An unlinked component owns no native review to dismiss.
    pub fn cancel_client_identity(&self, _: &crate::pki::IdentityRequest) {}

    /// # Errors
    /// Chromium is absent in this build; no identity can be selected.
    pub async fn select_client_identity(
        &self,
        _: &crate::pki::IdentityRequest,
        _: Option<&str>,
    ) -> Result<(), BrowserError> {
        Err(BrowserError::ComponentMissing)
    }

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

    /// # Errors
    /// No linked Windows native input driver is available.
    #[cfg(all(windows, feature = "native-test-driver"))]
    pub async fn acceptance_input(&self) -> Result<(), BrowserError> {
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
