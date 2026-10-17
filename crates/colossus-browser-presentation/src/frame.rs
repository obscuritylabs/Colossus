use crate::{Lease, PresentationError};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use zeroize::Zeroizing;

/// Maximum packed frame allocation, independent of declared dimensions.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// Fixed binary metadata and authentication header size.
pub const HEADER_BYTES: usize = 136;
const MAGIC: &[u8; 8] = b"CLSFRM01";
const AUTHENTICATED_BYTES: usize = 104;
type HmacSha256 = Hmac<Sha256>;

/// Packed BGRA pixels. These bytes are page data and must never be executable.
pub struct Frame {
    /// Exact enrollment and presentation ownership.
    pub lease: Lease,
    /// Positive monotonically increasing host frame sequence.
    pub sequence: u64,
    /// Packed BGRA row length in bytes.
    pub stride: u32,
    /// Bounded owned BGRA bytes from the untrusted page.
    pub pixels: Vec<u8>,
}
impl Frame {
    /// Validate packed dimensions and actual byte length before presentation.
    pub fn validate(&self) -> Result<(), PresentationError> {
        self.lease.validate()?;
        if self.sequence == 0
            || self.stride != self.lease.pixel_width * 4
            || self.pixels.len() != self.stride as usize * self.lease.pixel_height as usize
            || self.pixels.len() > MAX_FRAME_BYTES
        {
            return Err(PresentationError::Invalid);
        }
        Ok(())
    }
}
/// An independently inherited host-to-native-presenter channel, bound to one
/// enrollment. Construct only with a supervisor-derived presentation key.
pub struct FrameCodec {
    key: Zeroizing<[u8; 32]>,
    enrollment: [u8; 32],
    lease: Lease,
    sequence: u64,
}
impl FrameCodec {
    /// Bind a separately derived native key to one enrollment and active lease.
    pub fn new(
        key: Zeroizing<[u8; 32]>,
        enrollment: [u8; 32],
        lease: Lease,
    ) -> Result<Self, PresentationError> {
        lease.validate()?;
        Ok(Self {
            key,
            enrollment,
            lease,
            sequence: 0,
        })
    }
    fn mac(&self, header: &[u8], pixels: &[u8]) -> HmacSha256 {
        // HMAC accepts keys of every length; our native key is exactly32bytes.
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref()).expect("fixed HMAC key length");
        mac.update(b"colossus-browser-presentation-v1\0host-to-native-frame\0");
        mac.update(header);
        mac.update(pixels);
        mac
    }
    /// Validate bounded header metadata before a channel allocates payload bytes.
    pub fn payload_length(&self, header: &[u8]) -> Result<usize, PresentationError> {
        if header.len() != HEADER_BYTES || &header[..8] != MAGIC || header[8..40] != self.enrollment
        {
            return Err(PresentationError::Unauthenticated);
        }
        let u64_at = |offset| {
            u64::from_be_bytes(
                header[offset..offset + 8]
                    .try_into()
                    .expect("bounded header"),
            )
        };
        let u32_at = |offset| {
            u32::from_be_bytes(
                header[offset..offset + 4]
                    .try_into()
                    .expect("bounded header"),
            )
        };
        if u64_at(40) != self.lease.tab
            || u64_at(48) != self.lease.session_generation
            || u64_at(56) != self.lease.control_generation
            || u64_at(64) != self.lease.viewport_generation
            || u64_at(72) != self.lease.document_generation
            || u32_at(88) != self.lease.pixel_width
            || u32_at(92) != self.lease.pixel_height
        {
            return Err(PresentationError::Stale);
        }
        let length = u32_at(100) as usize;
        if u64_at(80) <= self.sequence
            || u32_at(96) != self.lease.pixel_width * 4
            || length != self.lease.pixel_width as usize * self.lease.pixel_height as usize * 4
            || length > MAX_FRAME_BYTES
        {
            return Err(PresentationError::Invalid);
        }
        Ok(length)
    }
    /// Authenticate one bounded host frame for the native presenter.
    pub fn encode(&self, frame: &Frame) -> Result<Vec<u8>, PresentationError> {
        frame.validate()?;
        if frame.lease != self.lease {
            return Err(PresentationError::Stale);
        }
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8..40].copy_from_slice(&self.enrollment);
        for (offset, value) in [
            (40, frame.lease.tab),
            (48, frame.lease.session_generation),
            (56, frame.lease.control_generation),
            (64, frame.lease.viewport_generation),
            (72, frame.lease.document_generation),
            (80, frame.sequence),
        ] {
            header[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        }
        for (offset, value) in [
            (88, frame.lease.pixel_width),
            (92, frame.lease.pixel_height),
            (96, frame.stride),
            (
                100,
                u32::try_from(frame.pixels.len()).map_err(|_| PresentationError::LimitExceeded)?,
            ),
        ] {
            header[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        let tag = self
            .mac(&header[..AUTHENTICATED_BYTES], &frame.pixels)
            .finalize()
            .into_bytes();
        header[AUTHENTICATED_BYTES..].copy_from_slice(&tag);
        let mut bytes = Vec::with_capacity(HEADER_BYTES + frame.pixels.len());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&frame.pixels);
        Ok(bytes)
    }
    /// Authenticate and consume one exact frame, rejecting replays and stale ownership.
    pub fn decode(&mut self, mut bytes: Vec<u8>) -> Result<Frame, PresentationError> {
        if bytes.len() < HEADER_BYTES || bytes.len() > HEADER_BYTES + MAX_FRAME_BYTES {
            return Err(PresentationError::LimitExceeded);
        }
        let length = self.payload_length(&bytes[..HEADER_BYTES])?;
        if bytes.len() != HEADER_BYTES + length {
            return Err(PresentationError::Invalid);
        }
        self.mac(&bytes[..AUTHENTICATED_BYTES], &bytes[HEADER_BYTES..])
            .verify_slice(&bytes[AUTHENTICATED_BYTES..HEADER_BYTES])
            .map_err(|_| PresentationError::Unauthenticated)?;
        let sequence = u64::from_be_bytes(bytes[80..88].try_into().expect("bounded header"));
        bytes.drain(..HEADER_BYTES);
        self.sequence = sequence;
        Ok(Frame {
            lease: self.lease,
            sequence,
            stride: self.lease.pixel_width * 4,
            pixels: bytes,
        })
    }
}
