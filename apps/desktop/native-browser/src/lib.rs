//! Native browser-engine adapter for human-operated Desktop guest views.
//!
//! Production builds expose navigation and observation, never arbitrary JavaScript
//! or host objects. Future agent control must enter through a separately authorized
//! runtime adapter; possession of a guest handle is not an agent permit.

mod engine;
mod navigation;
#[cfg(any(target_os = "macos", test))]
mod response;
mod types;

#[cfg(all(target_os = "macos", feature = "native-test-driver"))]
pub mod acceptance;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use engine::{control, harden, inspect, is_active, open_external, release, share_session};
pub use navigation::{NavigationPolicy, parse_address};
pub use types::{BrowserError, BrowserEvent, EventSink, NavigationAction, PageState};

#[cfg(all(windows, feature = "native-test-driver"))]
pub use windows::probe_file_picker;
