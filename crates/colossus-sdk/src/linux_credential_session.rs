//! Narrow native authority for the Linux Secret Service session.
//!
//! A managed child has no ambient environment. Its one session locator comes from
//! the native host and must resolve to a local Unix bus owned by the same OS user.

use crate::{SdkError, SdkResult};
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{os::unix::ffi::OsStringExt as _, path::PathBuf, time::Duration};

const INVALID_SESSION: SdkError = SdkError::InvalidConfiguration(
    "Linux Managed Local requires a same-user local Unix Secret Service session",
);

pub(super) async fn verified_address() -> SdkResult<String> {
    let uid = rustix::process::getuid();
    let address = match std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        Ok(address) => address,
        Err(std::env::VarError::NotPresent) => format!("unix:path=/run/user/{}/bus", uid.as_raw()),
        Err(std::env::VarError::NotUnicode(_)) => return Err(INVALID_SESSION),
    };
    verify(&address, uid).await?;
    Ok(address)
}

async fn verify(address: &str, expected_uid: rustix::process::Uid) -> SdkResult<()> {
    let socket_address = parse(address)?;
    let socket = rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .map_err(|_| INVALID_SESSION)?;
    match rustix::net::connect(&socket, &socket_address) {
        Ok(()) | Err(rustix::io::Errno::INPROGRESS) => {}
        Err(_) => return Err(INVALID_SESSION),
    }
    let stream = tokio::net::UnixStream::from_std(socket.into()).map_err(|_| INVALID_SESSION)?;
    tokio::time::timeout(Duration::from_secs(2), stream.writable())
        .await
        .map_err(|_| INVALID_SESSION)?
        .map_err(|_| INVALID_SESSION)?;
    if stream.take_error().map_err(|_| INVALID_SESSION)?.is_some() {
        return Err(INVALID_SESSION);
    }
    let peer = rustix::net::sockopt::socket_peercred(&stream).map_err(|_| INVALID_SESSION)?;
    if peer.uid != expected_uid {
        return Err(INVALID_SESSION);
    }
    Ok(())
}

fn parse(address: &str) -> SdkResult<SocketAddrUnix> {
    // Multiple transports, TCP, autolaunch and unfamiliar parameters cannot turn
    // this non-secret native locator into network or process-launch authority.
    if address.len() > 1024 || !address.is_ascii() || address.contains(';') {
        return Err(INVALID_SESSION);
    }
    let parameters = address.strip_prefix("unix:").ok_or(INVALID_SESSION)?;
    let mut endpoint = None;
    let mut guid_seen = false;
    for parameter in parameters.split(',') {
        let (key, value) = parameter.split_once('=').ok_or(INVALID_SESSION)?;
        match key {
            "path" | "abstract" if endpoint.is_none() => {
                endpoint = Some((key, decode(value)?));
            }
            "guid" if !guid_seen => {
                if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(INVALID_SESSION);
                }
                guid_seen = true;
            }
            _ => return Err(INVALID_SESSION),
        }
    }
    let (kind, name) = endpoint.ok_or(INVALID_SESSION)?;
    if kind == "abstract" {
        return SocketAddrUnix::new_abstract_name(&name).map_err(|_| INVALID_SESSION);
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(name));
    if !path.is_absolute() {
        return Err(INVALID_SESSION);
    }
    // Do not let a filesystem alias select a different native credential session.
    let canonical = std::fs::canonicalize(&path).map_err(|_| INVALID_SESSION)?;
    if canonical != path {
        return Err(INVALID_SESSION);
    }
    SocketAddrUnix::new(&path).map_err(|_| INVALID_SESSION)
}

fn decode(value: &str) -> SdkResult<Vec<u8>> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            let high = bytes.next().and_then(hex_digit).ok_or(INVALID_SESSION)?;
            let low = bytes.next().and_then(hex_digit).ok_or(INVALID_SESSION)?;
            high * 16 + low
        } else if byte.is_ascii_alphanumeric() || b"_-/.\\".contains(&byte) {
            byte
        } else {
            return Err(INVALID_SESSION);
        };
        if byte == 0 {
            return Err(INVALID_SESSION);
        }
        decoded.push(byte);
    }
    if decoded.is_empty() {
        return Err(INVALID_SESSION);
    }
    Ok(decoded)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn rejects_remote_ambiguous_or_malformed_session_authority() {
        for address in [
            "tcp:host=example.test,port=1234",
            "autolaunch:",
            "unix:abstract=test;tcp:host=example.test",
            "unix:abstract=test,abstract=other",
            "unix:abstract=test,path=/tmp/bus",
            "unix:abstract=test,guid=bad",
            "unix:abstract=test,guid=00000000000000000000000000000000,guid=00000000000000000000000000000000",
            "unix:abstract=test,unknown=value",
            "unix:abstract=",
            "unix:abstract=%00name",
            "unix:abstract=bad%",
            "unix:abstract=bad%gg",
            "unix:path=relative",
        ] {
            assert!(parse(address).is_err(), "accepted {address}");
        }
    }

    #[tokio::test]
    async fn local_path_requires_exact_peer_uid_and_rejects_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bus");
        let _listener = UnixListener::bind(&path).unwrap();
        let address = format!("unix:path={}", path.display());
        let uid = rustix::process::getuid();
        verify(&address, uid).await.unwrap();
        let other_uid = rustix::process::Uid::from_raw(uid.as_raw().wrapping_add(1));
        assert!(verify(&address, other_uid).await.is_err());
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(parse(&format!("unix:path={}", alias.display())).is_err());
    }

    #[tokio::test]
    async fn local_abstract_bus_preserves_percent_encoded_names() {
        use std::os::linux::net::SocketAddrExt as _;
        let name = format!("colossus-session-{}+", uuid::Uuid::now_v7());
        let socket = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let _listener = UnixListener::bind_addr(&socket).unwrap();
        let encoded = name.replace('+', "%2b");
        let address = format!("unix:abstract={encoded},guid=00000000000000000000000000000000");
        verify(&address, rustix::process::getuid()).await.unwrap();
    }
}
