//! Five fixed native channels for an independently owned no-exec browser main.
//!
//! This is a creator API only, not a browser-facing descriptor nomination surface.

use std::{
    ffi::OsString,
    fs::File,
    io,
    os::{
        fd::{AsRawFd as _, OwnedFd},
        unix::net::UnixStream,
    },
    path::Path,
};

use crate::macos::{DarwinChild, FileActions, cloexec_pipe, normalize_channel_source_above};

const CHILD_CHANNELS: [libc::c_int; 5] = [3, 4, 5, 6, 7];
const FIRST_SOURCE_FD: libc::c_int = 8;

/// Five private parent socket endpoints and separate output pipes for one child.
///
/// Parent endpoints are close-on-exec and at least descriptor eight. The child's
/// bootstrap/data/control/presentation sockets are fixed descriptors 3/4/5/6, with creator launch/bootstrap at seven.
/// Its native entry must mark those inherited handles close-on-exec before any
/// helper starts. Dropping this value kills and reaps only its exact direct child;
/// descendants, retained audit sessions and native shutdown remain separate owners.
pub struct SpawnedOwnedBrowserChannels {
    /// Exact direct child, stopped before its first userspace instruction.
    pub child: DarwinChild,
    /// Native admission/bootstrap channel, connected to child descriptor three.
    pub bootstrap: UnixStream,
    /// Browser operation channel, connected to child descriptor four.
    pub data: UnixStream,
    /// Cancellation and shutdown channel, connected to child descriptor five.
    pub control: UnixStream,
    /// Trusted native presentation channel, connected to child descriptor six.
    pub presentation: UnixStream,
    /// Creator-only launch/control stream connected to Main descriptor seven.
    pub creator: UnixStream,
    /// Parent reader connected only to the child's standard output.
    pub stdout: File,
    /// Parent reader connected only to the child's standard error.
    pub stderr: File,
}

/// Four Core channels and an exact child whose native keeper endpoint is direct.
/// Unlike the ordinary five-channel result, no Core copy of descriptor seven is
/// returned. The keeper endpoint is consumed before the creator can resume.
pub struct SpawnedOwnedBrowserChannelsWithKeeper {
    /// Retained exact stopped direct child.
    pub child: DarwinChild,
    /// Native admission channel connected to fixed descriptor three.
    pub bootstrap: UnixStream,
    /// Core operation/lifetime channel connected to fixed descriptor four.
    pub data: UnixStream,
    /// Cancellation channel connected to fixed descriptor five.
    pub control: UnixStream,
    /// Presentation channel connected to fixed descriptor six.
    pub presentation: UnixStream,
    /// Separate exact-child stdout reader.
    pub stdout: File,
    /// Separate exact-child stderr reader.
    pub stderr: File,
}

