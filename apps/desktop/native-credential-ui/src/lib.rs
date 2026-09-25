//! Native, main-thread credential entry. Plaintext never crosses renderer IPC.
//!
//! This private UI adapter isolates platform FFI from the unsafe-free Desktop host.
//! Platform controls may retain internal allocations; only owned Rust buffers and
//! the displayed control contents can be explicitly cleared here.

#[cfg(any(windows, target_os = "macos"))]
mod lifecycle;
mod prompt;
#[cfg(any(windows, target_os = "macos"))]
mod validation;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use prompt::{PromptError, prompt};

/// Run isolated, synthetic `AppKit` control acceptance on the process main thread.
/// This entry point exists only in explicitly enabled native test builds.
#[cfg(all(target_os = "macos", feature = "native-test-driver"))]
pub fn run_native_macos_acceptance() {
    macos::tests::run();
}

/// Exercise native clipboard and keyboard messages in an isolated window station.
/// This entry point exists only in explicitly enabled native test builds.
#[cfg(all(windows, feature = "native-test-driver"))]
pub fn run_native_windows_acceptance() {
    windows::acceptance::run();
}
