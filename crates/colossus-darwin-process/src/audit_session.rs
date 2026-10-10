//! Current-session Mach reference retention, without session transfer authority.

use std::{fmt, io, mem, sync::OnceLock};

use crate::DarwinProcessIdentity;

unsafe extern "C" {
    static mach_task_self_: libc::mach_port_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

/// An opaque send right retaining the calling process's genuine audit session.
///
/// Outstanding session send rights hold a kernel session reference and prevent
/// its ASID from being reused while retained. This type grants no public port
/// access, transfer, join, or serialization. It does not fence membership or
/// admit browser containment. Loss of the retaining process remains an unknown
/// ownership obligation; launchd restart alone creates a different session.
pub struct RetainedDarwinAuditSession {
    owner: DarwinProcessIdentity,
    _right: SessionSendRight,
}

impl RetainedDarwinAuditSession {
    pub(crate) fn send_name(&self) -> libc::mach_port_t {
        self._right.0
    }

    /// Retain the current assigned session and bind its genuine kernel owner.
    ///
    /// The private SDK entrypoint is resolved at runtime and missing support
    /// fails explicitly. Call before concurrent security-session manipulation:
    /// differing kernel identities around acquisition reject the result.
    pub fn current() -> io::Result<Self> {
        let owner = DarwinProcessIdentity::bind(std::process::id())?;
        if matches!(owner.audit_session_id(), 0 | u32::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Darwin owner has no assigned audit session",
            ));
        }
        let call = current_session_call().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Darwin audit-session retention is unavailable",
            )
        })?;
        // SAFETY: __error returns this thread's writable errno slot.
        unsafe { *libc::__error() = 0 };
        // SAFETY: dlsym resolved the SDK's fixed no-argument kernel interface.
        // Success returns one owned send right in this process's Mach namespace.
        let name = unsafe { call() };
        if matches!(name, 0 | u32::MAX) {
            let error = io::Error::last_os_error();
            return Err(if error.raw_os_error().is_some_and(|errno| errno != 0) {
                error
            } else {
                io::Error::other("Darwin returned an absent audit-session send right")
            });
        }
        let right = SessionSendRight(name);
        if owner != DarwinProcessIdentity::bind(std::process::id())? {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Darwin owner changed identity during audit-session retention",
            ));
        }
        Ok(Self {
            owner,
            _right: right,
        })
    }

    /// ASID retained by this process-owned kernel send reference.
    pub fn audit_session_id(&self) -> u32 {
        self.owner.audit_session_id()
    }

    /// Genuine owner identity observed during the send-right acquisition.
    ///
    /// A later owner exec, session change or crash requires independent fencing;
    /// this original token does not certify the owner's continued liveness.
    pub fn owner_identity(&self) -> &DarwinProcessIdentity {
        &self.owner
    }
}

impl fmt::Debug for RetainedDarwinAuditSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedDarwinAuditSession")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

type CurrentSessionCall = unsafe extern "C" fn() -> libc::mach_port_t;

fn current_session_call() -> Option<CurrentSessionCall> {
    static CURRENT: OnceLock<Option<CurrentSessionCall>> = OnceLock::new();
    *CURRENT.get_or_init(|| {
        // SAFETY: this fixed SIP-protected OS image is already loaded through
        // libSystem. NOLOAD prevents loading caller-controlled executable code;
        // FIRST restricts symbol lookup to this precise native SDK image. The
        // process-lifetime reference intentionally keeps the function ABI live.
        let library = unsafe {
            libc::dlopen(
                c"/usr/lib/system/libsystem_kernel.dylib".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL | libc::RTLD_NOLOAD | libc::RTLD_FIRST,
            )
        };
        if library.is_null() {
            return None;
        }
        // SAFETY: fixed NUL-terminated SDK symbol resolved only in the fixed OS
        // image above, without RTLD_DEFAULT or any caller-selected address.
        let symbol = unsafe { libc::dlsym(library, c"audit_session_self".as_ptr()) };
        if symbol.is_null() {
            None
        } else {
            // SAFETY: the SDK declares this exact no-argument return ABI for the
            // fixed symbol. Missing support was rejected before transmutation.
            Some(unsafe { mem::transmute::<*mut libc::c_void, CurrentSessionCall>(symbol) })
        }
    })
}

struct SessionSendRight(libc::mach_port_t);

impl Drop for SessionSendRight {
    fn drop(&mut self) {
        // SAFETY: this value owns one send reference returned by audit_session_self.
        // libSystem supplies this process's immutable task-self port. Deallocation
        // releases exactly one reference without exposing or joining the session.
        let _ = unsafe { mach_port_deallocate(mach_task_self_, self.0) };
    }
}

#[cfg(test)]
mod tests;
