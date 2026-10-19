//! Sealed application-native human admission. No wire DTO carries authority here.
mod allocation;
pub(super) mod driver;
mod expiry;
mod handoff;
pub(super) mod lifecycle;
pub(super) mod selection;
mod types;
mod watch;
#[cfg(windows)]
mod windows_watch;

pub(super) use types::NativeState;
pub use types::{
    NativeBrowserOpenRequest, RuntimeBrowserPresenter, RuntimeNativeBrowserAuthority,
    RuntimeNativeBrowserError, RuntimeNativeBrowserGranted, RuntimeNativeBrowserSession,
};

#[cfg(test)]
mod tests;
