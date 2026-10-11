//! Native Unix process entry; helpers and descriptors never come from model input.
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::ffi::{OsStr, OsString};
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt as _;
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream;
#[cfg(target_os = "macos")]
use std::{
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

use colossus_contracts::{BrowserCapabilities, BrowserMode};
use colossus_ports::BrowserDriverError;

#[cfg(target_os = "linux")]
pub use crate::container::Relay;

pub struct Entry {
    pub helper: Option<CString>,
    pub native_mode: i32,
}

/// Require the argument shape used by the signed front and active sandbox membership
/// before reading enrollment. The front's signature and exact applied policy
/// still require independent supervisor verification.
#[cfg(target_os = "macos")]
pub fn verify_main_entry(startup: &[OsString]) -> Result<(), BrowserDriverError> {
    if !fixed_main_arguments(startup) {
        return Err(BrowserDriverError::Denied);
    }
    // SAFETY: read-only process credentials and sandbox state, with no pointer
    // output. The front's exact policy/signature remains the launch authority.
    let (real_uid, effective_uid, sandboxed) = unsafe {
        (
            libc::getuid(),
            libc::geteuid(),
            sandbox_check(libc::getpid(), std::ptr::null(), 0),
        )
    };
    if real_uid == 0 || real_uid != effective_uid || sandboxed != 1 {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn fixed_main_arguments(startup: &[OsString]) -> bool {
    startup.len() == 4
        && startup
            .iter()
            .zip(["3", "4", "5", "6"])
            .all(|(observed, expected)| observed == OsStr::new(expected))
}

#[cfg(target_os = "macos")]
#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_check(
        pid: libc::pid_t,
        operation: *const libc::c_char,
        filter: libc::c_int,
        ...
    ) -> libc::c_int;
}

/// All four private streams must have one OS peer identity. This checks the
/// transport's provenance shape; enrollment authentication remains in the
/// browser bridge and the supervisor must prove the peer is its exact child.
#[cfg(target_os = "macos")]
pub fn verify_channel_peers(streams: &[UnixStream]) -> Result<(), BrowserDriverError> {
    if streams.len() != 4 {
        return Err(BrowserDriverError::Denied);
    }
    let mut expected = None;
    for stream in streams {
        let mut pid: libc::pid_t = 0;
        let mut length = std::mem::size_of_val(&pid) as libc::socklen_t;
        // SAFETY: getsockopt writes a peer PID into bounded stack storage.
        if unsafe {
            libc::getsockopt(
                std::os::fd::AsRawFd::as_raw_fd(stream),
                libc::SOL_LOCAL,
                libc::LOCAL_PEEREPID,
                std::ptr::from_mut(&mut pid).cast(),
                &mut length,
            )
        } != 0
            || length as usize != std::mem::size_of_val(&pid)
            || pid <= 1
        {
            return Err(BrowserDriverError::Denied);
        }
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: getpeereid writes two fixed-size peer credentials to valid
        // stack pointers. No caller-controlled identity is accepted as policy.
        if unsafe { libc::getpeereid(std::os::fd::AsRawFd::as_raw_fd(stream), &mut uid, &mut gid) }
            != 0
        {
            return Err(BrowserDriverError::Denied);
        }
        let peer = (pid, uid, gid);
        if expected.is_some_and(|expected| expected != peer) {
            return Err(BrowserDriverError::Denied);
        }
        expected = Some(peer);
    }
    Ok(())
}

/// Match CEF's direct loopback proxy to the immutable policy applied by the
/// signed front before Rust or CEF loaded. The macOS host has no in-process
/// relay: the supervisor owns this listener and the network-service helper may
/// connect only to the port in this resource.
#[cfg(target_os = "macos")]
pub fn verify_proxy_binding(
    address: &str,
    port: u16,
    profile: &Path,
) -> Result<(), BrowserDriverError> {
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|_| BrowserDriverError::Denied)?;
    let macos = executable.parent().ok_or(BrowserDriverError::Denied)?;
    let contents = macos.parent().ok_or(BrowserDriverError::Denied)?;
    let app = contents.parent().ok_or(BrowserDriverError::Denied)?;
    if executable.file_name() != Some(OsStr::new("colossus-native-browser-host Body"))
        || macos.file_name() != Some(OsStr::new("MacOS"))
        || contents.file_name() != Some(OsStr::new("Contents"))
        || app.extension() != Some(OsStr::new("app"))
    {
        return Err(BrowserDriverError::Denied);
    }
    let resource = contents.join("Resources/colossus-network-envelope.policy");
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(resource)
        .map_err(|_| BrowserDriverError::Denied)?;
    let before = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    // SAFETY: geteuid reads the current process credential without parameters.
    let owner = unsafe { libc::geteuid() };
    if !before.is_file()
        || before.uid() != owner
        || before.nlink() != 1
        || before.mode() & 0o7777 != 0o400
        || before.len() == 0
        || before.len() > 32 * 1024
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut bytes = vec![0; before.len() as usize];
    file.read_exact(&mut bytes)
        .map_err(|_| BrowserDriverError::Denied)?;
    let mut trailing = [0];
    if file
        .read(&mut trailing)
        .map_err(|_| BrowserDriverError::Denied)?
        != 0
    {
        return Err(BrowserDriverError::Denied);
    }
    let after = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    if (
        before.dev(),
        before.ino(),
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
    ) != (
        after.dev(),
        after.ino(),
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
    ) || !proxy_matches_policy(&bytes, app, address, port, profile)
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn proxy_matches_policy(
    bytes: &[u8],
    app: &Path,
    address: &str,
    port: u16,
    profile: &Path,
) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let fields = text.split('\n').collect::<Vec<_>>();
    let decimal = |value: &str| {
        (!value.is_empty()
            && !value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u32>().ok())
        .flatten()
    };
    fields.len() == 10
        && fields[0] == "COLOSSUS_MAC_NETWORK_ENVELOPE_V1"
        && decimal(fields[1]).is_some_and(|session| session != 0 && session != u32::MAX)
        && decimal(fields[2]) == Some(u32::from(port))
        && port != 0
        && fields[3] == app.parent().and_then(Path::to_str).unwrap_or_default()
        && fields[4] == app.to_str().unwrap_or_default()
        && fields[6] == profile.to_str().unwrap_or_default()
        && fields[9].is_empty()
        && address == "127.0.0.1"
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn signed_main_entry_accepts_only_four_fixed_private_fds() {
        assert!(fixed_main_arguments(
            &["3", "4", "5", "6"].map(OsString::from)
        ));
        for rejected in [
            vec!["3", "4", "5"],
            vec!["3", "4", "5", "6", "7"],
            vec!["3", "4", "5", "5"],
            vec!["6", "5", "4", "3"],
            vec!["--oci-presentation-sockets"],
        ] {
            assert!(!fixed_main_arguments(
                &rejected.into_iter().map(OsString::from).collect::<Vec<_>>()
            ));
        }
    }

    #[test]
    fn private_streams_have_one_os_peer_identity() {
        let pairs = (0..4)
            .map(|_| UnixStream::pair().expect("private pair"))
            .collect::<Vec<_>>();
        let streams = pairs
            .iter()
            .map(|(_, child)| child.try_clone().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(verify_channel_peers(&streams), Ok(()));
        assert_eq!(verify_channel_peers(&[]), Err(BrowserDriverError::Denied));
        assert_eq!(
            verify_channel_peers(&streams[..3]),
            Err(BrowserDriverError::Denied)
        );
    }

    #[test]
    fn signed_proxy_resource_must_match_the_exact_supervisor_port_and_profile() {
        let app = Path::new("/private/owned/Colossus Browser Host.app");
        let profile = Path::new("/private/owned/profile");
        let resource = "COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n123\n18081\n/private/owned\n/private/owned/Colossus Browser Host.app\ncom.obscuritylabs.colossus.native-browser-host.preview\n/private/owned/profile\n/private/broker\n/private/home\n";
        assert!(proxy_matches_policy(
            resource.as_bytes(),
            app,
            "127.0.0.1",
            18081,
            profile
        ));
        assert!(!proxy_matches_policy(
            resource.as_bytes(),
            app,
            "127.0.0.1",
            18082,
            profile
        ));
        assert!(!proxy_matches_policy(
            resource.as_bytes(),
            app,
            "localhost",
            18081,
            profile
        ));
        assert!(!proxy_matches_policy(
            resource.as_bytes(),
            app,
            "127.0.0.1",
            18081,
            Path::new("/private/other/profile")
        ));
        let malformed = resource.replace("123\n", "0123\n");
        assert!(!proxy_matches_policy(
            malformed.as_bytes(),
            app,
            "127.0.0.1",
            18081,
            profile
        ));
        let invalid_session = resource.replace("123\n", "0\n");
        assert!(!proxy_matches_policy(
            invalid_session.as_bytes(),
            app,
            "127.0.0.1",
            18081,
            profile
        ));
    }
}

impl Entry {
    pub fn prepare(
        mode: BrowserMode,
        capabilities: &BrowserCapabilities,
    ) -> Result<Self, BrowserDriverError> {
        #[cfg(target_os = "linux")]
        {
            let _ = capabilities;
            Ok(Self {
                helper: None,
                native_mode: match mode {
                    BrowserMode::Headless => 1,
                    BrowserMode::Embedded => 2,
                },
            })
        }
        #[cfg(target_os = "macos")]
        {
            // WindowServer-free deployment and owned Keychain stores have not
            // been accepted on macOS. An OSR desktop host cannot claim either.
            if mode != BrowserMode::Embedded
                || capabilities.private_ca_trust
                || capabilities.client_identities
            {
                return Err(BrowserDriverError::Unsupported);
            }
            let executable = std::env::current_exe()
                .and_then(|path| path.canonicalize())
                .map_err(|_| {
                    #[cfg(feature = "macos-synthetic-profile-diagnostic")]
                    eprintln!("Mac diagnostic entry: current executable unavailable");
                    BrowserDriverError::Unavailable
                })?;
            let macos = executable.parent().ok_or(BrowserDriverError::Denied)?;
            let contents = macos.parent().ok_or(BrowserDriverError::Denied)?;
            let app = contents.parent().ok_or(BrowserDriverError::Denied)?;
            if macos.file_name() != Some(std::ffi::OsStr::new("MacOS"))
                || contents.file_name() != Some(std::ffi::OsStr::new("Contents"))
                || app.extension() != Some(std::ffi::OsStr::new("app"))
            {
                return Err(BrowserDriverError::Denied);
            }
            let helper = contents.join(
                "Frameworks/Colossus Browser Helper.app/Contents/MacOS/Colossus Browser Helper",
            );
            let metadata = std::fs::symlink_metadata(&helper).map_err(|_| {
                #[cfg(feature = "macos-synthetic-profile-diagnostic")]
                eprintln!("Mac diagnostic entry: fixed helper metadata unavailable");
                BrowserDriverError::Unavailable
            })?;
            use std::os::unix::fs::PermissionsExt as _;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.permissions().mode() & 0o111 == 0
                || helper
                    .canonicalize()
                    .map_err(|_| BrowserDriverError::Denied)?
                    != helper
            {
                return Err(BrowserDriverError::Denied);
            }
            Ok(Self {
                helper: Some(
                    CString::new(helper.as_os_str().as_bytes())
                        .map_err(|_| BrowserDriverError::Denied)?,
                ),
                native_mode: 2,
            })
        }
    }
}

pub fn close_unrelated_descriptors(kept: &[i32]) -> Result<(), BrowserDriverError> {
    #[cfg(target_os = "linux")]
    {
        let mut kept = kept
            .iter()
            .map(|descriptor| *descriptor as u32)
            .collect::<Vec<_>>();
        kept.sort_unstable();
        let mut first = 3;
        for last in kept.into_iter().chain(std::iter::once(u32::MAX)) {
            let end = if last == u32::MAX { last } else { last - 1 };
            if first <= end {
                // SAFETY: bootstrap has one thread and exclusively owns all
                // retained sockets; each interval excludes them and stdio.
                if unsafe { libc::syscall(libc::SYS_close_range, first, end, 0_u32) } != 0 {
                    return Err(BrowserDriverError::Unavailable);
                }
            }
            if last == u32::MAX {
                break;
            }
            first = last + 1;
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // libproc reports this process's descriptors without creating a new
        // descriptor. No threads or CEF objects can race this closed bootstrap.
        const MAX_DESCRIPTORS: usize = 8192;
        let mut descriptors = vec![
            libc::proc_fdinfo {
                proc_fd: -1,
                proc_fdtype: 0
            };
            MAX_DESCRIPTORS
        ];
        let capacity = std::mem::size_of_val(descriptors.as_slice());
        for pass in 0..2 {
            // SAFETY: live bounded proc_fdinfo array, exact byte capacity, own PID.
            let length = unsafe {
                libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDLISTFDS,
                    0,
                    descriptors.as_mut_ptr().cast(),
                    capacity as libc::c_int,
                )
            };
            if length <= 0
                || length as usize >= capacity
                || !(length as usize).is_multiple_of(std::mem::size_of::<libc::proc_fdinfo>())
            {
                return Err(BrowserDriverError::Unavailable);
            }
            let count = length as usize / std::mem::size_of::<libc::proc_fdinfo>();
            for descriptor in &descriptors[..count] {
                if descriptor.proc_fd >= 3 && !kept.contains(&descriptor.proc_fd) {
                    if pass != 0 {
                        return Err(BrowserDriverError::Unavailable);
                    }
                    // SAFETY: positively enumerated own fd; no concurrent thread
                    // can close, reopen, or take ownership during this boundary.
                    if unsafe { libc::close(descriptor.proc_fd) } != 0 {
                        return Err(BrowserDriverError::Unavailable);
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn container_channels(
    presentation: bool,
) -> Result<Vec<std::os::unix::net::UnixStream>, BrowserDriverError> {
    #[cfg(target_os = "linux")]
    {
        crate::container::channels(presentation)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = presentation;
        Err(BrowserDriverError::Unsupported)
    }
}
