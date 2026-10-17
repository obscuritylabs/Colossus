//! Fixed native-browser bootstrap handles, separate from process containment.

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

const CHILD_CHANNELS: [libc::c_int; 4] = [3, 4, 5, 6];
const FIRST_SOURCE_FD: libc::c_int = 7;

/// Four private parent socket endpoints and separate output pipes for one child.
///
/// Parent endpoints are close-on-exec and at least descriptor seven. The child's
/// bootstrap/data/control/presentation sockets are fixed descriptors 3/4/5/6.
/// Its native entry must mark those inherited handles close-on-exec before any
/// helper starts. Dropping this value kills and reaps only its exact direct child;
/// descendants, retained audit sessions and native shutdown remain separate owners.
pub struct SpawnedPrivateChannels {
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
    /// Parent reader connected only to the child's standard output.
    pub stdout: File,
    /// Parent reader connected only to the child's standard error.
    pub stderr: File,
}

/// Exact stopped child with one fixed native control stream at descriptor three.
///
/// This narrow entry is for a separately admitted native broker, not CEF. Output
/// has distinct pipes and no extra native capability descriptors are inherited.
pub struct SpawnedParentChannel {
    /// Retained exact child; validate live code before explicitly resuming it.
    pub child: DarwinChild,
    /// Private parent endpoint connected only to child descriptor three.
    pub channel: UnixStream,
    /// Parent reader for standard output.
    pub stdout: File,
    /// Parent reader for standard error.
    pub stderr: File,
}

/// Spawn one stopped native broker with a single fixed connected control stream.
///
/// All executable, argument and environment inputs belong to trusted composition.
/// No descriptor nomination or channel address is accepted. This does not attest
/// a sandbox, publisher policy, native shutdown or descendant cleanup.
///
/// # Errors
/// Returns allocation, descriptor-action or native spawn errors. As with other
/// Darwin launchers, the caller retains uncertainty after a partial spawn error.
pub fn spawn_suspended_parent_channel(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
) -> io::Result<SpawnedParentChannel> {
    let channel = private_pair()?;
    let (stdout, child_stdout) = output_pipe()?;
    let (stderr, child_stderr) = output_pipe()?;
    let mut actions = FileActions::new()?;
    actions.open(0, c"/dev/null", libc::O_RDONLY, 0)?;
    actions.dup2(child_stdout.as_raw_fd(), 1)?;
    actions.dup2(child_stderr.as_raw_fd(), 2)?;
    actions.dup2(channel.child.as_raw_fd(), 3)?;
    for endpoint in [
        &channel.child,
        &channel.parent,
        &stdout,
        &child_stdout,
        &stderr,
        &child_stderr,
    ] {
        actions.close(endpoint.as_raw_fd())?;
    }
    let child = crate::macos::spawn_suspended(executable, arguments, environment, &actions)?;
    Ok(SpawnedParentChannel {
        child,
        channel: channel.parent.into(),
        stdout: stdout.into(),
        stderr: stderr.into(),
    })
}

/// Spawn an exact executable with four fixed private Unix stream channels.
///
/// Executable, arguments and environment come from trusted native composition.
/// No raw descriptor nomination is accepted. Standard input is `/dev/null`;
/// stdout and stderr use distinct anonymous pipes. Darwin closes undeclared child
/// handles and stops the child before userspace, even when caller stdio is closed.
/// The caller must validate its live signed code before explicitly resuming it.
/// This API does not authenticate channel frames, impose network containment or
/// acknowledge helper/session-wide cleanup.
pub fn spawn_suspended_private_channels(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
) -> io::Result<SpawnedPrivateChannels> {
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
    Ok(SpawnedPrivateChannels {
        child,
        bootstrap: bootstrap.parent.into(),
        data: data.parent.into(),
        control: control.parent.into(),
        presentation: presentation.parent.into(),
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
