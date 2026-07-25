//! Safe, bounded Windows primitives shared by trusted Colossus host processes.
//!
//! Win32 calls are isolated here so the runtime, SDK, sidecar, and Desktop native
//! backend can remain `unsafe_code = "forbid"`. Path binding opens every component
//! without following a reparse point and retains those handles for the binding's
//! lifetime.

#![cfg_attr(windows, allow(unsafe_code))]

mod credentials;
mod error;
mod path;

pub use credentials::prompt_secret;
pub use error::WindowsNativeError;
pub use path::{BoundPath, FileIdentity};

#[cfg(windows)]
mod conpty;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use conpty::{
    ConptyChild, ConptyControl, DesktopTuiAuthenticationChannels, SpawnedConpty,
    spawn_verified_conpty, take_desktop_tui_authentication_channels,
};
#[cfg(windows)]
pub use process::{
    KillOnCloseJob, configure_suspended_process, install_bootstrap_pipe_as_standard_io,
    validate_named_pipe_client, validate_named_pipe_server,
};

#[cfg(test)]
mod tests;
