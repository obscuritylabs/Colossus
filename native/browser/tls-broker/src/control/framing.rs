use super::{ControlError, Message, codec};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UnixStream,
};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"CTLSB001";
const DOMAIN: &[u8] = b"colossus-private-tls-broker-v1\0";
pub(super) const HEADER_BYTES: usize = 53;
pub(super) const TAG_BYTES: usize = 32;
pub(super) const MAX_PAYLOAD_BYTES: usize = 1024 * 1024 + 8192;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Role {
    Owner = 1,
    Broker = 2,
}

/// The caller must authenticate the kernel peer and signed native package before
/// constructing this channel or delivering its independent secret. Possession of
/// bootstrap bytes, a PID, or a peer-supplied code hash does not prove admission.
pub(super) struct Channel {
    stream: UnixStream,
    key: Zeroizing<[u8; 32]>,
    generation: [u8; 32],
    role: Role,
    sent: u64,
    received: u64,
    usable: bool,
}

impl Channel {
    pub(super) fn new(
        stream: UnixStream,
        key: Zeroizing<[u8; 32]>,
        generation: [u8; 32],
        role: Role,
    ) -> Result<Self, ControlError> {
        if *key == [0; 32]
            || generation == [0; 32]
            || *key == generation
            || stream.peer_addr().is_err()
        {
            return Err(ControlError::Authentication);
        }
        Ok(Self {
            stream,
            key,
            generation,
            role,
            sent: 0,
            received: 0,
            usable: true,
        })
    }

    pub(super) fn generation(&self) -> [u8; 32] {
        self.generation
    }

    pub(super) async fn send(&mut self, message: &Message) -> Result<(), ControlError> {
        if !self.usable {
            return Err(ControlError::Interrupted);
        }
        let payload = codec::encode(message)?;
        let sequence = self.sent.checked_add(1).ok_or(ControlError::Interrupted)?;
        let mut header = [0_u8; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8] = self.role as u8;
        header[9..17].copy_from_slice(&sequence.to_be_bytes());
        header[17..49].copy_from_slice(&self.generation);
        header[49..53].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        let tag = tag(&self.key, &header, &payload)?;
        // A cancelled or partial write permanently fences the stream. Retrying a
        // frame could reinterpret a prefix already observed by the peer.
        self.usable = false;
        self.stream.write_all(&header).await.map_err(io_error)?;
        self.stream.write_all(&payload).await.map_err(io_error)?;
        self.stream.write_all(&tag).await.map_err(io_error)?;
        self.sent = sequence;
        self.usable = true;
        Ok(())
    }

    pub(super) async fn receive(&mut self) -> Result<Message, ControlError> {
        if !self.usable {
            return Err(ControlError::Interrupted);
        }
        self.usable = false;
        let mut header = [0_u8; HEADER_BYTES];
        self.stream
            .read_exact(&mut header)
            .await
            .map_err(io_error)?;
        let expected_role = match self.role {
            Role::Owner => Role::Broker,
            Role::Broker => Role::Owner,
        };
        let sequence = u64::from_be_bytes(
            header[9..17]
                .try_into()
                .map_err(|_| ControlError::Authentication)?,
        );
        let length = u32::from_be_bytes(
            header[49..53]
                .try_into()
                .map_err(|_| ControlError::Authentication)?,
        ) as usize;
        if &header[..8] != MAGIC
            || header[8] != expected_role as u8
            || header[17..49] != self.generation
            || self.received.checked_add(1) != Some(sequence)
            || length == 0
            || length > MAX_PAYLOAD_BYTES
        {
            return Err(ControlError::Authentication);
        }
        let mut payload = Zeroizing::new(vec![0_u8; length]);
        let mut received_tag = [0_u8; TAG_BYTES];
        self.stream
            .read_exact(&mut payload)
            .await
            .map_err(io_error)?;
        self.stream
            .read_exact(&mut received_tag)
            .await
            .map_err(io_error)?;
        let mut verifier = mac(&self.key)?;
        verifier.update(DOMAIN);
        verifier.update(&header);
        verifier.update(&payload);
        verifier
            .verify_slice(&received_tag)
            .map_err(|_| ControlError::Authentication)?;
        let message = codec::decode(&payload, self.generation)?;
        self.received = sequence;
        self.usable = true;
        Ok(message)
    }
}

fn mac(key: &[u8; 32]) -> Result<Hmac<Sha256>, ControlError> {
    Hmac::<Sha256>::new_from_slice(key).map_err(|_| ControlError::Authentication)
}

fn tag(key: &[u8; 32], header: &[u8], payload: &[u8]) -> Result<[u8; 32], ControlError> {
    let mut mac = mac(key)?;
    mac.update(DOMAIN);
    mac.update(header);
    mac.update(payload);
    Ok(mac.finalize().into_bytes().into())
}

