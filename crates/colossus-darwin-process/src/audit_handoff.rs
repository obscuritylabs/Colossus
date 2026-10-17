//! Observation-only cross-process retention from one closed current-only source.

mod device;
mod mach;
mod wire;

use std::{
    fmt,
    fs::File,
    io::{self, Read as _},
    os::fd::AsRawFd as _,
    time::{Duration, Instant},
};

use crate::{
    DarwinAuditPeer, DarwinAuditSessionEnd, DarwinProcessIdentity, RetainedDarwinAuditSession,
};
use mach::{Message, ReceivePort, SendRight};
use wire::{DeviceMetadata, Kind};

const MAX_WAIT: Duration = Duration::from_secs(30);
const MAX_RECORD_BYTES: usize = 0x7fff;
const MAX_RECORDS: usize = 64;

/// Closed current-session producer of one unread scoped audit-device handoff.
///
/// Only `current` constructs it. Its session reference comes exclusively from
/// `audit_session_self`; the readonly device comes exclusively from the fixed
/// `/dev/auditsessions` open. Neither raw ports nor nominated descriptors enter
/// this API. Local owners remain retained even if transfer acceptance is unknown.
pub struct DarwinCurrentAuditObservation {
    retained: RetainedDarwinAuditSession,
    file: File,
    metadata: DeviceMetadata,
    attempted: bool,
}

impl DarwinCurrentAuditObservation {
    /// Acquire the genuine current session and an unread scoped readonly device.
    pub fn current() -> io::Result<Self> {
        let retained = RetainedDarwinAuditSession::current()?;
        let file = crate::audit_events::device::open_current()?;
        let metadata = device::metadata(&file)?;
        device::validate_unread(&file, metadata)?;
        require_current(retained.owner_identity())?;
        Ok(Self {
            retained,
            file,
            metadata,
            attempted: false,
        })
    }

    /// Genuine kernel identity bound during current-session acquisition.
    pub fn owner_identity(&self) -> &DarwinProcessIdentity {
        self.retained.owner_identity()
    }

    /// Authenticate an independently enrolled outside-ASID keeper and transfer
    /// exactly two rights in one bounded Mach message, once for this producer.
    ///
    /// Trusted native bootstrap supplies the launchd MachService and nonzero
    /// nonce. The keeper's challenge trailer must match its complete enrolled
    /// token and manifest cdhash before transmission. Session `COPY_SEND` and
    /// device-fileport `MOVE_SEND` are atomic; acceptance is separately verified.
    /// Any failure consumes the attempt and preserves this local owner. The
    /// host must fence admission when acceptance remains unknown.
    pub fn send(
        &mut self,
        service_name: &str,
        nonce: [u8; 16],
        expected_keeper: &DarwinAuditPeer,
        timeout: Duration,
    ) -> io::Result<()> {
        let deadline = deadline(timeout)?;
        require_nonce(nonce)?;
        if self.attempted {
            return Err(invalid("audit producer handoff was already attempted"));
        }
        self.attempted = true;
        require_current(self.owner_identity())?;
        expected_keeper.verify()?;
        if expected_keeper.identity().audit_session_id() == self.retained.audit_session_id() {
            return Err(invalid(
                "audit keeper must remain outside the producer's ASID",
            ));
        }
        let remote = SendRight::lookup(service_name)?;
        let reply = ReceivePort::reply()?;
        Message::new(Kind::Hello, remote.name(), reply.name(), nonce, [0; 16]).send(deadline)?;
        let challenge_message = Message::receive(&reply, deadline)?;
        expected_keeper.authenticate(&wire::audit_trailer(challenge_message.bytes())?)?;
        let challenge = wire::parse(challenge_message.bytes(), Kind::Challenge)?;
        if challenge.nonce != nonce {
            return Err(invalid("audit keeper challenge nonce mismatched"));
        }
        require_current(self.owner_identity())?;
        expected_keeper.verify()?;
        device::validate_unread(&self.file, self.metadata)?;
        let fileport = SendRight::fileport(self.file.as_raw_fd())?;
        let mut transfer = Message::new(
            Kind::Transfer,
            remote.name(),
            reply.name(),
            nonce,
            challenge.challenge,
        );
        wire::add_transfer(
            transfer.bytes_mut(),
            self.retained.send_name(),
            fileport.into_message(),
            self.retained.audit_session_id(),
            self.metadata,
        );
        transfer.send(deadline)?;
        let accepted_message = Message::receive(&reply, deadline)?;
        expected_keeper.authenticate(&wire::audit_trailer(accepted_message.bytes())?)?;
        let accepted = wire::parse(accepted_message.bytes(), Kind::Accepted)?;
        wire::matches_challenge(&accepted, nonce, challenge.challenge)?;
        require_current(self.owner_identity())?;
        Ok(())
    }
}

