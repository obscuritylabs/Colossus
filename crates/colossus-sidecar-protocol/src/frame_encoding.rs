use crate::{MAX_FRAME_BYTES, ProtocolError};
use serde::Serialize;
use std::io::{self, Write};
use zeroize::Zeroizing;

struct FrameBuffer(Zeroizing<Vec<u8>>);

impl Write for FrameBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > (MAX_FRAME_BYTES + 4).saturating_sub(self.0.len()) {
            return Err(io::Error::other("sidecar frame exceeds its bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>, ProtocolError> {
    // Reserve the full bound before writing secrets: growing a Vec would leave
    // prior allocations containing plaintext outside the zeroizing owner.
    let mut bytes = Vec::with_capacity(MAX_FRAME_BYTES + 4);
    bytes.extend_from_slice(&[0; 4]);
    let mut frame = FrameBuffer(Zeroizing::new(bytes));
    serde_json::to_writer(&mut frame, value).map_err(|_| ProtocolError::InvalidFrame)?;
    let length = u32::try_from(frame.0.len() - 4).map_err(|_| ProtocolError::InvalidFrame)?;
    if length == 0 {
        return Err(ProtocolError::InvalidFrame);
    }
    frame.0[..4].copy_from_slice(&length.to_be_bytes());
    Ok(frame.0)
}
