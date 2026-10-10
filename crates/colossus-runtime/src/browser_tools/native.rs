//! Sealed application-native human admission. No wire DTO carries authority here.
mod allocation;
pub(super) mod driver;
mod handoff;
pub(super) mod lifecycle;
mod types;

pub(super) use types::NativeState;
pub use types::{
    NativeBrowserOpenRequest, RuntimeBrowserPresenter, RuntimeNativeBrowserAuthority,
    RuntimeNativeBrowserError, RuntimeNativeBrowserGranted, RuntimeNativeBrowserSession,
};

#[cfg(test)]
mod tests;
