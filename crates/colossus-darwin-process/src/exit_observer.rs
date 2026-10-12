//! Kernel exit observations for an already retained process incarnation.

use std::{
    io, mem,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::ExitStatusExt,
    },
    process::ExitStatus,
    time::{Duration, Instant},
};

use crate::DarwinProcessIdentity;

/// A private kernel observer attached while the enrolled process is alive.
///
/// Registration checks the genuine identity before and after attaching the
/// kqueue filter. The kernel retains the process attachment, so subsequent PID
/// reuse cannot retarget this observer. This does not reap a non-child, observe
/// descendants, fence creation or prove removal of its native resources.
pub struct DarwinProcessExitObserver {
    queue: OwnedFd,
    identity: DarwinProcessIdentity,
    exit: Option<DarwinProcessExit>,
}

/// A kernel-reported exit of the observer's enrolled process.
#[derive(Clone, Debug)]
pub struct DarwinProcessExit {
    identity: DarwinProcessIdentity,
    status: ExitStatus,
}

impl DarwinProcessExit {
    /// Identity checked when the kernel process filter was attached.
    pub fn identity(&self) -> &DarwinProcessIdentity {
        &self.identity
    }

    /// Kernel exit status; separate from the actual parent's reap evidence.
    pub fn status(&self) -> ExitStatus {
        self.status
    }
}

impl DarwinProcessExitObserver {
    /// Attach to a positively enrolled live process, before permitting its exit.
    ///
    /// An exit, exec or identity change during attachment fails closed. No PID
    /// lookup after exit is accepted as replacement evidence. Callers still own
    /// cleanup when registration fails or an event does not arrive.
    pub fn register(identity: &DarwinProcessIdentity) -> io::Result<Self> {
        require_identity(identity)?;
        // SAFETY: kqueue has no pointer arguments and returns a new owned fd.
        let raw = unsafe { libc::kqueue() };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful kqueue call returned this exclusively owned fd.
        let queue = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: F_SETFD changes only flags on this live, owned descriptor.
        if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let change = libc::kevent {
            ident: identity.pid() as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT | libc::EV_RECEIPT,
            fflags: libc::NOTE_EXIT | libc::NOTE_EXITSTATUS,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let mut receipt = empty_event();
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: both fixed records and timeout remain valid for this call.
        let count = unsafe { libc::kevent(queue.as_raw_fd(), &change, 1, &mut receipt, 1, &zero) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count != 1 || receipt.flags & libc::EV_ERROR == 0 {
            return Err(invalid("absent Darwin process registration receipt"));
        }
        if receipt.data != 0 {
            let errno = i32::try_from(receipt.data)
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| invalid("invalid Darwin process registration error"))?;
            return Err(io::Error::from_raw_os_error(errno));
        }
        require_identity(identity)?;
        Ok(Self {
            queue,
            identity: identity.clone(),
            exit: None,
        })
    }

    /// Wait at most sixty seconds for the enrolled process's kernel exit event.
    ///
    /// A timeout returns `None` and preserves the observer for a later wait.
    /// Interrupted waits retain the same absolute deadline. A successful event
    /// is retained, making later observations stable without a new PID lookup.
    pub fn wait(&mut self, timeout: Duration) -> io::Result<Option<DarwinProcessExit>> {
        if timeout > Duration::from_secs(60) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "exit observation timeout exceeds bound",
            ));
        }
        if let Some(exit) = &self.exit {
            return Ok(Some(exit.clone()));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = libc::timespec {
                tv_sec: remaining.as_secs() as libc::time_t,
                tv_nsec: remaining.subsec_nanos() as libc::c_long,
            };
            let mut event = empty_event();
            // SAFETY: the queue is owned, there are no changes, and the event and
            // bounded timeout records remain valid for the synchronous call.
            let count = unsafe {
                libc::kevent(
                    self.queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    &mut event,
                    1,
                    &timeout,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted && Instant::now() < deadline {
                    continue;
                }
                return Err(error);
            }
            if count == 0 {
                return Ok(None);
            }
            if count != 1
                || event.ident != self.identity.pid() as libc::uintptr_t
                || event.filter != libc::EVFILT_PROC
                || event.flags & libc::EV_ERROR != 0
                || event.fflags & libc::NOTE_EXIT == 0
                || event.fflags & libc::NOTE_EXITSTATUS == 0
            {
                return Err(invalid("invalid Darwin process exit event"));
            }
            let raw = i32::try_from(event.data)
                .map_err(|_| invalid("invalid Darwin process exit status"))?;
            let exit = DarwinProcessExit {
                identity: self.identity.clone(),
                status: ExitStatus::from_raw(raw),
            };
            self.exit = Some(exit.clone());
            return Ok(Some(exit));
        }
    }
}

fn require_identity(identity: &DarwinProcessIdentity) -> io::Result<()> {
    if DarwinProcessIdentity::bind(identity.pid())? != *identity {
        return Err(invalid(
            "Darwin process changed during exit observer registration",
        ));
    }
    Ok(())
}

fn empty_event() -> libc::kevent {
    // SAFETY: the SDK record consists of integer fields and a nullable pointer.
    unsafe { mem::zeroed() }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
