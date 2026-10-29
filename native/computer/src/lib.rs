//! Private, window-bound Windows computer-use adapter.
//!
//! Only trusted CLI/sidecar composition installs this driver. Public model tools
//! never nominate an executable, native handle, process identity or helper role.

#[cfg(windows)]
mod driver;
#[cfg(not(windows))]
#[path = "driver_unavailable.rs"]
mod driver;
#[cfg(any(windows, test))]
mod focus_settle;
#[cfg(any(windows, test))]
mod geometry;
mod human_appshot;
#[cfg(any(windows, test))]
mod input_marker;
#[cfg(any(windows, test))]
mod lifetime;
#[cfg(any(windows, test))]
mod operator_control;
#[cfg(any(windows, test))]
mod owner_registry;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod protocol;
mod trusted;
#[cfg(any(windows, test))]
mod wire;

pub use driver::NativeComputerDriver;
pub use human_appshot::{
    ForegroundAppshot, HumanAppshot, prepare_foreground_appshot, run_appshot_worker_if_requested,
};
pub use trusted::{NativeProcessIdentity, TrustedComputerContext};

/// Dispatch the fixed private helper mode before composing a normal host runtime.
/// No extra command-line arguments, shell or public listener are accepted.
pub fn run_if_requested() -> Option<std::process::ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("__computer-helper-v1")) {
        return None;
    }
    if args.next().is_some() {
        return Some(std::process::ExitCode::FAILURE);
    }
    #[cfg(windows)]
    return Some(if platform::entry().is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    });
    #[cfg(not(windows))]
    Some(std::process::ExitCode::FAILURE)
}
