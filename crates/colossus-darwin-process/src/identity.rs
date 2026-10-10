//! Genuine kernel audit identities and identity-bound signal dispatch.

use std::{fmt, io, mem, sync::OnceLock};

const TASK_AUDIT_TOKEN: libc::task_flavor_t = 15;
const AUDIT_TOKEN_WORDS: libc::mach_msg_type_number_t = 8;

#[repr(C)]
#[derive(Clone, Eq, PartialEq)]
struct AuditToken {
    words: [u32; AUDIT_TOKEN_WORDS as usize],
}

unsafe extern "C" {
    static mach_task_self_: libc::mach_port_t;
    fn task_name_for_pid(
        task: libc::mach_port_t,
        pid: libc::pid_t,
        name: *mut libc::mach_port_t,
    ) -> libc::kern_return_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

/// A genuine process identity copied from the kernel's `TASK_AUDIT_TOKEN`.
///
/// The token has no public constructor, raw accessor, or serialization. It binds
/// the process incarnation and exec generation; it does not attest executable
/// bytes, sandbox policy, or continuing membership in a supervised domain.
#[derive(Clone, Eq, PartialEq)]
pub struct DarwinProcessIdentity {
    token: AuditToken,
}

impl DarwinProcessIdentity {
    pub(crate) fn audit_token_words(&self) -> &[u32; 8] {
        &self.token.words
    }

    pub(crate) fn matches_audit_token(&self, words: &[u32; 8]) -> bool {
        &self.token.words == words
    }

    /// Bind one live process through its kernel task-name port.
    ///
    /// Unreadable, exited, or zombie processes fail closed. Callers must retain
    /// their independent allocation owner while identity lookup is unresolved.
    pub fn bind(pid: u32) -> io::Result<Self> {
        let pid = libc::pid_t::try_from(pid)
            .ok()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| invalid("Darwin process identifier must be positive"))?;
        let mut name = 0;
        // SAFETY: `name` is writable for one Mach port name. The task-self send
        // right belongs to this process; no caller pointer escapes the call.
        let result = unsafe { task_name_for_pid(self_task(), pid, &mut name) };
        let port = TaskName(name);
        if result != 0 {
            return Err(lookup_error(pid, "task-name lookup", result));
        }
        if name == 0 {
            return Err(io::Error::other("Darwin returned an absent task-name port"));
        }
        let mut token = AuditToken {
            words: [0; AUDIT_TOKEN_WORDS as usize],
        };
        let mut count = AUDIT_TOKEN_WORDS;
        // SAFETY: the owned task-name send right remains alive for this call.
        // TASK_AUDIT_TOKEN writes at most eight 32-bit words into `token`; count
        // is initialized to that exact capacity and is separately writable.
        let result = unsafe {
            libc::task_info(
                port.0,
                TASK_AUDIT_TOKEN,
                token.words.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if result != 0 {
            return Err(lookup_error(pid, "audit-token lookup", result));
        }
        if count != AUDIT_TOKEN_WORDS || token.words[5] != pid.cast_unsigned() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Darwin returned an inconsistent process audit token",
            ));
        }
        Ok(Self { token })
    }

    /// Process identifier embedded in the genuine kernel token.
    pub fn pid(&self) -> u32 {
        self.token.words[5]
    }

    /// Kernel audit session inherited across ordinary fork, setsid and reparenting.
    ///
    /// This value alone is not a retained session capability or an immutable
    /// domain: a valid foreign session Mach right can change session membership.
    pub fn audit_session_id(&self) -> u32 {
        self.token.words[6]
    }

    /// Effective user identifier at the moment the kernel token was copied.
    pub fn effective_uid(&self) -> u32 {
        self.token.words[1]
    }

    /// Real user identifier at the moment the kernel token was copied.
    pub fn real_uid(&self) -> u32 {
        self.token.words[3]
    }

    /// Signal only this kernel process incarnation and exec generation.
    ///
    /// Darwin also checks the caller's ordinary signal authority. An intervening
    /// exec or PID reuse returns `ESRCH`; an absent supported entrypoint returns
    /// `Unsupported`. Signal zero is intentionally excluded: this API rejects it
    /// and it cannot serve as a live identity check.
    pub fn signal(&self, signal: DarwinProcessSignal) -> io::Result<()> {
        let call = signal_call().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Darwin audit-token signaling is unavailable",
            )
        })?;
        let mut token = self.token.clone();
        // SAFETY: dlsym resolved the fixed OS entrypoint with the SDK's exact ABI.
        // `token` is a genuine, fully initialized 32-byte kernel audit token and
        // remains alive throughout this synchronous call. The signal is closed.
        let result = unsafe { call(&mut token, signal.number()) };
        if result == 0 {
            Ok(())
        } else if result > 0 {
            // Unlike kill(2), this libproc interface returns a positive errno.
            Err(io::Error::from_raw_os_error(result))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Darwin returned an invalid audit-token signal result",
            ))
        }
    }
}

