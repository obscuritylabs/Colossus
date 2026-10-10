//! Fixed Mach framing. Parsing here supplies shape checks, never peer authority.

use std::io;

use super::invalid;

pub(super) const MESSAGE_SIZE: usize = 128;
pub(super) const BUFFER_SIZE: usize = 512;
pub(super) const MESSAGE_ID: u32 = 0x434f_4153;
pub(super) const COMPLEX: u32 = 0x8000_0000;
pub(super) const MOVE_SEND: u8 = 17;
pub(super) const COPY_SEND: u8 = 19;
pub(super) const MAKE_SEND_ONCE: u32 = 21;
pub(super) const MOVE_SEND_ONCE: u32 = 18;
const TRAILER_SIZE: usize = 52;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Hello = 1,
    Challenge = 2,
    Transfer = 3,
    Accepted = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DeviceMetadata {
    pub(super) dev: u64,
    pub(super) ino: u64,
    pub(super) rdev: u64,
    pub(super) mode: u32,
    pub(super) flags: u32,
}

pub(super) struct Frame {
    pub(super) nonce: [u8; 16],
    pub(super) challenge: [u8; 16],
    pub(super) asid: u32,
    pub(super) metadata: DeviceMetadata,
}

pub(super) fn initialize(
    bytes: &mut [u8; BUFFER_SIZE],
    kind: Kind,
    remote: u32,
    reply: u32,
    nonce: [u8; 16],
    challenge: [u8; 16],
) {
    bytes.fill(0);
    put_word(
        bytes,
        0,
        if matches!(kind, Kind::Challenge | Kind::Accepted) {
            MOVE_SEND_ONCE
        } else {
            u32::from(COPY_SEND) | if reply != 0 { MAKE_SEND_ONCE << 8 } else { 0 }
        },
    );
    put_word(bytes, 4, MESSAGE_SIZE as u32);
    put_word(bytes, 8, remote);
    put_word(bytes, 12, reply);
    put_word(bytes, 20, MESSAGE_ID);
    put_word(bytes, 52, 1);
    put_word(bytes, 56, kind as u32);
    bytes[60..76].copy_from_slice(&nonce);
    bytes[76..92].copy_from_slice(&challenge);
}

pub(super) fn add_transfer(
    bytes: &mut [u8; BUFFER_SIZE],
    session: u32,
    fileport: u32,
    asid: u32,
    metadata: DeviceMetadata,
) {
    let bits = word(bytes, 0).unwrap_or(0) | COMPLEX;
    put_word(bytes, 0, bits);
    put_word(bytes, 24, 2);
    for (offset, name, disposition) in [(28, session, COPY_SEND), (40, fileport, MOVE_SEND)] {
        put_word(bytes, offset, name);
        bytes[offset + 10] = disposition;
    }
    put_word(bytes, 92, asid);
    put_long(bytes, 96, metadata.dev);
    put_long(bytes, 104, metadata.ino);
    put_long(bytes, 112, metadata.rdev);
    put_word(bytes, 120, metadata.mode);
    put_word(bytes, 124, metadata.flags);
}

pub(super) fn audit_trailer(bytes: &[u8]) -> io::Result<[u32; 8]> {
    let size = word(bytes, 4)? as usize;
    if !(24..=BUFFER_SIZE - TRAILER_SIZE).contains(&size) {
        return Err(invalid("Mach audit handoff has invalid message bounds"));
    }
    let start = (size + 3) & !3;
    if word(bytes, start)? != 0 || word(bytes, start + 4)? as usize != TRAILER_SIZE {
        return Err(invalid(
            "Mach audit handoff lacks its exact kernel audit trailer",
        ));
    }
    let mut token = [0; 8];
    for (index, target) in token.iter_mut().enumerate() {
        *target = word(bytes, start + 20 + index * 4)?;
    }
    Ok(token)
}