/// One launchd-created receive endpoint enrolled for one exact source and nonce.
///
/// Capacity is one message and acceptance is single-use. Unknown, replayed or
/// malformed input permanently fences this receiver. Port rights from a foreign
/// sender are released generically; bounded rights from the exact enrolled
/// owner's partial/malformed transfer stay retained here while fenced, until the
/// independent host explicitly drops this owner. No received right is classified
/// as a kernel object, joined, or supplied to a process-spawn attribute.
pub struct DarwinAuditObservationReceiver {
    receive: Option<ReceivePort>,
    source: DarwinAuditPeer,
    keeper: DarwinProcessIdentity,
    nonce: [u8; 16],
    attempted: bool,
    retained_on_failure: Vec<SendRight>,
}

impl DarwinAuditObservationReceiver {
    /// Check in only a native launchd MachService configured for this keeper.
    ///
    /// The service name and nonce come from authenticated native bootstrap; an
    /// endpoint name never substitutes for genuine peer and signed-code checks.
    pub fn check_in(
        service_name: &str,
        expected_source: DarwinAuditPeer,
        nonce: [u8; 16],
    ) -> io::Result<Self> {
        require_nonce(nonce)?;
        expected_source.verify()?;
        let keeper = DarwinProcessIdentity::bind(std::process::id())?;
        if keeper.audit_session_id() == expected_source.identity().audit_session_id()
            || matches!(keeper.audit_session_id(), 0 | u32::MAX)
        {
            return Err(invalid(
                "audit receiver must have an independent outside ASID",
            ));
        }
        let receive = ReceivePort::check_in(service_name)?;
        require_current(&keeper)?;
        Ok(Self {
            receive: Some(receive),
            source: expected_source,
            keeper,
            nonce,
            attempted: false,
            retained_on_failure: Vec::new(),
        })
    }

    /// Accept one atomic current-source handoff with genuine kernel audit trailers.
    ///
    /// The total handshake deadline is positive and at most thirty seconds.
    /// A failed attempt cannot be retried or promote cleanup. Keep this receiver
    /// alive when failed matching-owner rights remain an unresolved obligation.
    pub fn receive(&mut self, timeout: Duration) -> io::Result<ReceivedDarwinAuditObservation> {
        let deadline = deadline(timeout)?;
        if self.attempted {
            return Err(invalid("audit receiver is fenced or already consumed"));
        }
        self.attempted = true;
        require_current(&self.keeper)?;
        let mut hello_message = self.receive_message(deadline)?;
        self.authenticate_or_retain(&mut hello_message)?;
        let hello = match wire::parse(hello_message.bytes(), Kind::Hello) {
            Ok(frame) => frame,
            Err(error) => {
                self.retain_matching_ports(&mut hello_message);
                return Err(error);
            }
        };
        if hello.nonce != self.nonce {
            self.retain_matching_ports(&mut hello_message);
            return Err(invalid("audit producer nonce mismatched"));
        }
        let reply = hello_message.take_reply()?;
        let challenge = mach::random_challenge();
        require_nonce(challenge)?;
        Message::new(
            Kind::Challenge,
            reply.into_message(),
            0,
            self.nonce,
            challenge,
        )
        .send(deadline)?;
        let mut transfer_message = self.receive_message(deadline)?;
        self.authenticate_or_retain(&mut transfer_message)?;
        let frame = match wire::parse(transfer_message.bytes(), Kind::Transfer) {
            Ok(frame) => frame,
            Err(error) => {
                self.retain_matching_ports(&mut transfer_message);
                return Err(error);
            }
        };
        // Retain both rights before any subsequent validation can fail. Matching
        // owner capabilities cannot disappear because a descriptor was partial.
        self.retain_matching_ports(&mut transfer_message);
        wire::matches_challenge(&frame, self.nonce, challenge)?;
        if frame.asid != self.source.identity().audit_session_id()
            || self.retained_on_failure.len() != 2
        {
            return Err(invalid(
                "audit transfer metadata is inconsistent with its exact owner",
            ));
        }
        let file = self.retained_on_failure[1].duplicate_file()?;
        device::validate_unread(&file, frame.metadata)?;
        self.source.verify()?;
        require_current(&self.keeper)?;
        let reply = transfer_message.take_reply()?;
        Message::new(
            Kind::Accepted,
            reply.into_message(),
            0,
            self.nonce,
            challenge,
        )
        .send(deadline)?;
        require_current(&self.keeper)?;
        let receive = self
            .receive
            .take()
            .ok_or_else(|| invalid("audit receiver endpoint was already consumed"))?;
        let rights = std::mem::take(&mut self.retained_on_failure);
        Ok(ReceivedDarwinAuditObservation {
            source: self.source.clone(),
            keeper: self.keeper.clone(),
            _rights: rights,
            receive,
            retained_replay: Vec::new(),
            file,
            metadata: frame.metadata,
            pending: Vec::new(),
            records: 0,
            fenced: false,
            ended: false,
        })
    }