impl fmt::Debug for DarwinProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DarwinProcessIdentity")
            .field("pid", &self.pid())
            .field("audit_session_id", &self.audit_session_id())
            .field("real_uid", &self.real_uid())
            .field("effective_uid", &self.effective_uid())
            .finish_non_exhaustive()
    }
}

/// Closed signals used by an independently authorized native supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DarwinProcessSignal {
    /// Suspend an exact process pending supervised reconciliation.
    Stop,
    /// Continue an exact process after native validation.
    Continue,
    /// Request orderly termination of an exact process.
    Terminate,
    /// Force termination of an exact process; separately acknowledge its exit.
    Kill,
}

impl DarwinProcessSignal {
    fn number(self) -> libc::c_int {
        match self {
            Self::Stop => libc::SIGSTOP,
            Self::Continue => libc::SIGCONT,
            Self::Terminate => libc::SIGTERM,
            Self::Kill => libc::SIGKILL,
        }
    }
}

/// Whether the installed OS exposes the required identity-bound signal entrypoint.
///
/// This is an ABI prerequisite only, not browser containment acceptance.
pub fn audit_token_signals_supported() -> bool {
    signal_call().is_some()
}

type SignalCall = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;

fn signal_call() -> Option<SignalCall> {
    static SIGNAL: OnceLock<Option<SignalCall>> = OnceLock::new();
    *SIGNAL.get_or_init(|| {
        // SAFETY: the fixed NUL-terminated symbol name is valid for the call.
        // RTLD_DEFAULT searches the process's already loaded native OS libraries.
        let symbol =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
        if symbol.is_null() {
            None
        } else {
            // SAFETY: the SDK declares this exact function signature for the
            // resolved named symbol. No user-provided address or ABI is accepted.
            Some(unsafe { mem::transmute::<*mut libc::c_void, SignalCall>(symbol) })
        }
    })
}

struct TaskName(libc::mach_port_t);

impl Drop for TaskName {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: this value exclusively owns the task-name send right
            // returned by task_name_for_pid, and releases that right exactly once.
            let _ = unsafe { mach_port_deallocate(self_task(), self.0) };
        }
    }
}

fn self_task() -> libc::mach_port_t {
    // SAFETY: libSystem initializes this process-owned immutable task-self port.
    unsafe { mach_task_self_ }
}

fn lookup_error(pid: libc::pid_t, operation: &str, status: libc::kern_return_t) -> io::Error {
    let mut info: libc::proc_bsdinfo = unsafe {
        // SAFETY: this SDK C record contains integer and fixed-array fields; all
        // zero bits are valid initialized storage for a subsequent bounded write.
        mem::zeroed()
    };
    // SAFETY: __error returns this thread's writable errno slot.
    unsafe { *libc::__error() = 0 };
    // SAFETY: `info` is writable for the exact size passed. proc_pidinfo copies
    // a fixed SDK record and retains no pointer to this stack allocation.
    let copied = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            // This documented flavor's nonzero argument includes the zombie list.
            1,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
        )
    };
    if copied == mem::size_of::<libc::proc_bsdinfo>() as libc::c_int
        && info.pbi_status == libc::SZOMB
    {
        return io::Error::new(
            io::ErrorKind::WouldBlock,
            "Darwin process is a zombie awaiting independent reaping",
        );
    }
    if copied == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        return io::Error::from_raw_os_error(libc::ESRCH);
    }
    io::Error::other(format!("Darwin {operation} failed (Mach status {status})"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
