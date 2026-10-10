//! Closed BSM session-record formats from the shipped SDK and native fixture.

use std::io;

use super::{MAX_RECORD_BYTES, invalid};

pub(crate) fn record_size(bytes: &[u8]) -> io::Result<Option<usize>> {
    if bytes.len() < 5 {
        return Ok(None);
    }
    if !matches!(bytes[0], 0x14 | 0x15 | 0x74 | 0x79) {
        return Err(invalid("unsupported BSM session header"));
    }
    let size = word(bytes, 1)? as usize;
    if !(68..=MAX_RECORD_BYTES).contains(&size) {
        return Err(invalid("BSM session record violates its finite bounds"));
    }
    Ok(Some(size))
}

pub(crate) fn session_record(bytes: &[u8], expected_asid: u32) -> io::Result<bool> {
    if record_size(bytes)? != Some(bytes.len()) || bytes.get(5) != Some(&11) {
        return Err(invalid("invalid BSM session record length or version"));
    }
    let event = short(bytes, 6)?;
    if !(44901..=44904).contains(&event) {
        return Err(invalid("unexpected non-session BSM event"));
    }
    let mut offset = header_size(bytes)?;
    let mut subject = false;
    let mut returned = false;
    let mut arguments = 0;
    while offset < bytes.len() {
        let tail = &bytes[offset..];
        let size = match tail[0] {
            0x2d | 0x71 => {
                arguments += 1;
                if arguments > 8 {
                    return Err(invalid("BSM session argument count exceeds its bound"));
                }
                argument_size(tail)?
            }
            0x24 | 0x75 | 0x7a | 0x7c => {
                if subject || word(tail, 25)? != expected_asid || word(tail, 21)? != 0 {
                    return Err(invalid(
                        "BSM session subject is duplicate, foreign or non-kernel",
                    ));
                }
                subject = true;
                subject_size(tail)?
            }
            0x27 | 0x72 => {
                let size = if tail[0] == 0x27 { 6 } else { 10 };
                if returned || tail.len() < size || tail[1..size].iter().any(|byte| *byte != 0) {
                    return Err(invalid("BSM session return token is invalid"));
                }
                returned = true;
                size
            }
            0x13 => {
                if tail.len() != 7
                    || short(tail, 1)? != 0xb105
                    || word(tail, 3)? as usize != bytes.len()
                    || !subject
                    || !returned
                {
                    return Err(invalid("BSM session trailer is invalid or incomplete"));
                }
                return Ok(event == 44903);
            }
            _ => return Err(invalid("unsupported BSM session token")),
        };
        if size > tail.len() {
            return Err(invalid("truncated BSM session token"));
        }
        offset += size;
    }
    Err(invalid("BSM session record has no complete trailer"))
}

fn header_size(bytes: &[u8]) -> io::Result<usize> {
    let size = match bytes[0] {
        0x14 => 18,
        0x74 => 26,
        0x15 => 22 + address_size(word(bytes, 10)?)?,
        0x79 => 30 + address_size(word(bytes, 10)?)?,
        _ => return Err(invalid("unsupported BSM session header")),
    };
    if size >= bytes.len() {
        return Err(invalid("truncated BSM session header"));
    }
    Ok(size)
}

fn subject_size(bytes: &[u8]) -> io::Result<usize> {
    match bytes[0] {
        0x24 => Ok(37),
        0x75 => Ok(41),
        0x7a => Ok(37 + address_size(word(bytes, 33)?)?),
        0x7c => Ok(41 + address_size(word(bytes, 37)?)?),
        _ => Err(invalid("unsupported BSM subject token")),
    }
}

fn argument_size(bytes: &[u8]) -> io::Result<usize> {
    let prefix = if bytes[0] == 0x2d { 8 } else { 12 };
    let length = short(bytes, prefix - 2)? as usize;
    if length == 0
        || length > 256
        || prefix + length > bytes.len()
        || bytes[prefix + length - 1] != 0
    {
        return Err(invalid("BSM session argument text violates its bound"));
    }
    Ok(prefix + length)
}

fn address_size(family: u32) -> io::Result<usize> {
    match family {
        4 => Ok(4),
        16 => Ok(16),
        _ => Err(invalid("unsupported BSM address family")),
    }
}

fn word(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("truncated BSM scalar"))?;
    Ok(u32::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| invalid("invalid BSM scalar"))?,
    ))
}

fn short(bytes: &[u8], offset: usize) -> io::Result<u16> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("truncated BSM scalar"))?;
    Ok(u16::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| invalid("invalid BSM scalar"))?,
    ))
}

#[cfg(test)]
mod tests;
