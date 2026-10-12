use crate::{Frame, Message, arguments::invalid};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinProcessExitObserver, DarwinProcessIdentity, DarwinProcessSignal,
};
use std::{
    collections::BTreeMap,
    io,
    net::Shutdown,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

struct Member {
    identity: DarwinProcessIdentity,
    observer: Option<DarwinProcessExitObserver>,
}

/// Private connected keeper endpoint supplied by trusted native composition.
///
/// This is not a discoverable service: composition must create the connected
/// socket pair, authenticate both signed owners, and keep it out of browser
/// inheritance. The allocation nonce comes from that same composition.
pub struct KeeperClient {
    stream: UnixStream,
    nonce: [u8; 16],
    sequence: u64,
    broken: bool,
}
impl KeeperClient {
    /// Adopt the already authenticated keeper-only channel with bounded I/O.
    pub fn new(stream: UnixStream, nonce: [u8; 16]) -> io::Result<Self> {
        if nonce == [0; 16] {
            return Err(invalid("zero keeper allocation"));
        }
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(Self {
            stream,
            nonce,
            sequence: 0,
            broken: false,
        })
    }
    pub(crate) fn enroll(&mut self, slot: u64, pid: u32, cdhash: [u8; 20]) -> io::Result<()> {
        self.exchange(Message::Enroll { slot, pid, cdhash }, slot, pid)
    }
    pub(crate) fn reaped(&mut self, slot: u64) -> io::Result<()> {
        self.exchange(Message::Reaped(slot), slot, 0)
    }
    pub(crate) fn confirm_empty(&mut self) -> io::Result<bool> {
        match self.request(Message::ConfirmEmpty)? {
            Message::Shutdown {
                reaped: 0,
                unknown: 0,
                keeper_empty,
            } => Ok(keeper_empty),
            _ => self.reject("keeper returned invalid empty acknowledgement"),
        }
    }
    fn exchange(&mut self, message: Message, slot: u64, pid: u32) -> io::Result<()> {
        if self.request(message)?
            != (Message::Reply {
                slot,
                pid,
                status: None,
            })
        {
            return self.reject("keeper exact operation was not acknowledged");
        }
        Ok(())
    }

    fn request(&mut self, message: Message) -> io::Result<Message> {
        if self.broken {
            return Err(invalid("keeper channel is permanently fenced"));
        }
        let result = (|| {
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or_else(|| invalid("keeper sequence exhausted"))?;
            Frame {
                sequence: self.sequence,
                nonce: self.nonce,
                message,
            }
            .write(&mut self.stream)?;
            let response = Frame::read(&mut self.stream)?;
            if response.sequence != self.sequence || response.nonce != self.nonce {
                return Err(invalid(
                    "keeper acknowledgement changed sequence or allocation",
                ));
            }
            Ok(response.message)
        })();
        if result.is_err() {
            self.broken = true;
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        result
    }

    fn reject<T>(&mut self, reason: &'static str) -> io::Result<T> {
        self.broken = true;
        let _ = self.stream.shutdown(Shutdown::Both);
        Err(invalid(reason))
    }
}

/// Independent keeper of exact, immutable process incarnations.
///
/// Browser processes must have final deny-fork/exec before loading CEF. The
/// keeper never authorizes creation, receives Mach rights, joins an audit
/// session, follows a process group, or performs a PID census. Connection loss
/// kills and observes exit of every already enrolled incarnation. The creator-to-enrollment crash
/// window is deliberately not represented as acknowledged cleanup.
pub struct KeeperServer {
    nonce: [u8; 16],
    sequence: u64,
    members: BTreeMap<u64, Member>,
}
impl KeeperServer {
    /// Create a bounded keeper for an authenticated native-only socket pair.
    pub fn new(nonce: [u8; 16]) -> io::Result<Self> {
        if nonce == [0; 16] {
            return Err(invalid("zero keeper allocation"));
        }
        Ok(Self {
            nonce,
            sequence: 0,
            members: BTreeMap::new(),
        })
    }
    /// Serve until broker EOF, framing error or failed native enrollment.
    ///
    /// An error fences admission, requests exact-member termination and waits
    /// within one five-second deadline for the retained kernel exit observers.
    /// Missing events preserve obligations and fail cleanup. Even known-roster
    /// exit acknowledgement is not whole-domain or actual-parent reap evidence.
    pub fn serve(&mut self, mut stream: UnixStream, lifetime: UnixStream) -> io::Result<()> {
        // Retain the unique native-only lease endpoint until serving ends.
        let _lifetime = lifetime;
        let result: io::Result<()> = (|| loop {
            let frame = Frame::read(&mut stream)?;
            if frame.nonce != self.nonce || self.sequence.checked_add(1) != Some(frame.sequence) {
                return Err(invalid("keeper replay or allocation mismatch"));
            }
            self.sequence = frame.sequence;
            if frame.message == Message::ConfirmEmpty {
                Frame {
                    sequence: self.sequence,
                    nonce: self.nonce,
                    message: Message::Shutdown {
                        reaped: 0,
                        unknown: 0,
                        keeper_empty: self.members.is_empty(),
                    },
                }
                .write(&mut stream)?;
                continue;
            }
            let (slot, pid) = match frame.message {
                Message::Enroll { slot, pid, cdhash } => {
                    if self.members.len() >= 64 || self.members.contains_key(&slot) || pid == 0 {
                        return Err(invalid("keeper membership limit or duplicate"));
                    }
                    let identity = DarwinProcessIdentity::bind(pid)?;
                    DarwinAuditPeer::enroll(&identity, cdhash)?;
                    // Retain the genuine incarnation before observer setup. A
                    // failed registration remains a cleanup obligation, never
                    // an absent-member or successful enrollment receipt.
                    self.members.insert(
                        slot,
                        Member {
                            identity: identity.clone(),
                            observer: None,
                        },
                    );
                    let observer = DarwinProcessExitObserver::register(&identity)?;
                    self.members
                        .get_mut(&slot)
                        .ok_or_else(|| invalid("keeper enrollment disappeared"))?
                        .observer = Some(observer);
                    (slot, pid)
                }
                Message::Reaped(slot) => {
                    let member = self
                        .members
                        .get_mut(&slot)
                        .ok_or_else(|| invalid("keeper has no such obligation"))?;
                    let observer = member
                        .observer
                        .as_mut()
                        .ok_or_else(|| invalid("keeper exit observer was not admitted"))?;
                    if observer.wait(Duration::ZERO)?.is_none() {
                        return Err(invalid("keeper has not observed enrolled process exit"));
                    }
                    self.members.remove(&slot);
                    (slot, 0)
                }
                _ => return Err(invalid("browser operation on keeper-only channel")),
            };
            Frame {
                sequence: self.sequence,
                nonce: self.nonce,
                message: Message::Reply {
                    slot,
                    pid,
                    status: None,
                },
            }
            .write(&mut stream)?;
        })();
        let cleanup = self.kill_enrolled();
        result.and(cleanup)
    }
    /// Terminate enrolled incarnations and acknowledge their kernel exit events.
    ///
    /// The single deadline bounds the whole known roster. Unobserved entries
    /// remain retained after failure; a signal or an ESRCH alone is not evidence
    /// of exit. This does not reap non-children or resolve unknown creation.
    pub fn kill_enrolled(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut failure = None;
        for member in self.members.values() {
            if let Err(error) = member.identity.signal(DarwinProcessSignal::Kill)
                && error.raw_os_error() != Some(3)
            {
                failure.get_or_insert(error);
            }
        }
        let mut exited = Vec::new();
        for (slot, member) in &mut self.members {
            let observed = match &mut member.observer {
                Some(observer) => observer.wait(deadline.saturating_duration_since(Instant::now())),
                None => Err(invalid("keeper exit observer was not admitted")),
            };
            match observed {
                Ok(Some(_)) => exited.push(*slot),
                Ok(None) => {
                    failure.get_or_insert_with(|| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "keeper enrolled process exit was not observed",
                        )
                    });
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        for slot in exited {
            self.members.remove(&slot);
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for KeeperServer {
    fn drop(&mut self) {
        let _ = self.kill_enrolled();
    }
}

#[cfg(test)]
#[path = "keeper_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "keeper_client_tests.rs"]
mod client_tests;