fn io_error(error: std::io::Error) -> ControlError {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionReset => ControlError::Closed,
        _ => ControlError::Interrupted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::{Future as _, poll_fn},
        pin::pin,
        task::Poll,
        time::Duration,
    };

    const KEY: [u8; 32] = [7; 32];
    const GENERATION: [u8; 32] = [8; 32];

    fn frame(role: Role, sequence: u64, generation: [u8; 32], payload: &[u8]) -> Vec<u8> {
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8] = role as u8;
        header[9..17].copy_from_slice(&sequence.to_be_bytes());
        header[17..49].copy_from_slice(&generation);
        header[49..53].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        let mut wire = header.to_vec();
        wire.extend_from_slice(payload);
        wire.extend_from_slice(&tag(&KEY, &header, payload).unwrap());
        wire
    }

    #[tokio::test]
    async fn duplicate_authenticated_sequence_and_reflected_role_fail_closed() {
        let (mut owner, broker) = UnixStream::pair().unwrap();
        let mut broker =
            Channel::new(broker, Zeroizing::new(KEY), GENERATION, Role::Broker).unwrap();
        let status = frame(Role::Owner, 1, GENERATION, &[2]);
        owner.write_all(&status).await.unwrap();
        assert!(matches!(broker.receive().await, Ok(Message::Status)));
        owner.write_all(&status).await.unwrap();
        assert!(matches!(
            broker.receive().await,
            Err(ControlError::Authentication)
        ));
        assert!(matches!(
            broker.receive().await,
            Err(ControlError::Interrupted)
        ));
        let (mut owner, broker) = UnixStream::pair().unwrap();
        let mut broker =
            Channel::new(broker, Zeroizing::new(KEY), GENERATION, Role::Broker).unwrap();
        owner
            .write_all(&frame(Role::Broker, 1, GENERATION, &[2]))
            .await
            .unwrap();
        assert!(matches!(
            broker.receive().await,
            Err(ControlError::Authentication)
        ));
    }

    #[tokio::test]
    async fn authenticated_payload_tamper_and_closed_operation_fail_before_dispatch() {
        for (payload, tamper, expected) in [
            (&[2][..], true, ControlError::Authentication),
            (&[255][..], false, ControlError::InvalidEnvelope),
        ] {
            let (mut owner, broker) = UnixStream::pair().unwrap();
            let mut broker =
                Channel::new(broker, Zeroizing::new(KEY), GENERATION, Role::Broker).unwrap();
            let mut bytes = frame(Role::Owner, 1, GENERATION, payload);
            if tamper {
                bytes[HEADER_BYTES] = 4;
            }
            owner.write_all(&bytes).await.unwrap();
            assert!(matches!(broker.receive().await, Err(error) if error == expected));
        }
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_before_waiting_for_or_allocating_payload() {
        let (mut owner, broker) = UnixStream::pair().unwrap();
        let mut broker =
            Channel::new(broker, Zeroizing::new(KEY), GENERATION, Role::Broker).unwrap();
        let mut header = frame(Role::Owner, 1, GENERATION, &[2])[..HEADER_BYTES].to_vec();
        header[49..53].copy_from_slice(&((MAX_PAYLOAD_BYTES + 1) as u32).to_be_bytes());
        owner.write_all(&header).await.unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), broker.receive())
                .await
                .unwrap(),
            Err(ControlError::Authentication)
        ));
    }

    #[tokio::test]
    async fn cancelled_partial_receive_cannot_reinterpret_remaining_bytes() {
        let (mut owner, broker) = UnixStream::pair().unwrap();
        let mut broker =
            Channel::new(broker, Zeroizing::new(KEY), GENERATION, Role::Broker).unwrap();
        let bytes = frame(Role::Owner, 1, GENERATION, &[2]);
        owner.write_all(&bytes[..9]).await.unwrap();
        {
            let mut received = pin!(broker.receive());
            poll_fn(|context| {
                assert!(received.as_mut().poll(context).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        owner.write_all(&bytes[9..]).await.unwrap();
        assert!(matches!(
            broker.receive().await,
            Err(ControlError::Interrupted)
        ));
    }

    #[tokio::test]
    async fn zero_secret_or_generation_is_rejected_before_control_read() {
        for (key, generation) in [([0; 32], GENERATION), (KEY, [0; 32]), (KEY, KEY)] {
            let (_owner, broker) = UnixStream::pair().unwrap();
            assert!(matches!(
                Channel::new(broker, Zeroizing::new(key), generation, Role::Broker),
                Err(ControlError::Authentication)
            ));
        }
    }
}
