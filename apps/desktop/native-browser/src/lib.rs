//! Native browser-engine adapter for human-operated Desktop guest views.
//!
//! This crate exposes navigation and observation, never arbitrary JavaScript or
//! host objects. Future agent control must enter through a separately authorized
//! runtime adapter; possession of a guest handle is not an agent permit.

mod engine;
mod navigation;
mod types;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use engine::{control, harden, inspect, is_active, open_external, release, share_session};
pub use navigation::{NavigationPolicy, parse_address};
pub use types::{BrowserError, BrowserEvent, EventSink, NavigationAction, PageState};
