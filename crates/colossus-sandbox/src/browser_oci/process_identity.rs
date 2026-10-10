use std::{
    fs::{File, OpenOptions},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

use colossus_ports::BrowserDriverError;
use rustix::{
    event::{PollFd, PollFlags, Timespec},
    fd::OwnedFd,
    process::{Pid, PidfdFlags},
};
use tokio::net::UnixStream;

/// An actual living native executable in the exact retained container cgroup and
/// distinct network namespace. Numeric PID/UID equality alone permits PID reuse.
pub(super) struct ProcessIdentity {
    pid: u32,
    uid: u32,
    gid: u32,
    pidfd: OwnedFd,
    _network_namespace: File,
}
impl ProcessIdentity {
    pub(super) fn bind(
        pid: u32,
        uid: u32,
        gid: u32,
        container: &str,
        executable: &Path,
    ) -> Result<Self, BrowserDriverError> {
        let raw = i32::try_from(pid).map_err(|_| BrowserDriverError::Denied)?;
        let pidfd = rustix::process::pidfd_open(
            Pid::from_raw(raw).ok_or(BrowserDriverError::Denied)?,
            PidfdFlags::NONBLOCK,
        )
        .map_err(|_| BrowserDriverError::Unavailable)?;
        ensure_live(&pidfd)?;
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        let expected = executable
            .metadata()
            .map_err(|_| BrowserDriverError::Unavailable)?;
        let actual = proc
            .join("exe")
            .metadata()
            .map_err(|_| BrowserDriverError::Unavailable)?;
        if actual.dev() != expected.dev() || actual.ino() != expected.ino() {
            return Err(BrowserDriverError::Denied);
        }
        let cgroups =
            std::fs::read(proc.join("cgroup")).map_err(|_| BrowserDriverError::Unavailable)?;
        if cgroups.len() > 16 * 1024 {
            return Err(BrowserDriverError::Denied);
        }
        let cgroups = std::str::from_utf8(&cgroups).map_err(|_| BrowserDriverError::Denied)?;
        if !cgroups.lines().any(|line| {
            line.split(':').nth(2).is_some_and(|path| {
                path.split('/')
                    .any(|part| part == container || part == format!("docker-{container}.scope"))
            })
        }) {
            return Err(BrowserDriverError::Denied);
        }
        // These are trusted procfs kernel links, deliberately opened following
        // the namespace link and retained through the process/proxy lifetime.
        let namespace = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::CLOEXEC.bits() as i32)
            .open(proc.join("ns/net"))
            .map_err(|_| BrowserDriverError::Unavailable)?;
        let own =
            std::fs::metadata("/proc/self/ns/net").map_err(|_| BrowserDriverError::Unavailable)?;
        let child = namespace
            .metadata()
            .map_err(|_| BrowserDriverError::Unavailable)?;
        if own.dev() == child.dev() && own.ino() == child.ino() {
            return Err(BrowserDriverError::Denied);
        }
        ensure_live(&pidfd)?;
        Ok(Self {
            pid,
            uid,
            gid,
            pidfd,
            _network_namespace: namespace,
        })
    }

    pub(super) fn verify_peer(&self, stream: &UnixStream) -> Result<(), BrowserDriverError> {
        ensure_live(&self.pidfd)?;
        super::relay::verify_peer(stream, self.pid, self.uid, self.gid)?;
        ensure_live(&self.pidfd)
    }

    pub(super) fn exited(&self) -> Result<bool, BrowserDriverError> {
        let mut descriptors = [PollFd::new(&self.pidfd, PollFlags::IN)];
        let ready = rustix::event::poll(
            &mut descriptors,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if ready == 0 {
            return Ok(false);
        }
        if descriptors[0]
            .revents()
            .intersects(PollFlags::IN | PollFlags::HUP)
        {
            return Ok(true);
        }
        Err(BrowserDriverError::OutcomeUnknown)
    }
}

fn ensure_live(pidfd: &OwnedFd) -> Result<(), BrowserDriverError> {
    let mut descriptors = [PollFd::new(pidfd, PollFlags::IN)];
    if rustix::event::poll(
        &mut descriptors,
        Some(&Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
    )
    .map_err(|_| BrowserDriverError::Unavailable)?
        != 0
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}