pub(super) fn parse(bytes: &[u8], kind: Kind) -> io::Result<Frame> {
    if word(bytes, 4)? as usize != MESSAGE_SIZE
        || word(bytes, 20)? != MESSAGE_ID
        || word(bytes, 52)? != 1
        || word(bytes, 56)? != kind as u32
        || word(bytes, 16)? != 0
    {
        return Err(invalid("Mach audit handoff has unknown framing"));
    }
    let bits = word(bytes, 0)?;
    let complex = bits & COMPLEX != 0;
    let expects_reply = matches!(kind, Kind::Hello | Kind::Transfer);
    if bits & 0xff != if expects_reply { MOVE_SEND_ONCE } else { 0 }
        || (expects_reply && matches!(word(bytes, 8)?, 0 | u32::MAX))
        || (!expects_reply && word(bytes, 8)? != 0)
        || matches!(word(bytes, 12)?, 0 | u32::MAX)
    {
        return Err(invalid("audit handoff has unexpected reply-port rights"));
    }
    // Receive transforms descriptor dispositions to the received right type.
    if complex != matches!(kind, Kind::Transfer) || bits & !(COMPLEX | 0xffff) != 0 {
        return Err(invalid("Mach audit handoff has unsupported header rights"));
    }
    if complex {
        if word(bytes, 24)? != 2 {
            return Err(invalid("audit handoff requires exactly two atomic rights"));
        }
        for offset in [28, 40] {
            if word(bytes, offset)? == 0
                || word(bytes, offset)? == u32::MAX
                || word(bytes, offset + 4)? != 0
                || bytes.get(offset + 8..offset + 10) != Some(&[0, 0])
                || bytes.get(offset + 10) != Some(&MOVE_SEND)
                || bytes.get(offset + 11) != Some(&0)
            {
                return Err(invalid("audit handoff has an unsupported port descriptor"));
            }
        }
        if word(bytes, 28)? == word(bytes, 40)? {
            return Err(invalid("audit handoff rights alias unexpectedly"));
        }
    } else if bytes
        .get(24..52)
        .is_none_or(|range| range.iter().any(|byte| *byte != 0))
    {
        return Err(invalid("audit handshake contains unexpected descriptors"));
    }
    let mut nonce = [0; 16];
    let mut challenge = [0; 16];
    nonce.copy_from_slice(
        bytes
            .get(60..76)
            .ok_or_else(|| invalid("truncated audit nonce"))?,
    );
    challenge.copy_from_slice(
        bytes
            .get(76..92)
            .ok_or_else(|| invalid("truncated audit challenge"))?,
    );
    if nonce == [0; 16]
        || (kind != Kind::Hello && challenge == [0; 16])
        || (kind == Kind::Hello && challenge != [0; 16])
    {
        return Err(invalid(
            "audit handoff has an absent or unexpected challenge",
        ));
    }
    if kind != Kind::Transfer
        && bytes
            .get(92..MESSAGE_SIZE)
            .is_none_or(|range| range.iter().any(|byte| *byte != 0))
    {
        return Err(invalid(
            "audit handshake contains unexpected device metadata",
        ));
    }
    Ok(Frame {
        nonce,
        challenge,
        asid: word(bytes, 92)?,
        metadata: DeviceMetadata {
            dev: long(bytes, 96)?,
            ino: long(bytes, 104)?,
            rdev: long(bytes, 112)?,
            mode: word(bytes, 120)?,
            flags: word(bytes, 124)?,
        },
    })
}

pub(super) fn matches_challenge(
    frame: &Frame,
    nonce: [u8; 16],
    challenge: [u8; 16],
) -> io::Result<()> {
    if frame.nonce != nonce || frame.challenge != challenge {
        return Err(invalid(
            "audit handoff has a replayed or mismatched challenge",
        ));
    }
    Ok(())
}

pub(super) fn word(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_ne_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated Mach audit handoff"))?
            .try_into()
            .map_err(|_| invalid("invalid Mach audit word"))?,
    ))
}

fn long(bytes: &[u8], offset: usize) -> io::Result<u64> {
    Ok(u64::from_ne_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| invalid("truncated Mach audit metadata"))?
            .try_into()
            .map_err(|_| invalid("invalid Mach audit metadata"))?,
    ))
}

pub(super) fn put_word(bytes: &mut [u8], offset: usize, word: u32) {
    bytes[offset..offset + 4].copy_from_slice(&word.to_ne_bytes());
}

fn put_long(bytes: &mut [u8], offset: usize, word: u64) {
    bytes[offset..offset + 8].copy_from_slice(&word.to_ne_bytes());
}

#[cfg(test)]
mod tests;
