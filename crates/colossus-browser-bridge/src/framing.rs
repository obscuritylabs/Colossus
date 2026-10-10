use std::{pin::Pin, sync::Arc};

use colossus_ports::BrowserDriverError;
use hmac::{Hmac, Mac as _};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::{BrowserBridgeEnrollment, BrowserBridgeKey};

/// Maximum payload before parsing/allocation. Browser observations have a separate 64 KiB cap.
pub(crate) const MAX_FRAME_BYTES: usize = 128 * 1024;
type HmacSha256 = Hmac<Sha256>;

/// Supervisor-created anonymous inherited channel. There is no endpoint/path constructor.
pub struct InheritedBrowserChannel {
    reader: Pin<Box<dyn AsyncRead + Send>>,
    writer: Pin<Box<dyn AsyncWrite + Send>>,
}

impl InheritedBrowserChannel {
    /// Take ownership of inherited handles after native bootstrap authenticated the child.
    /// No browser page or renderer may supply either handle.
    pub fn new(
        reader: impl AsyncRead + Send + 'static,
        writer: impl AsyncWrite + Send + 'static,
    ) -> Self {
        Self {
            reader: Box::pin(reader),
            writer: Box::pin(writer),
        }
    }
}

pub(crate) struct AuthenticatedChannel {
    channel: InheritedBrowserChannel,
    key: Arc<BrowserBridgeKey>,
    enrollment_digest: [u8; 32],
    role: &'static [u8],
}

impl AuthenticatedChannel {
    pub(crate) fn new(
        channel: InheritedBrowserChannel,
        key: Arc<BrowserBridgeKey>,
        enrollment: &BrowserBridgeEnrollment,
        role: &'static [u8],
    ) -> Result<Self, BrowserDriverError> {
        let bytes = serde_json::to_vec(enrollment).map_err(|_| BrowserDriverError::Failed)?;
        Ok(Self {
            channel,
            key,
            enrollment_digest: Sha256::digest(bytes).into(),
            role,
        })
    }

    fn mac(
        &self,
        direction: &[u8],
        sequence: u64,
        bytes: &[u8],
    ) -> Result<HmacSha256, BrowserDriverError> {
        let mut mac = HmacSha256::new_from_slice(self.key.0.as_ref())
            .map_err(|_| BrowserDriverError::Failed)?;
        mac.update(b"colossus-browser-bridge-v1\0");
        mac.update(&self.enrollment_digest);
        mac.update(self.role);
        mac.update(direction);
        mac.update(&sequence.to_be_bytes());
        mac.update(
            &u32::try_from(bytes.len())
                .map_err(|_| BrowserDriverError::LimitExceeded)?
                .to_be_bytes(),
        );
        mac.update(bytes);
        Ok(mac)
    }

    pub(crate) async fn write<T: Serialize>(
        &mut self,
        direction: &[u8],
        sequence: u64,
        value: &T,
    ) -> Result<(), BrowserDriverError> {
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(value).map_err(|_| BrowserDriverError::Failed)?,
        );
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let tag = self
            .mac(direction, sequence, &bytes)?
            .finalize()
            .into_bytes();
        let length = u32::try_from(bytes.len()).map_err(|_| BrowserDriverError::LimitExceeded)?;
        let writer = &mut self.channel.writer;
        writer
            .write_all(&length.to_be_bytes())
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        writer
            .write_all(&sequence.to_be_bytes())
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        writer
            .write_all(&tag)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        writer
            .write_all(&bytes)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        writer
            .flush()
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
    }

    pub(crate) async fn read<T: DeserializeOwned>(
        &mut self,
        direction: &[u8],
        expected_sequence: u64,
    ) -> Result<T, BrowserDriverError> {
        let reader = &mut self.channel.reader;
        let mut length = [0; 4];
        reader
            .read_exact(&mut length)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let length = usize::try_from(u32::from_be_bytes(length))
            .map_err(|_| BrowserDriverError::LimitExceeded)?;
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let mut sequence = [0; 8];
        let mut tag = [0; 32];
        reader
            .read_exact(&mut sequence)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        reader
            .read_exact(&mut tag)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if u64::from_be_bytes(sequence) != expected_sequence {
            return Err(BrowserDriverError::Denied);
        }
        let mut bytes = zeroize::Zeroizing::new(vec![0; length]);
        reader
            .read_exact(&mut bytes)
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        self.mac(direction, expected_sequence, &bytes)?
            .verify_slice(&tag)
            .map_err(|_| BrowserDriverError::Denied)?;
        serde_json::from_slice(&bytes).map_err(|_| BrowserDriverError::Denied)
    }
}