/// Spawn a stopped creator with a consumed direct native keeper channel at seven.
/// The supplied endpoint must be a connected Unix stream created by trusted
/// native composition. No raw descriptor selector is accepted. Every undeclared
/// descriptor is closed in the child; the consumed endpoint leaves no retained
/// Core relay/copy, so ordinary Core death cannot sever creator-to-keeper traffic.
/// Independent signed entry and cleanup obligations are unchanged.
pub fn spawn_suspended_owned_browser_channels_with_keeper(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
    keeper: UnixStream,
) -> io::Result<SpawnedOwnedBrowserChannelsWithKeeper> {
    // Querying a peer rejects an unconnected/listening endpoint before spawn.
    keeper.peer_addr()?;
    let duplicate = keeper.try_clone()?;
    drop(keeper);
    let keeper: OwnedFd = duplicate.into();
    let keeper = normalize_channel_source_above(keeper, FIRST_SOURCE_FD)?;
    // SAFETY: this newly owned live socket has no concurrent flag mutation.
    if unsafe { libc::fcntl(keeper.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let bootstrap = private_pair()?;
    let data = private_pair()?;
    let control = private_pair()?;
    let presentation = private_pair()?;
    let (stdout, child_stdout) = output_pipe()?;
    let (stderr, child_stderr) = output_pipe()?;
    let pairs = [&bootstrap, &data, &control, &presentation];
    let mut actions = FileActions::new()?;
    actions.open(0, c"/dev/null", libc::O_RDONLY, 0)?;
    actions.dup2(child_stdout.as_raw_fd(), 1)?;
    actions.dup2(child_stderr.as_raw_fd(), 2)?;
    for (pair, destination) in pairs.iter().zip(CHILD_CHANNELS[..4].iter().copied()) {
        actions.dup2(pair.child.as_raw_fd(), destination)?;
    }
    actions.dup2(keeper.as_raw_fd(), CHILD_CHANNELS[4])?;
    for pair in pairs {
        actions.close(pair.child.as_raw_fd())?;
        actions.close(pair.parent.as_raw_fd())?;
    }
    actions.close(keeper.as_raw_fd())?;
    for fd in [&stdout, &child_stdout, &stderr, &child_stderr] {
        actions.close(fd.as_raw_fd())?;
    }
    let child = crate::macos::spawn_suspended(executable, arguments, environment, &actions)?;
    Ok(SpawnedOwnedBrowserChannelsWithKeeper {
        child,
        bootstrap: bootstrap.parent.into(),
        data: data.parent.into(),
        control: control.parent.into(),
        presentation: presentation.parent.into(),
        stdout: stdout.into(),
        stderr: stderr.into(),
    })
}

/// Spawn an exact Main image with five fixed private Unix stream channels.
///
/// The independently supervised creator must call this API, retain the returned
/// direct child before validation, and enroll its observer before resume. Core or
/// Desktop must not create Main themselves and then transfer only its PID.
///
/// Executable, arguments and environment come from trusted native composition.
/// No raw descriptor nomination is accepted. Standard input is `/dev/null`;
/// stdout and stderr use distinct anonymous pipes. Darwin closes undeclared child
/// handles and stops the child before userspace, even when caller stdio is closed.
/// The caller must validate its live signed code before explicitly resuming it.
/// This API does not authenticate channel frames, impose network containment or
/// acknowledge helper/session-wide cleanup.
///
/// # Errors
/// Returns native allocation, descriptor-action or spawn errors. Partial native
/// creation errors retain an unresolved cleanup obligation with the caller.
pub fn spawn_suspended_owned_browser_channels(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
) -> io::Result<SpawnedOwnedBrowserChannels> {
    let bootstrap = private_pair()?;
    let data = private_pair()?;
    let control = private_pair()?;
    let presentation = private_pair()?;
    let creator = private_pair()?;
    let (stdout, child_stdout) = output_pipe()?;
    let (stderr, child_stderr) = output_pipe()?;
    let pairs = [&bootstrap, &data, &control, &presentation, &creator];
    let mut actions = FileActions::new()?;
    actions.open(0, c"/dev/null", libc::O_RDONLY, 0)?;
    actions.dup2(child_stdout.as_raw_fd(), 1)?;
    actions.dup2(child_stderr.as_raw_fd(), 2)?;
    // All source endpoints were normalized above every destination first, so
    // earlier dup/open actions cannot overwrite later sources after closed stdio.
    for (pair, destination) in pairs.iter().zip(CHILD_CHANNELS) {
        actions.dup2(pair.child.as_raw_fd(), destination)?;
    }
    for pair in pairs {
        actions.close(pair.child.as_raw_fd())?;
        actions.close(pair.parent.as_raw_fd())?;
    }
    for fd in [&stdout, &child_stdout, &stderr, &child_stderr] {
        actions.close(fd.as_raw_fd())?;
    }
    let child = crate::macos::spawn_suspended(executable, arguments, environment, &actions)?;
    Ok(SpawnedOwnedBrowserChannels {
        child,
        bootstrap: bootstrap.parent.into(),
        data: data.parent.into(),
        control: control.parent.into(),
        presentation: presentation.parent.into(),
        creator: creator.parent.into(),
        stdout: stdout.into(),
        stderr: stderr.into(),
    })
}

struct SocketPair {
    parent: OwnedFd,
    child: OwnedFd,
}

fn private_pair() -> io::Result<SocketPair> {
    let (parent, child) = UnixStream::pair()?;
    let parent = normalize_channel_source_above(parent.into(), FIRST_SOURCE_FD)?;
    let child = normalize_channel_source_above(child.into(), FIRST_SOURCE_FD)?;
    for endpoint in [&parent, &child] {
        // SAFETY: each socket endpoint is live and uniquely owned. F_SETFD changes
        // only descriptor inheritance flags and accesses no caller memory.
        if unsafe { libc::fcntl(endpoint.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(SocketPair { parent, child })
}

fn output_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let (parent, child) = cloexec_pipe()?;
    Ok((
        normalize_channel_source_above(parent, FIRST_SOURCE_FD)?,
        normalize_channel_source_above(child, FIRST_SOURCE_FD)?,
    ))
}

#[cfg(test)]
mod direct_keeper_tests {
    use super::*;
    use std::os::fd::FromRawFd as _;

    #[test]
    fn disconnected_keeper_is_rejected_before_any_native_creation() {
        // SAFETY: socket creates one fresh Unix stream descriptor without a peer.
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0);
        // SAFETY: this test uniquely owns the successful socket descriptor.
        let stream = unsafe { UnixStream::from_raw_fd(raw) };
        let error = spawn_suspended_owned_browser_channels_with_keeper(
            Path::new("/private/tmp/never-created-direct-keeper-fixture"),
            &[],
            &[],
            stream,
        )
        .err()
        .expect("an unconnected endpoint must fail before spawn");
        assert_eq!(error.raw_os_error(), Some(libc::ENOTCONN));
    }
}
