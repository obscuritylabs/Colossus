//! Bounded kernel UID census with genuine process audit identities.

use std::{collections::BTreeSet, io, mem};

use crate::DarwinProcessIdentity;

const PROC_UID_ONLY: u32 = 4;
const PROC_RUID_ONLY: u32 = 5;
const MAX_SNAPSHOT_PROCESSES: usize = 4096;

/// A bounded observation of genuine kernel identities in one credential domain.
///
/// Enumeration includes both real and effective UID membership. Unknown, exited,
/// zombie, truncated, or changing census results fail closed. This observation
/// does not itself retain a UID/ASID reservation or prevent later domain admission.
#[derive(Debug)]
pub struct DarwinProcessSnapshot {
    uid: u32,
    audit_session: Option<u32>,
    identities: Vec<DarwinProcessIdentity>,
}

impl DarwinProcessSnapshot {
    /// Inspect all real/effective-UID members within a finite ceiling (1..=4096).
    ///
    /// Errors retain the caller's cleanup obligation. Never interpret a denied
    /// task lookup, zombie, full buffer, or concurrent process change as emptiness.
    pub fn for_uid(uid: u32, maximum: usize) -> io::Result<Self> {
        if uid == u32::MAX || maximum == 0 || maximum > MAX_SNAPSHOT_PROCESSES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Darwin UID snapshot requires a valid UID and finite process ceiling",
            ));
        }
        let pids = census(uid, maximum)?;
        let mut identities = Vec::with_capacity(pids.len());
        for &pid in &pids {
            let identity = DarwinProcessIdentity::bind(pid)?;
            if identity.real_uid() != uid && identity.effective_uid() != uid {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Darwin process changed UID during bounded enumeration",
                ));
            }
            identities.push(identity);
        }
        if pids != census(uid, maximum)? {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Darwin process census changed during bounded enumeration",
            ));
        }
        Ok(Self {
            uid,
            audit_session: None,
            identities,
        })
    }

    /// Filter an already complete UID census by its genuine kernel ASID values.
    ///
    /// The caller must separately retain this ASID and prevent foreign joins or
    /// session changes. This method supplies no such lifetime or sandbox authority.
    pub fn for_audit_session(mut self, audit_session: u32) -> io::Result<Self> {
        if self.audit_session.is_some() || matches!(audit_session, 0 | u32::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Darwin audit-session filter requires an unfiltered UID census and concrete session",
            ));
        }
        self.identities
            .retain(|identity| identity.audit_session_id() == audit_session);
        self.audit_session = Some(audit_session);
        Ok(self)
    }

    /// Genuine process identities retained by this bounded observation.
    pub fn identities(&self) -> &[DarwinProcessIdentity] {
        &self.identities
    }

    /// Record a successful empty kernel census, never direct-child/group exit.
    ///
    /// This point-in-time evidence is insufficient for whole-browser cleanup.
    /// An independent owner must establish retained domain identity, immutable
    /// membership, fenced admission, and native shutdown before releasing state.
    pub fn empty_evidence(&self) -> Option<EmptyProcessDomainEvidence> {
        self.identities
            .is_empty()
            .then_some(EmptyProcessDomainEvidence {
                uid: self.uid,
                audit_session: self.audit_session,
            })
    }
}

/// Successful empty-census evidence whose constructor requires kernel enumeration.
///
/// It is neither a CEF shutdown acknowledgement nor a whole-tree lifetime receipt.
#[derive(Debug)]
pub struct EmptyProcessDomainEvidence {
    uid: u32,
    audit_session: Option<u32>,
}

impl EmptyProcessDomainEvidence {
    /// Real/effective UID included in the observed credential domain.
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Optional ASID filter applied to genuine kernel identity values.
    pub fn audit_session_id(&self) -> Option<u32> {
        self.audit_session
    }
}

fn census(uid: u32, maximum: usize) -> io::Result<BTreeSet<u32>> {
    let mut pids = BTreeSet::new();
    for filter in [PROC_UID_ONLY, PROC_RUID_ONLY] {
        // The spare element distinguishes an exact ceiling from truncation.
        let mut buffer = vec![0_i32; maximum + 1];
        let bytes = mem::size_of_val(buffer.as_slice());
        // SAFETY: __error returns this thread's writable errno slot. libproc can
        // report an error as zero bytes, so stale errno cannot be left in place.
        unsafe { *libc::__error() = 0 };
        // SAFETY: `buffer` is initialized writable storage for exactly `bytes`.
        // The filter is a fixed SDK UID census and the count is bounded above.
        let copied = unsafe {
            libc::proc_listpids(
                filter,
                uid,
                buffer.as_mut_ptr().cast(),
                bytes as libc::c_int,
            )
        };
        let error = io::Error::last_os_error();
        if copied < 0 || copied == 0 && error.raw_os_error().is_some_and(|errno| errno != 0) {
            return Err(error);
        }
        let copied = usize::try_from(copied).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Darwin returned an invalid census length",
            )
        })?;
        if copied > bytes || copied % mem::size_of::<i32>() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Darwin returned an inconsistent process census",
            ));
        }
        let count = copied / mem::size_of::<i32>();
        if count > maximum {
            return Err(io::Error::other(
                "Darwin process census exceeds its finite ceiling",
            ));
        }
        for &pid in &buffer[..count] {
            let pid = u32::try_from(pid)
                .ok()
                .filter(|pid| *pid > 0)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Darwin census contains an invalid PID",
                    )
                })?;
            pids.insert(pid);
        }
        if pids.len() > maximum {
            return Err(io::Error::other(
                "Darwin process census exceeds its finite ceiling",
            ));
        }
    }
    Ok(pids)
}

#[cfg(test)]
mod tests;
