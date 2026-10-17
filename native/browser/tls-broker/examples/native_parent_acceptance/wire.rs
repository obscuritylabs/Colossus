//! Closed generation-authenticated fixture control, separate from broker traffic.
use super::Failure;
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UnixStream,
};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"CTOY0001";
const DOMAIN: &[u8] = b"colossus-native-tls-fixture-v1\0";
const HEADER: usize = 53;

#[derive(Clone, Copy)]
pub enum Role {
    Controller = 1,
    Parent = 2,
}
pub enum Message {
    Ready { pid: u32, port: u16, verified: bool },
    Crash,
    Ack,
}

pub struct Wire {
    stream: UnixStream,
    key: Zeroizing<[u8; 32]>,
    generation: [u8; 32],
    role: Role,
    sent: u64,
    received: u64,
    usable: bool,
}

impl Wire {
    pub fn new(
        stream: UnixStream,
        key: Zeroizing<[u8; 32]>,
        generation: [u8; 32],
        role: Role,
    ) -> Result<Self, Failure> {
        if *key == [0; 32] || generation == [0; 32] || *key == generation {
            return Err("fixture_control_rejected");
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

    pub async fn send(&mut self, message: Message) -> Result<(), Failure> {
        if !self.usable {
            return Err("fixture_control_interrupted");
        }
        let mut payload = Vec::new();
        match message {
            Message::Ready {
                pid,
                port,
                verified,
            } => {
                if pid == 0 || port == 0 {
                    return Err("fixture_control_rejected");
                }
                payload.push(1);
                payload.extend_from_slice(&pid.to_be_bytes());
                payload.extend_from_slice(&port.to_be_bytes());
                payload.push(u8::from(verified));
            }
            Message::Crash => payload.push(2),
            Message::Ack => payload.push(3),
        }
        let next = self
            .sent
            .checked_add(1)
            .ok_or("fixture_control_interrupted")?;
        let mut header = [0; HEADER];
        header[..8].copy_from_slice(MAGIC);
        header[8] = self.role as u8;
        header[9..17].copy_from_slice(&next.to_be_bytes());
        header[17..49].copy_from_slice(&self.generation);
        header[49..53].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref())
            .map_err(|_| "fixture_control_rejected")?;
        mac.update(DOMAIN);
        mac.update(&header);
        mac.update(&payload);
        self.usable = false;
        self.stream
            .write_all(&header)
            .await
            .map_err(|_| "fixture_control_interrupted")?;
        self.stream
            .write_all(&payload)
            .await
            .map_err(|_| "fixture_control_interrupted")?;
        self.stream
            .write_all(&mac.finalize().into_bytes())
            .await
            .map_err(|_| "fixture_control_interrupted")?;
        self.sent = next;
        self.usable = true;
        Ok(())
    }

    pub async fn receive(&mut self) -> Result<Message, Failure> {
        if !self.usable {
            return Err("fixture_control_interrupted");
        }
        self.usable = false;
        let mut header = [0; HEADER];
        self.stream
            .read_exact(&mut header)
            .await
            .map_err(|_| "fixture_control_interrupted")?;
        let role = match self.role {
            Role::Controller => Role::Parent,
            Role::Parent => Role::Controller,
        };
        let sequence = u64::from_be_bytes(
            header[9..17]
                .try_into()
                .map_err(|_| "fixture_control_rejected")?,
        );
        let length = u32::from_be_bytes(
            header[49..53]
                .try_into()
                .map_err(|_| "fixture_control_rejected")?,
        ) as usize;
        if &header[..8] != MAGIC
            || header[8] != role as u8
            || header[17..49] != self.generation
            || self.received.checked_add(1) != Some(sequence)
            || !(1..=8).contains(&length)
        {
            return Err("fixture_control_rejected");
        }
        let mut payload = [0; 8];
        let mut tag = [0; 32];
        self.stream
            .read_exact(&mut payload[..length])
            .await
            .map_err(|_| "fixture_control_interrupted")?;
        self.stream
            .read_exact(&mut tag)
            .await
            .map_err(|_| "fixture_control_interrupted")?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref())
            .map_err(|_| "fixture_control_rejected")?;
        mac.update(DOMAIN);
        mac.update(&header);
        mac.update(&payload[..length]);
        mac.verify_slice(&tag)
            .map_err(|_| "fixture_control_rejected")?;
        let message = match &payload[..length] {
            [1, a, b, c, d, e, f, verified] if *verified <= 1 => {
                let pid = u32::from_be_bytes([*a, *b, *c, *d]);
                let port = u16::from_be_bytes([*e, *f]);
                if pid == 0 || port == 0 {
                    return Err("fixture_control_rejected");
                }
                Message::Ready {
                    pid,
                    port,
                    verified: *verified == 1,
                }
            }
            [2] => Message::Crash,
            [3] => Message::Ack,
            _ => return Err("fixture_control_rejected"),
        };
        self.received = sequence;
        self.usable = true;
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn generation_and_role_authenticate_the_private_controller_receipt() {
        let (controller, parent) = UnixStream::pair().unwrap();
        let mut controller = Wire::new(
            controller,
            Zeroizing::new([1; 32]),
            [2; 32],
            Role::Controller,
        )
        .unwrap();
        let mut parent = Wire::new(parent, Zeroizing::new([1; 32]), [2; 32], Role::Parent).unwrap();
        parent
            .send(Message::Ready {
                pid: 42,
                port: 9000,
                verified: true,
            })
            .await
            .unwrap();
        assert!(matches!(
            controller.receive().await,
            Ok(Message::Ready {
                pid: 42,
                port: 9000,
                verified: true
            })
        ));
        controller.send(Message::Crash).await.unwrap();
        assert!(matches!(parent.receive().await, Ok(Message::Crash)));
        let (controller, parent) = UnixStream::pair().unwrap();
        let mut controller = Wire::new(
            controller,
            Zeroizing::new([1; 32]),
            [3; 32],
            Role::Controller,
        )
        .unwrap();
        let mut parent = Wire::new(parent, Zeroizing::new([1; 32]), [2; 32], Role::Parent).unwrap();
        parent.send(Message::Ack).await.unwrap();
        assert!(controller.receive().await.is_err());
    }
}
