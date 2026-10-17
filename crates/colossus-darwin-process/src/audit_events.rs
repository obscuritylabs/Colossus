//! Scoped kernel session events, with bounded decoding and retained identity.

pub(crate) mod device;
pub(crate) mod parser;

use std::{
    fs::File,
    io::{self, Read as _},
    os::fd::{AsRawFd as _, OwnedFd},
    time::{Duration, Instant},
};

use crate::{DarwinProcessIdentity, RetainedDarwinAuditSession};

const MAX_RECORD_BYTES: usize = 0x7fff;
const MAX_RECORDS: usize = 64;
const MAX_WAIT: Duration = Duration::from_secs(30);

/// One readonly kernel event stream bound at open to the current retained ASID.
///
/// This constructor never requests global audit records or changes audit policy.
/// The borrow retains the local Mach session reference. Exporting the descriptor
/// does not transfer that reference; an independent native keeper and protected
/// receiver binding remain mandatory before outside-process lifetime acceptance.
pub struct DarwinAuditSessionEvents<'a> {
    retained: &'a RetainedDarwinAuditSession,
    file: File,
    pending: Vec<u8>,
    records: usize,
    consumed: bool,
    fenced: bool,
    ended: bool,
}

impl<'a> DarwinAuditSessionEvents<'a> {
    /// Open only the calling owner's assigned session's readonly kernel device.
    ///
    /// Current identity must still equal the retained owner before and after
    /// opening. Unknown device support, drops, or changed identity fails closed.
    pub fn current(retained: &'a RetainedDarwinAuditSession) -> io::Result<Self> {
        let owner = DarwinProcessIdentity::bind(std::process::id())?;
        if &owner != retained.owner_identity() {
            return Err(invalid("audit device requires its retained current owner"));
        }
        let file = device::open_current()?;
        if owner != DarwinProcessIdentity::bind(std::process::id())? {
            return Err(invalid(
                "audit owner changed while opening the scoped device",
            ));
        }
        Ok(Self {
            retained,
            file,
            pending: Vec::new(),
            records: 0,
            consumed: false,
            fenced: false,
            ended: false,
        })
    }

    /// Wait at most thirty seconds for an actual matching kernel SESSION_END.
    ///
    /// At most 64 bounded records are consumed for this stream's entire lifetime.
    /// A timeout retains partial framing for retry; drops, EOF, invalid framing,
    /// foreign ASID or unsupported formats permanently fence the observation.
    pub fn wait_for_end(&mut self, timeout: Duration) -> io::Result<DarwinAuditSessionEnd> {
        if timeout.is_zero() || timeout > MAX_WAIT {
            return Err(invalid(
                "audit wait must have a positive bound of at most thirty seconds",
            ));
        }
        if self.fenced || self.ended {
            return Err(invalid("audit observation is fenced or already consumed"));
        }
        let result = self.observe_until(Instant::now() + timeout);
        if let Err(error) = &result
            && error.kind() != io::ErrorKind::TimedOut
        {
            self.fenced = true;
        }
        result
    }

    /// Consume an unread device owner for separately authenticated native IPC.
    ///
    /// Partial records never silently cross this handoff. The recipient must
    /// verify descriptor provenance and bind the genuine producer; this method
    /// neither constructs a receiver nor transfers the retained Mach reference.
    pub fn into_owned_fd(self) -> io::Result<OwnedFd> {
        if self.consumed || self.fenced || self.ended {
            return Err(invalid(
                "audit descriptor handoff requires an unread unfenced stream",
            ));
        }
        device::require_no_drops(self.file.as_raw_fd())?;
        Ok(self.file.into())
    }

    fn observe_until(&mut self, deadline: Instant) -> io::Result<DarwinAuditSessionEnd> {
        loop {
            device::require_no_drops(self.file.as_raw_fd())?;
            if let Some(size) = parser::record_size(&self.pending)?
                && self.pending.len() >= size
            {
                self.records += 1;
                if self.records > MAX_RECORDS {
                    return Err(invalid("audit stream exceeded its finite record ceiling"));
                }
                let end = parser::session_record(
                    &self.pending[..size],
                    self.retained.audit_session_id(),
                )?;
                self.pending.drain(..size);
                if end {
                    device::require_no_drops(self.file.as_raw_fd())?;
                    self.ended = true;
                    return Ok(DarwinAuditSessionEnd {
                        owner: self.retained.owner_identity().clone(),
                        observed_at: Instant::now(),
                    });
                }
                continue;
            }
            device::select_readable(self.file.as_raw_fd(), deadline)?;
            let mut buffer = [0_u8; MAX_RECORD_BYTES];
            match self.file.read(&mut buffer) {
                Ok(0) => {
                    return Err(invalid(
                        "audit device ended without a kernel session receipt",
                    ));
                }
                Ok(count) => {
                    self.consumed = true;
                    if self.pending.len() + count > 2 * MAX_RECORD_BYTES {
                        return Err(invalid("audit stream exceeded its finite framing buffer"));
                    }
                    self.pending.extend_from_slice(&buffer[..count]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

/// Actual matching kernel zero-process session event, distinct from CEF teardown.
///
/// The constructor is private and requires the scoped kernel stream. END alone
/// cannot prove immutable membership, admission fencing, native shutdown, reaping,
/// or retained identity after the keeper process is lost.
#[derive(Debug)]
pub struct DarwinAuditSessionEnd {
    owner: DarwinProcessIdentity,
    observed_at: Instant,
}

impl DarwinAuditSessionEnd {
    pub(crate) fn observed(owner: DarwinProcessIdentity) -> Self {
        Self {
            owner,
            observed_at: Instant::now(),
        }
    }

    /// Exact retained session whose kernel process counter reached zero.
    pub fn audit_session_id(&self) -> u32 {
        self.owner.audit_session_id()
    }

    /// Original genuine producer identity bound when its scoped device was opened.
    pub fn owner_identity(&self) -> &DarwinProcessIdentity {
        &self.owner
    }

    /// Monotonic observation time; the event is not continuing liveness authority.
    pub fn observed_at(&self) -> Instant {
        self.observed_at
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
