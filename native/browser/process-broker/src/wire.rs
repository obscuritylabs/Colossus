use crate::{HelperRole, SpawnArguments, arguments::invalid};
use std::io::{self, Read, Write};

/// Maximum complete frame, before allocating or reading a payload.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Fixed header size: magic, sequence, allocation nonce and payload length.
pub const HEADER_BYTES: usize = 36;
const MAGIC: &[u8; 8] = b"CBROK001";

/// Closed private operations. This protocol never carries descriptor numbers,
/// paths, environment maps, Mach rights, audit ports, or executable nominations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    /// Create and enroll one stopped direct child; do not resume it.
    Spawn(SpawnArguments),
    /// Resume after Chromium has registered its one-shot Mojo rendezvous.
    Resume(u64),
    /// Ask the real parent for the exact child's terminal wait status.
    Status(u64),
    /// Force termination and reap the exact retained direct child.
    Terminate(u64),
    /// Fence all subsequent creation and terminate every retained child.
    Close,
    /// Return an owned slot, stopped PID and optional raw wait status.
    Reply {
        /// Broker-owned immutable child slot.
        slot: u64,
        /// PID is presentation metadata, never creation authority.
        pid: u32,
        /// Raw wait status supplied by the actual parent after reaping.
        status: Option<i32>,
    },
    /// Categorical failure, without peer-controlled diagnostic bytes.
    Rejected,
    /// Trusted broker-to-keeper enrollment; never accepted on browser channel.
    Enroll {
        /// Broker-owned immutable child slot.
        slot: u64,
        /// Stopped direct-child PID for independent kernel identity capture.
        pid: u32,
        /// Front code hash copied from trusted native manifest metadata.
        cdhash: [u8; 20],
    },
    /// Remove a keeper obligation after the real parent has reaped its child.
    Reaped(u64),
    /// Native keeper query after creator admission is permanently fenced.
    ConfirmEmpty,
    /// Exact known-child ledger acknowledgement, including unresolved creation.
    Shutdown {
        /// Known direct children with a retained terminal wait status.
        reaped: u16,
        /// Attempts without positive exact-child cleanup evidence.
        unknown: u16,
        /// Independent enrolled-member roster positively acknowledged empty.
        keeper_empty: bool,
    },
}

/// Allocation-bound frame with a strictly increasing channel sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    /// Per-channel request sequence; zero is invalid.
    pub sequence: u64,
    /// Private native-composition allocation nonce; zero is invalid.
    pub nonce: [u8; 16],
    /// Closed validated operation.
    pub message: Message,
}

impl Frame {
    /// Read one bounded frame. A missing or truncated frame fails closed.
    pub fn read(input: &mut impl Read) -> io::Result<Self> {
        let mut header = [0; HEADER_BYTES];
        input.read_exact(&mut header)?;
        if &header[..8] != MAGIC {
            return Err(invalid("invalid broker frame magic"));
        }
        let sequence = u64::from_be_bytes(
            header[8..16]
                .try_into()
                .map_err(|_| invalid("invalid sequence"))?,
        );
        let nonce = header[16..32]
            .try_into()
            .map_err(|_| invalid("invalid nonce"))?;
        let length = u32::from_be_bytes(
            header[32..36]
                .try_into()
                .map_err(|_| invalid("invalid length"))?,
        ) as usize;
        if sequence == 0
            || nonce == [0; 16]
            || length == 0
            || length > MAX_FRAME_BYTES - HEADER_BYTES
        {
            return Err(invalid("invalid broker frame bounds"));
        }
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        let mut cursor = Cursor {
            bytes: &bytes,
            position: 0,
        };
        let operation = cursor.byte()?;
        let message = match operation {
            1 => {
                let role = HelperRole::decode(cursor.byte()?)?;
                let count = cursor.short()?;
                if count == 0 || count > 128 {
                    return Err(invalid("invalid argument count"));
                }
                let mut arguments = Vec::with_capacity(count.into());
                for _ in 0..count {
                    let size = cursor.short()? as usize;
                    if size == 0 || size > 4096 {
                        return Err(invalid("invalid argument length"));
                    }
                    arguments.push(
                        std::str::from_utf8(cursor.take(size)?)
                            .map_err(|_| invalid("invalid argument encoding"))?
                            .to_owned(),
                    );
                }
                Message::Spawn(SpawnArguments::new(role, arguments)?)
            }
            2 => Message::Resume(cursor.long()?),
            3 => Message::Status(cursor.long()?),
            4 => Message::Terminate(cursor.long()?),
            5 => Message::Close,
            6 => {
                let slot = cursor.long()?;
                let pid = cursor.word()?;
                let has_status = cursor.byte()?;
                let status = match has_status {
                    0 => None,
                    1 => Some(cursor.word()? as i32),
                    _ => return Err(invalid("invalid wait status marker")),
                };
                Message::Reply { slot, pid, status }
            }
            7 => Message::Rejected,
            8 => Message::Enroll {
                slot: cursor.long()?,
                pid: cursor.word()?,
                cdhash: cursor
                    .take(20)?
                    .try_into()
                    .map_err(|_| invalid("invalid cdhash"))?,
            },
            9 => Message::Reaped(cursor.long()?),
            10 => {
                let reaped = cursor.short()?;
                let unknown = cursor.short()?;
                let keeper_empty = match cursor.byte()? {
                    0 => false,
                    1 => true,
                    _ => return Err(invalid("invalid keeper-empty marker")),
                };
                if reaped > 64 || unknown > 64 {
                    return Err(invalid("invalid shutdown ledger bounds"));
                }
                Message::Shutdown {
                    reaped,
                    unknown,
                    keeper_empty,
                }
            }
            11 => Message::ConfirmEmpty,
            _ => return Err(invalid("unknown broker operation")),
        };
        if cursor.position != bytes.len() {
            return Err(invalid("trailing broker payload"));
        }
        Ok(Self {
            sequence,
            nonce,
            message,
        })
    }

