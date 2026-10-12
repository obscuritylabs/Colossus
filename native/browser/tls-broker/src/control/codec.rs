use super::{BootstrapEnvelope, BrokerReady, ControlError, Message, framing::MAX_PAYLOAD_BYTES};
use crate::{Bootstrap, BrokerLimits, BrokerState};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use zeroize::Zeroizing;

const MAX_ORIGIN_BYTES: usize = 2048;
const MAX_ROOT_BYTES: usize = 1024 * 1024;
const MAX_DOWNSTREAM_CERT_BYTES: usize = 65_536;

pub(super) fn encode(message: &Message) -> Result<Zeroizing<Vec<u8>>, ControlError> {
    let mut out = Zeroizing::new(Vec::new());
    match message {
        Message::Bootstrap(envelope) | Message::BootstrapPki(envelope, _) => {
            let input = &envelope.bootstrap;
            validate_envelope(envelope)?;
            out.push(if matches!(message, Message::BootstrapPki(_, _)) {
                9
            } else {
                1
            });
            short(&mut out, input.origin.len() as u16);
            out.extend_from_slice(input.origin.as_bytes());
            out.push(input.destinations.len() as u8);
            for destination in &input.destinations {
                match destination.ip() {
                    IpAddr::V4(ip) => {
                        out.push(4);
                        out.extend_from_slice(&ip.octets());
                    }
                    IpAddr::V6(ip) => {
                        out.push(6);
                        out.extend_from_slice(&ip.octets());
                    }
                }
                short(&mut out, destination.port());
            }
            out.push(input.ca_der.len() as u8);
            for root in &input.ca_der {
                blob(&mut out, root);
            }
            short(&mut out, envelope.port);
            short(&mut out, envelope.limits.connections as u16);
            long(&mut out, milliseconds(envelope.limits.lifetime)?);
            long(&mut out, milliseconds(envelope.limits.exchange_timeout)?);
            word(&mut out, envelope.limits.exchange_bytes as u32);
            if let Message::BootstrapPki(_, selection) = message {
                blob(
                    &mut out,
                    &selection
                        .encode()
                        .map_err(|_| ControlError::InvalidEnvelope)?,
                );
            }
        }
        Message::Status => out.push(2),
        Message::Revoke => out.push(3),
        Message::Shutdown => out.push(4),
        Message::Ready(ready) => {
            validate_ready(ready)?;
            out.push(5);
            short(&mut out, ready.port);
            out.push(ready.credential.len() as u8);
            out.extend_from_slice(ready.credential.as_bytes());
            blob(&mut out, &ready.ca_der);
            blob(&mut out, &ready.leaf_der);
            out.extend_from_slice(&ready.leaf_sha256);
        }
        Message::State(state) => {
            out.push(6);
            out.push(*state as u8);
        }
        Message::Revoked => out.push(7),
        Message::ShutdownAck => out.push(8),
    }
    if out.len() > MAX_PAYLOAD_BYTES {
        return Err(ControlError::InvalidEnvelope);
    }
    Ok(out)
}

