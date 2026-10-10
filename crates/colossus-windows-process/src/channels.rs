//! Atomic private channel inheritance for a trusted browser host.
use super::{SandboxedChild, SpawnRequest, WindowsProcessError};
use std::fs::File;

/// Parent ends of one exclusively inherited duplex channel.
/// Each direction is an anonymous pipe, with no externally connectable name.
pub struct PrivateProcessChannel {
    /// Read bytes written by the exact spawned child.
    pub reader: File,
    /// Write bytes to the exact spawned child.
    pub writer: File,
}

/// Full Job/AppContainer owner and the private parent-side channels it created.
pub struct SupervisedChild {
    /// Retain until all Job processes exit and all channel workers are drained.
    pub child: SandboxedChild,
    /// Bootstrap, data, control, and optional presentation, in that order.
    pub channels: Vec<PrivateProcessChannel>,
}

/// Atomically launch a trusted host with three or four private duplex channels.
///
/// The child arguments receive `--inherited-browser-pipes` followed by decimal
/// read/write HANDLE pairs. Values are native handles, never authentication keys.
/// All parent handles have inheritance disabled, and only the nominated child
/// handles enter the same atomic AppContainer/Job/HANDLE_LIST startup operation.
/// The child must positively validate pipe type/uniqueness and clear inheritance
/// before CEF creates helpers. Blocking reads require an owned cancellation/join
/// adapter; dropping a `tokio::fs::File` future does not prove an I/O thread exited.
pub fn spawn_with_private_channels(
    request: &SpawnRequest,
    count: usize,
) -> Result<SupervisedChild, WindowsProcessError> {
    super::api::validate_request(request)?;
    if !(3..=4).contains(&count)
        || request.arguments.len() + 1 + count * 2 > 256
        || request
            .arguments
            .iter()
            .any(|argument| argument == "--inherited-browser-pipes")
    {
        return Err(WindowsProcessError::Invalid(
            "private browser channel enrollment must contain three or four fresh channels".into(),
        ));
    }
    #[cfg(windows)]
    {
        super::windows_impl::spawn_with_channels(request, count)
    }
    #[cfg(not(windows))]
    {
        Err(WindowsProcessError::UnsupportedPlatform)
    }
}
