//! Native browser-engine adapter for human-operated Desktop guest views.
//!
//! Production builds expose navigation and observation, never arbitrary JavaScript
//! or host objects. Future agent control must enter through a separately authorized
//! runtime adapter; possession of a guest handle is not an agent permit.

#[cfg(all(
    feature = "windows-browser-functional-preview",
    any(
        not(windows),
        not(target_arch = "x86_64"),
        feature = "cef-preview",
        feature = "native-test-driver"
    )
))]
compile_error!("functional browser presentation supports Windows x64 without CEF or test bridges");

#[cfg(feature = "cef-preview")]
pub mod chromium;
#[cfg(any(
    feature = "cef-preview",
    feature = "windows-browser-functional-preview"
))]
pub mod contained;
mod engine;
mod navigation;
#[cfg(feature = "cef-preview")]
pub mod pki;
#[cfg(all(colossus_browser_presenter_linked, any(windows, target_os = "macos")))]
pub mod presentation;
#[cfg(all(colossus_browser_presenter_linked, any(windows, target_os = "macos")))]
pub mod remote;
#[cfg(any(target_os = "macos", test))]
mod response;
mod types;
mod view;

#[cfg(all(target_os = "macos", feature = "native-test-driver"))]
pub mod acceptance;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use engine::{clear_site_data, harden_human_profile};
pub use engine::{control, harden, inspect, is_active, open_external, release, share_session};
pub use navigation::{NavigationPolicy, parse_address};
pub use types::{BrowserError, BrowserEvent, EventSink, NavigationAction, PageState};
pub use view::BrowserView;

#[cfg(all(windows, feature = "native-test-driver"))]
pub use windows::probe_file_picker;