    fn authenticate_or_retain(&mut self, message: &mut Message) -> io::Result<()> {
        let token = wire::audit_trailer(message.bytes())?;
        if self.source.matches(&token) {
            if let Err(error) = self.source.authenticate(&token) {
                self.retain_matching_ports(message);
                return Err(error);
            }
            Ok(())
        } else {
            Err(invalid(
                "audit message came from a foreign process identity",
            ))
        }
    }

    fn receive_message(&mut self, deadline: Instant) -> io::Result<Message> {
        let receive = self
            .receive
            .as_ref()
            .ok_or_else(|| invalid("audit receiver endpoint was already consumed"))?;
        match Message::receive(receive, deadline) {
            Ok(message) => Ok(message),
            Err(mut failure) => {
                if let Some(message) = failure.partial.as_mut()
                    && wire::audit_trailer(message.bytes())
                        .is_ok_and(|token| self.source.matches(&token))
                {
                    self.retain_matching_ports(message);
                }
                Err(failure.error)
            }
        }
    }

    fn retain_matching_ports(&mut self, message: &mut Message) {
        if self.retained_on_failure.is_empty() {
            self.retained_on_failure = message.take_ports();
        }
    }
}

/// Opaque received observation capability retaining the source ASID and device.
///
/// This is deliberately distinct from `RetainedDarwinAuditSession`: only the
/// authenticated current-only handoff constructs it. It has no raw port/FD
/// access, constructor, conversion, serialization, joining or launch operation.
/// A matching kernel END proves only a zero-process event for this retained ASID;
/// admission fencing, escaped obligations, native CEF shutdown, exact reaping and
/// physical cleanup remain independent controller responsibilities.
pub struct ReceivedDarwinAuditObservation {
    source: DarwinAuditPeer,
    keeper: DarwinProcessIdentity,
    _rights: Vec<SendRight>,
    receive: ReceivePort,
    retained_replay: Vec<SendRight>,
    file: File,
    metadata: DeviceMetadata,
    pending: Vec<u8>,
    records: usize,
    fenced: bool,
    ended: bool,
}

impl ReceivedDarwinAuditObservation {
    /// ASID retained by the received reference; metadata grants no authority.
    pub fn audit_session_id(&self) -> u32 {
        self.source.identity().audit_session_id()
    }

    /// Exact genuine producer enrolled before its accepted atomic handoff.
    pub fn owner_identity(&self) -> &DarwinProcessIdentity {
        self.source.identity()
    }

    /// Recheck source authentication for admission and lifecycle supervision.
    ///
    /// Source exit must fence admission and be reconciled by the controller. It
    /// does not discard an intact independent observer: its kernel END remains
    /// useful after a crash. Exec/session changes leave separate escaped-process
    /// obligations, which an END for the old ASID cannot discharge.
    pub fn check_source(&self) -> io::Result<()> {
        self.source.verify()
    }

