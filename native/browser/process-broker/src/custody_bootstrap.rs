//! Fixed native front handshake; no browser/control-path nomination.
use crate::arguments::invalid;
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

pub(crate) fn write(
    stream: &mut UnixStream,
    nonce: [u8; 16],
    pid: u32,
    role: u8,
    body: [u8; 32],
    policy: [u8; 32],
) -> io::Result<()> {
    if nonce == [0; 16] || pid <= 1 || role > 4 || body == [0; 32] || policy == [0; 32] {
        return Err(invalid("invalid fixed custody bootstrap"));
    }
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"CBOOT003")?;
    stream.write_all(&nonce)?;
    stream.write_all(&pid.to_be_bytes())?;
    stream.write_all(&[role])?;
    stream.write_all(&body)?;
    stream.write_all(&policy)?;
    stream.write_all(&[0, 0, 0])?; // Normal body admission, no operator extension cookie.
    stream.flush()
}
pub(crate) fn admit(
    stream: &mut UnixStream,
    nonce: [u8; 16],
    role: u8,
    body: [u8; 32],
    policy: [u8; 32],
) -> io::Result<()> {
    let mut receipt = [0; 92];
    stream.read_exact(&mut receipt)?;
    validate(&receipt, nonce, role, body, policy, 0)?;
    stream.write_all(b"CCAK0001")?;
    stream.write_all(&nonce)?;
    stream.write_all(&[role, 0])?;
    stream.flush()?;
    stream.set_read_timeout(None)
}
fn validate(
    receipt: &[u8; 92],
    nonce: [u8; 16],
    role: u8,
    body: [u8; 32],
    policy: [u8; 32],
    mode: u8,
) -> io::Result<()> {
    let flags = u16::from_be_bytes([receipt[89], receipt[90]]);
    if &receipt[..8] != b"CCUS0001"
        || receipt[8..24] != nonce
        || receipt[24] != role
        || receipt[25..57] != policy
        || receipt[57..89] != body
        || receipt[91] != mode
        || flags != if mode == 0 { 0x00ff } else { 0x01ff }
    {
        return Err(invalid("native custody denial receipt is not exact"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn receipt() -> [u8; 92] {
        let mut r = [0; 92];
        r[..8].copy_from_slice(b"CCUS0001");
        r[8..24].fill(1);
        r[24] = 2;
        r[25..57].fill(3);
        r[57..89].fill(4);
        r[90] = 255;
        r
    }
    #[test]
    fn every_binding_and_denial_is_required_before_body_ack() {
        let valid = receipt();
        assert!(validate(&valid, [1; 16], 2, [4; 32], [3; 32], 0).is_ok());
        for offset in [0, 8, 24, 25, 57, 89, 90, 91] {
            let mut r = valid;
            r[offset] ^= 1;
            assert!(validate(&r, [1; 16], 2, [4; 32], [3; 32], 0).is_err());
        }
        let mut probe = valid;
        probe[89] = 1;
        probe[91] = 1;
        assert!(validate(&probe, [1; 16], 2, [4; 32], [3; 32], 1).is_ok());
        assert!(validate(&valid, [1; 16], 2, [4; 32], [3; 32], 1).is_err());
    }
    #[test]
    fn malformed_receipt_never_emits_acknowledgement() {
        let (mut server, mut peer) = UnixStream::pair().unwrap();
        let mut r = receipt();
        r[90] = 0;
        peer.write_all(&r).unwrap();
        assert!(admit(&mut server, [1; 16], 2, [4; 32], [3; 32]).is_err());
        peer.set_nonblocking(true).unwrap();
        let mut byte = [0];
        assert_eq!(
            peer.read(&mut byte).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
}
