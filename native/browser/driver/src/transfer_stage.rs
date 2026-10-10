//! Native-only, retained file custody behind the contained browser's private profile.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "transfer_stage_unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "transfer_stage_windows.rs"]
mod platform;

#[cfg(colossus_cef_linked)]
pub use platform::Stage;