pub(super) fn decode(bytes: &[u8], generation: [u8; 32]) -> Result<Message, ControlError> {
    let mut input = Cursor { bytes, offset: 0 };
    let kind = input.byte()?;
    let message = match kind {
        1 | 9 => {
            let origin_size = input.short()? as usize;
            if origin_size == 0 || origin_size > MAX_ORIGIN_BYTES {
                return Err(ControlError::InvalidEnvelope);
            }
            let origin = std::str::from_utf8(input.take(origin_size)?)
                .map_err(|_| ControlError::InvalidEnvelope)?
                .to_owned();
            let count = input.byte()?;
            if count == 0 || count > 8 {
                return Err(ControlError::InvalidEnvelope);
            }
            let mut destinations = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let ip = match input.byte()? {
                    4 => IpAddr::V4(Ipv4Addr::from(
                        <[u8; 4]>::try_from(input.take(4)?)
                            .map_err(|_| ControlError::InvalidEnvelope)?,
                    )),
                    6 => IpAddr::V6(Ipv6Addr::from(
                        <[u8; 16]>::try_from(input.take(16)?)
                            .map_err(|_| ControlError::InvalidEnvelope)?,
                    )),
                    _ => return Err(ControlError::InvalidEnvelope),
                };
                destinations.push(SocketAddr::new(ip, input.short()?));
            }
            let count = input.byte()?;
            if count == 0 || count > 8 {
                return Err(ControlError::InvalidEnvelope);
            }
            let mut ca_der = Vec::with_capacity(count as usize);
            let mut total = 0_usize;
            for _ in 0..count {
                let der = input.blob(MAX_ROOT_BYTES)?;
                total = total
                    .checked_add(der.len())
                    .ok_or(ControlError::InvalidEnvelope)?;
                if total > MAX_ROOT_BYTES {
                    return Err(ControlError::InvalidEnvelope);
                }
                ca_der.push(der.to_vec());
            }
            let port = input.short()?;
            let connections = input.short()? as usize;
            let lifetime = Duration::from_millis(input.long()?);
            let exchange_timeout = Duration::from_millis(input.long()?);
            let exchange_bytes = input.word()? as usize;
            let envelope = BootstrapEnvelope {
                bootstrap: Bootstrap {
                    origin,
                    destinations,
                    ca_der,
                    generation,
                },
                limits: BrokerLimits {
                    connections,
                    lifetime,
                    exchange_timeout,
                    exchange_bytes,
                },
                port,
            };
            validate_envelope(&envelope)?;
            if kind == 9 {
                let selection = crate::PkiSelection::decode(input.blob(354)?)
                    .map_err(|_| ControlError::InvalidEnvelope)?;
                Message::BootstrapPki(envelope, selection)
            } else {
                Message::Bootstrap(envelope)
            }
        }
        2 => Message::Status,
        3 => Message::Revoke,
        4 => Message::Shutdown,
        5 => {
            let port = input.short()?;
            let credential_size = input.byte()? as usize;
            if credential_size != 44 {
                return Err(ControlError::InvalidEnvelope);
            }
            let credential = Zeroizing::new(
                std::str::from_utf8(input.take(credential_size)?)
                    .map_err(|_| ControlError::InvalidEnvelope)?
                    .to_owned(),
            );
            let ca_der = input.blob(MAX_DOWNSTREAM_CERT_BYTES)?.to_vec();
            let leaf_der = input.blob(MAX_DOWNSTREAM_CERT_BYTES)?.to_vec();
            let leaf_sha256 = input
                .take(32)?
                .try_into()
                .map_err(|_| ControlError::InvalidEnvelope)?;
            let ready = BrokerReady {
                port,
                credential,
                ca_der,
                leaf_der,
                leaf_sha256,
            };
            validate_ready(&ready)?;
            Message::Ready(ready)
        }
        6 => Message::State(match input.byte()? {
            0 => BrokerState::Active,
            1 => BrokerState::Revoking,
            2 => BrokerState::Revoked,
            3 => BrokerState::Interrupted,
            _ => return Err(ControlError::InvalidEnvelope),
        }),
        7 => Message::Revoked,
        8 => Message::ShutdownAck,
        _ => return Err(ControlError::InvalidEnvelope),
    };
    if input.offset != bytes.len() {
        return Err(ControlError::InvalidEnvelope);
    }
    Ok(message)
}

fn validate_envelope(envelope: &BootstrapEnvelope) -> Result<(), ControlError> {
    let input = &envelope.bootstrap;
    if input.origin.is_empty()
        || input.origin.len() > MAX_ORIGIN_BYTES
        || input.destinations.is_empty()
        || input.destinations.len() > 8
        || input.destinations.iter().any(|address| matches!(address, SocketAddr::V6(address) if address.flowinfo() != 0 || address.scope_id() != 0))
        || input.ca_der.is_empty()
        || input.ca_der.len() > 8
        || input
            .ca_der
            .iter()
            .any(|root| root.is_empty() || root.len() > MAX_ROOT_BYTES)
        || input.ca_der.iter().map(Vec::len).sum::<usize>() > MAX_ROOT_BYTES
        || envelope.port == 0
        || !(1..=256).contains(&envelope.limits.connections)
        || envelope.limits.lifetime.is_zero()
        || envelope.limits.lifetime > Duration::from_secs(86400)
        || envelope.limits.exchange_timeout.is_zero()
        || envelope.limits.exchange_timeout > envelope.limits.lifetime
        || !(64 * 1024..=64 * 1024 * 1024).contains(&envelope.limits.exchange_bytes)
    {
        return Err(ControlError::InvalidEnvelope);
    }
    Ok(())
}

