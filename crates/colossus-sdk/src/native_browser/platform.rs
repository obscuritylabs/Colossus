use super::*;
use std::{path::Path, time::Duration};

pub(super) fn endpoint(instance: &Path, enrollment: &NativeBrowserEnrollment) -> SdkResult<String> {
    #[cfg(unix)]
    {
        let _ = instance;
        return enrollment
            .unix_endpoint(rustix::process::geteuid().as_raw())
            .map_err(|_| SdkError::IdentityMismatch)?
            .to_str()
            .map(str::to_owned)
            .ok_or(SdkError::InvalidConfiguration(
                "native browser endpoint cannot be encoded",
            ));
    }
    #[cfg(windows)]
    {
        let _ = instance;
        let generation = hex::encode(enrollment.generation);
        return Ok(format!(
            r"\\.\pipe\colossus-native-browser-{}-{generation}",
            hex::encode(enrollment.instance)
        ));
    }
    #[allow(unreachable_code)]
    Err(SdkError::InvalidConfiguration(
        "native browser transport is unsupported",
    ))
}

#[cfg(unix)]
pub(super) type Stream = tokio::net::UnixStream;
#[cfg(windows)]
pub(super) type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(not(any(unix, windows)))]
pub(super) type Stream = tokio::io::DuplexStream;

#[cfg(unix)]
pub(super) async fn connect(endpoint: &str, child: u32) -> SdkResult<Stream> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    let path = Path::new(endpoint);
    let parent = path.parent().ok_or(SdkError::IdentityMismatch)?;
    let owner = std::fs::symlink_metadata(parent).map_err(|_| SdkError::IdentityMismatch)?;
    let socket = std::fs::symlink_metadata(path).map_err(|_| SdkError::SidecarFailed)?;
    if !owner.file_type().is_dir()
        || owner.uid() != rustix::process::geteuid().as_raw()
        || owner.mode() & 0o077 != 0
        || !socket.file_type().is_socket()
        || socket.uid() != owner.uid()
        || socket.mode() & 0o077 != 0
    {
        return Err(SdkError::IdentityMismatch);
    }
    let stream = tokio::time::timeout(Duration::from_secs(5), Stream::connect(endpoint))
        .await
        .map_err(|_| SdkError::SidecarFailed)?
        .map_err(|_| SdkError::SidecarFailed)?;
    let peer = stream.peer_cred().map_err(|_| SdkError::IdentityMismatch)?;
    if peer.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(child)
        || peer.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(stream)
}

#[cfg(windows)]
pub(super) async fn connect(endpoint: &str, child: u32) -> SdkResult<Stream> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
            Ok(stream) => {
                colossus_windows_native::validate_named_pipe_server(&stream, child)
                    .map_err(|_| SdkError::IdentityMismatch)?;
                return Ok(stream);
            }
            Err(error)
                if (error.raw_os_error() == Some(231)
                    || error.kind() == std::io::ErrorKind::NotFound)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(_) => return Err(SdkError::SidecarFailed),
        }
    }
}
#[cfg(not(any(unix, windows)))]
pub(super) async fn connect(_: &str, _: u32) -> SdkResult<Stream> {
    Err(SdkError::InvalidConfiguration(
        "native browser transport is unsupported",
    ))
}