    /// Permanently invalidate event observability while retaining both rights.
    pub fn fence(&mut self) {
        self.fenced = true;
    }

    /// Observe one strict matching kernel session END within a bounded wait.
    ///
    /// Device drops/loss, unknown records, metadata changes or keeper identity
    /// transitions permanently fence this capability. Source process death is
    /// handled by its independent controller and does not destroy cleanup
    /// evidence. Timeouts preserve bounded partial framing for another wait.
    pub fn wait_for_end(&mut self, timeout: Duration) -> io::Result<DarwinAuditSessionEnd> {
        let deadline = deadline(timeout)?;
        if self.fenced || self.ended {
            return Err(invalid("received audit observation is fenced or consumed"));
        }
        let result = self.observe_until(deadline);
        if result
            .as_ref()
            .is_err_and(|error| error.kind() != io::ErrorKind::TimedOut)
        {
            self.fenced = true;
        }
        result
    }

    fn observe_until(&mut self, deadline: Instant) -> io::Result<DarwinAuditSessionEnd> {
        loop {
            require_current(&self.keeper)?;
            self.require_no_replay()?;
            device::validate_continuing(&self.file, self.metadata)?;
            if let Some(size) = crate::audit_events::parser::record_size(&self.pending)?
                && self.pending.len() >= size
            {
                self.records += 1;
                if self.records > MAX_RECORDS {
                    return Err(invalid(
                        "received audit stream exceeded its finite record ceiling",
                    ));
                }
                let end = crate::audit_events::parser::session_record(
                    &self.pending[..size],
                    self.audit_session_id(),
                )?;
                self.pending.drain(..size);
                if end {
                    require_current(&self.keeper)?;
                    self.require_no_replay()?;
                    device::validate_continuing(&self.file, self.metadata)?;
                    self.ended = true;
                    return Ok(DarwinAuditSessionEnd::observed(
                        self.source.identity().clone(),
                    ));
                }
                continue;
            }
            crate::audit_events::device::select_readable(self.file.as_raw_fd(), deadline)?;
            let mut buffer = [0; MAX_RECORD_BYTES];
            match self.file.read(&mut buffer) {
                Ok(0) => return Err(invalid("received audit device ended without kernel END")),
                Ok(count) => {
                    if self.pending.len() + count > 2 * MAX_RECORD_BYTES {
                        return Err(invalid("received audit framing exceeded its finite buffer"));
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

    fn require_no_replay(&mut self) -> io::Result<()> {
        let result = Message::try_receive(&self.receive);
        let message = match result {
            Ok(message) => message,
            Err(mut failure) => {
                if let Some(message) = failure.partial.as_mut()
                    && wire::audit_trailer(message.bytes())
                        .is_ok_and(|token| self.source.matches(&token))
                {
                    self.retained_replay = message.take_ports();
                }
                return Err(failure.error);
            }
        };
        if let Some(mut message) = message {
            if wire::audit_trailer(message.bytes()).is_ok_and(|token| self.source.matches(&token)) {
                self.retained_replay = message.take_ports();
            }
            return Err(invalid(
                "received audit endpoint has replayed or unexpected input",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for ReceivedDarwinAuditObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceivedDarwinAuditObservation")
            .field("source", &self.source)
            .field("keeper", &self.keeper)
            .field("fenced", &self.fenced)
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

fn require_current(expected: &DarwinProcessIdentity) -> io::Result<()> {
    if &DarwinProcessIdentity::bind(std::process::id())? != expected {
        return Err(invalid(
            "Darwin audit owner changed incarnation, exec or session",
        ));
    }
    Ok(())
}

fn require_nonce(nonce: [u8; 16]) -> io::Result<()> {
    if nonce == [0; 16] {
        Err(invalid(
            "audit handoff requires a nonzero native bootstrap nonce",
        ))
    } else {
        Ok(())
    }
}

fn deadline(timeout: Duration) -> io::Result<Instant> {
    if timeout.is_zero() || timeout > MAX_WAIT {
        return Err(invalid(
            "audit deadline must be positive and at most thirty seconds",
        ));
    }
    Ok(Instant::now() + timeout)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