fn validate_ready(ready: &BrokerReady) -> Result<(), ControlError> {
    if ready.port == 0
        || ready.credential.len() != 44
        || ready.ca_der.is_empty()
        || ready.ca_der.len() > MAX_DOWNSTREAM_CERT_BYTES
        || ready.leaf_der.is_empty()
        || ready.leaf_der.len() > MAX_DOWNSTREAM_CERT_BYTES
        || <[u8; 32]>::from(Sha256::digest(&ready.leaf_der)) != ready.leaf_sha256
    {
        return Err(ControlError::InvalidEnvelope);
    }
    let decoded = Zeroizing::new(
        base64::engine::general_purpose::STANDARD
            .decode(ready.credential.as_bytes())
            .map_err(|_| ControlError::InvalidEnvelope)?,
    );
    if decoded.len() != 32 || decoded.iter().all(|byte| *byte == 0) {
        return Err(ControlError::InvalidEnvelope);
    }
    colossus_native_browser_pki::ca_der(&ready.ca_der)
        .map_err(|_| ControlError::InvalidEnvelope)?;
    Ok(())
}

fn milliseconds(value: Duration) -> Result<u64, ControlError> {
    let millis = u64::try_from(value.as_millis()).map_err(|_| ControlError::InvalidEnvelope)?;
    if millis == 0 {
        return Err(ControlError::InvalidEnvelope);
    }
    Ok(millis)
}
fn short(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn long(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn blob(out: &mut Vec<u8>, value: &[u8]) {
    word(out, value.len() as u32);
    out.extend_from_slice(value);
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], ControlError> {
        let end = self
            .offset
            .checked_add(size)
            .ok_or(ControlError::InvalidEnvelope)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(ControlError::InvalidEnvelope)?;
        self.offset = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, ControlError> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> Result<u16, ControlError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| ControlError::InvalidEnvelope)?,
        ))
    }
    fn word(&mut self) -> Result<u32, ControlError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| ControlError::InvalidEnvelope)?,
        ))
    }
    fn long(&mut self) -> Result<u64, ControlError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| ControlError::InvalidEnvelope)?,
        ))
    }
    fn blob(&mut self, max: usize) -> Result<&'a [u8], ControlError> {
        let size = self.word()? as usize;
        if size == 0 || size > max {
            return Err(ControlError::InvalidEnvelope);
        }
        self.take(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_ipv6_pin_is_rejected_instead_of_silently_changing_the_destination() {
        let scoped = std::net::SocketAddrV6::new(Ipv6Addr::LOCALHOST, 443, 0, 4);
        let envelope = BootstrapEnvelope {
            bootstrap: Bootstrap {
                origin: "https://localhost".into(),
                destinations: vec![scoped.into()],
                ca_der: vec![vec![1]],
                generation: [6; 32],
            },
            limits: BrokerLimits {
                connections: 1,
                lifetime: Duration::from_secs(2),
                exchange_timeout: Duration::from_secs(1),
                exchange_bytes: 64 * 1024,
            },
            port: 9000,
        };
        assert!(matches!(
            encode(&Message::Bootstrap(envelope)),
            Err(ControlError::InvalidEnvelope)
        ));
    }

    #[test]
    fn closed_command_rejects_trailing_fields_and_unknown_operations() {
        for bytes in [&[2, 0][..], &[4, 1][..], &[255][..]] {
            assert!(matches!(
                decode(bytes, [6; 32]),
                Err(ControlError::InvalidEnvelope)
            ));
        }
    }
}