    /// Write one complete bounded frame with no ancillary data mechanism.
    pub fn write(&self, output: &mut impl Write) -> io::Result<()> {
        if self.sequence == 0 || self.nonce == [0; 16] {
            return Err(invalid("invalid broker identity"));
        }
        let mut payload = Vec::new();
        match &self.message {
            Message::Spawn(arguments) => {
                payload.push(1);
                payload.push(arguments.role() as u8);
                payload.extend_from_slice(&(arguments.values().len() as u16).to_be_bytes());
                for value in arguments.values() {
                    payload.extend_from_slice(&(value.len() as u16).to_be_bytes());
                    payload.extend_from_slice(value.as_bytes());
                }
            }
            Message::Resume(slot)
            | Message::Status(slot)
            | Message::Terminate(slot)
            | Message::Reaped(slot) => {
                payload.push(match self.message {
                    Message::Resume(_) => 2,
                    Message::Status(_) => 3,
                    Message::Terminate(_) => 4,
                    _ => 9,
                });
                payload.extend_from_slice(&slot.to_be_bytes());
            }
            Message::Close => payload.push(5),
            Message::Reply { slot, pid, status } => {
                payload.push(6);
                payload.extend_from_slice(&slot.to_be_bytes());
                payload.extend_from_slice(&pid.to_be_bytes());
                payload.push(u8::from(status.is_some()));
                if let Some(status) = status {
                    payload.extend_from_slice(&status.to_be_bytes());
                }
            }
            Message::Rejected => payload.push(7),
            Message::ConfirmEmpty => payload.push(11),
            Message::Shutdown {
                reaped,
                unknown,
                keeper_empty,
            } => {
                if *reaped > 64 || *unknown > 64 {
                    return Err(invalid("invalid shutdown ledger bounds"));
                }
                payload.push(10);
                payload.extend_from_slice(&reaped.to_be_bytes());
                payload.extend_from_slice(&unknown.to_be_bytes());
                payload.push(u8::from(*keeper_empty));
            }
            Message::Enroll { slot, pid, cdhash } => {
                payload.push(8);
                payload.extend_from_slice(&slot.to_be_bytes());
                payload.extend_from_slice(&pid.to_be_bytes());
                payload.extend_from_slice(cdhash);
            }
        }
        if payload.len() > MAX_FRAME_BYTES - HEADER_BYTES {
            return Err(invalid("broker frame exceeds limit"));
        }
        output.write_all(MAGIC)?;
        output.write_all(&self.sequence.to_be_bytes())?;
        output.write_all(&self.nonce)?;
        output.write_all(&(payload.len() as u32).to_be_bytes())?;
        output.write_all(&payload)?;
        output.flush()
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, size: usize) -> io::Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(size)
            .ok_or_else(|| invalid("invalid cursor size"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid("truncated broker payload"))?;
        self.position = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| invalid("invalid short"))?,
        ))
    }
    fn word(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid word"))?,
        ))
    }
    fn long(&mut self) -> io::Result<u64> {
        let value = u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| invalid("invalid long"))?,
        );
        if value == 0 {
            return Err(invalid("zero broker slot"));
        }
        Ok(value)
    }
}
