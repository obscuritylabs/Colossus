use super::{
    BootstrapEnvelope, BrokerReady, ControlError, Message,
    framing::{Channel, Role},
};
use crate::{BrokerLease, BrokerState};
use std::time::Duration;
use tokio::{net::UnixStream, time::Instant};
use zeroize::Zeroizing;

const CONTROL_DEADLINE: Duration = Duration::from_secs(5);
const DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// Local evidence only; none of these variants proves native process-tree cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrainEvidence {
    /// No listener or exchange lease was allocated by this control server.
    NoLeaseAllocated,
    /// Both exact listeners and every tracked exchange have positively drained.
    TrackedSocketsClosed,
    /// No positive drainage acknowledgement exists; native owner retains its duty.
    OutcomeUnknown,
}

/// Why this private server finished, without peer-provided diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlExitReason {
    Shutdown,
    ControlClosed,
    Expired,
    Rejected,
    Interrupted,
}

/// Categorical server termination and its exact socket ownership evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlExit {
    pub reason: ControlExitReason,
    pub drainage: DrainEvidence,
}

/// Owner-side private channel after independent native kernel-peer and package
/// admission. This constructor does not establish that admission. The key must be
/// delivered privately only after it succeeds, never through argv, environment,
/// page IPC or a peer-supplied expected code hash.
pub struct BrokerClient {
    channel: Channel,
    bootstrapped: bool,
    in_flight: bool,
    closed: bool,
}

impl BrokerClient {
    /// Adopt one connected, already-admitted Unix stream and independent key.
    pub fn from_admitted_stream(
        stream: UnixStream,
        key: Zeroizing<[u8; 32]>,
        generation: [u8; 32],
    ) -> Result<Self, ControlError> {
        Ok(Self {
            channel: Channel::new(stream, key, generation, Role::Owner)?,
            bootstrapped: false,
            in_flight: false,
            closed: false,
        })
    }

    /// Admit exactly one immutable envelope for this private generation.
    pub async fn bootstrap(
        &mut self,
        envelope: BootstrapEnvelope,
    ) -> Result<BrokerReady, ControlError> {
        if self.bootstrapped || envelope.bootstrap.generation != self.channel.generation() {
            return Err(ControlError::InvalidEnvelope);
        }
        match self.exchange(Message::Bootstrap(envelope)).await? {
            Message::Ready(ready) => {
                self.bootstrapped = true;
                Ok(ready)
            }
            _ => {
                self.closed = true;
                Err(ControlError::Rejected)
            }
        }
    }
    /// Carry only a reviewed opaque native registry selection on the same
    /// authenticated Core channel. No secret or signer can enter this envelope.
    pub async fn bootstrap_with_pki(
        &mut self,
        envelope: BootstrapEnvelope,
        selection: crate::PkiSelection,
    ) -> Result<BrokerReady, ControlError> {
        if self.bootstrapped || envelope.bootstrap.generation != self.channel.generation() {
            return Err(ControlError::InvalidEnvelope);
        }
        match self
            .exchange(Message::BootstrapPki(envelope, selection))
            .await?
        {
            Message::Ready(ready) => {
                self.bootstrapped = true;
                Ok(ready)
            }
            _ => {
                self.closed = true;
                Err(ControlError::Rejected)
            }
        }
    }

    /// Observe current socket-lease state without claiming process cleanup.
    pub async fn status(&mut self) -> Result<BrokerState, ControlError> {
        self.require_bootstrap()?;
        match self.exchange(Message::Status).await? {
            Message::State(state) => Ok(state),
            _ => {
                self.closed = true;
                Err(ControlError::Rejected)
            }
        }
    }

    /// Return only after both listeners and every tracked TLS exchange have drained.
    pub async fn revoke(&mut self) -> Result<(), ControlError> {
        self.require_bootstrap()?;
        match self.exchange(Message::Revoke).await? {
            Message::Revoked => Ok(()),
            _ => {
                self.closed = true;
                Err(ControlError::Rejected)
            }
        }
    }

    /// Request final drainage and receive a generation-bound acknowledgement.
    /// The native owner must still retain and reap the exact broker process.
    pub async fn shutdown(&mut self) -> Result<(), ControlError> {
        self.require_bootstrap()?;
        match self.exchange(Message::Shutdown).await? {
            Message::ShutdownAck => {
                self.closed = true;
                Ok(())
            }
            _ => {
                self.closed = true;
                Err(ControlError::Rejected)
            }
        }
    }

    fn require_bootstrap(&self) -> Result<(), ControlError> {
        if !self.bootstrapped || self.closed {
            Err(ControlError::Rejected)
        } else {
            Ok(())
        }
    }

    async fn exchange(&mut self, message: Message) -> Result<Message, ControlError> {
        if self.in_flight || self.closed {
            return Err(ControlError::Interrupted);
        }
        // Cancellation at any await fences future requests, including the gap
        // after a complete write and before response receipt has begun.
        self.in_flight = true;
        let response = tokio::time::timeout(CONTROL_DEADLINE, async {
            self.channel.send(&message).await?;
            self.channel.receive().await
        })
        .await
        .map_err(|_| ControlError::Interrupted)??;
        self.in_flight = false;
        Ok(response)
    }
}

/// Serve only an independently admitted owned connected Unix stream. Native entry
/// must validate the kernel peer against an independent signed-package policy
/// before obtaining this key or calling here. This API cannot prove admission or
/// enable production, and accepts no file, descriptor, process or signer nomination.
/// EOF, malformed authentication, expiry and send failure all attempt bounded
/// socket drainage; only a successful drain can produce a shutdown acknowledgement.
pub async fn serve_control(
    stream: UnixStream,
    key: Zeroizing<[u8; 32]>,
    generation: [u8; 32],
) -> ControlExit {
    serve_with_identity(stream, key, generation, None).await
}

/// Local native composition only; this object is never selected from IPC. The
/// ordinary closed bootstrap still carries only public upstream TLS authority.
pub async fn serve_control_with_local_identity(
    stream: UnixStream,
    key: Zeroizing<[u8; 32]>,
    generation: [u8; 32],
    identity: crate::ClientIdentity,
) -> ControlExit {
    serve_with_identity(stream, key, generation, Some(identity)).await
}

#[cfg(all(target_os = "macos", feature = "macos-native-custody-development"))]
/// The configured native lane requires a reviewed selector before any listener
/// or identity import. The concrete registry was independently admitted locally;
/// no received signer, path or public trust nomination is accepted here.
pub async fn serve_control_with_native_registry(
    stream: UnixStream,
    key: Zeroizing<[u8; 32]>,
    generation: [u8; 32],
    registry: &crate::NativeRegistry,
) -> ControlExit {
    let mut channel = match Channel::new(stream, key, generation, Role::Broker) {
        Ok(channel) => channel,
        Err(error) => return exit(error, DrainEvidence::NoLeaseAllocated),
    };
    let received = match receive_first(&mut channel, true).await {
        Ok(received) => received,
        Err(exit) => return exit,
    };
    let Message::BootstrapPki(envelope, selection) = received else {
        return exit(ControlError::Rejected, DrainEvidence::NoLeaseAllocated);
    };
    let resolved = match registry.resolve(envelope, selection) {
        Ok(resolved) => resolved,
        Err(_) => return exit(ControlError::Rejected, DrainEvidence::NoLeaseAllocated),
    };
    serve_envelope(channel, generation, resolved.envelope, resolved.identity).await
}

async fn serve_with_identity(
    stream: UnixStream,
    key: Zeroizing<[u8; 32]>,
    generation: [u8; 32],
    identity: Option<crate::ClientIdentity>,
) -> ControlExit {
    let mut channel = match Channel::new(stream, key, generation, Role::Broker) {
        Ok(channel) => channel,
        Err(error) => return exit(error, DrainEvidence::NoLeaseAllocated),
    };
    let received = match receive_first(&mut channel, false).await {
        Ok(received) => received,
        Err(exit) => return exit,
    };
    let Message::Bootstrap(envelope) = received else {
        return exit(ControlError::Rejected, DrainEvidence::NoLeaseAllocated);
    };
    serve_envelope(channel, generation, envelope, identity).await
}

async fn receive_first(channel: &mut Channel, configured: bool) -> Result<Message, ControlExit> {
    match tokio::time::timeout(CONTROL_DEADLINE, channel.receive()).await {
        Ok(Ok(message))
            if matches!(
                (&message, configured),
                (Message::Bootstrap(_), false) | (Message::BootstrapPki(_, _), true)
            ) =>
        {
            Ok(message)
        }
        Ok(Ok(_)) => Err(exit(
            ControlError::Rejected,
            DrainEvidence::NoLeaseAllocated,
        )),
        Ok(Err(error)) => Err(exit(error, DrainEvidence::NoLeaseAllocated)),
        Err(_) => Err(exit(
            ControlError::Interrupted,
            DrainEvidence::NoLeaseAllocated,
        )),
    }
}

async fn serve_envelope(
    mut channel: Channel,
    generation: [u8; 32],
    envelope: BootstrapEnvelope,
    identity: Option<crate::ClientIdentity>,
) -> ControlExit {
    let deadline = Instant::now() + envelope.limits.lifetime;
    let started = match identity {
        Some(identity) => {
            BrokerLease::start_with_identity_on_loopbacks(
                envelope.bootstrap,
                envelope.limits,
                envelope.port,
                identity,
            )
            .await
        }
        None => {
            BrokerLease::start_on_loopbacks(envelope.bootstrap, envelope.limits, envelope.port)
                .await
        }
    };
    let mut lease = match started {
        Ok(lease) => lease,
        Err(_) => {
            return ControlExit {
                reason: ControlExitReason::Rejected,
                drainage: DrainEvidence::NoLeaseAllocated,
            };
        }
    };
    let ready = match ready(&lease, &generation) {
        Some(ready) => ready,
        None => {
            return ControlExit {
                reason: ControlExitReason::Expired,
                drainage: drain(&mut lease).await,
            };
        }
    };
    if let Err(error) = send(&mut channel, &Message::Ready(ready)).await {
        return exit(error, drain(&mut lease).await);
    }
    loop {
        let command = match tokio::time::timeout_at(deadline, channel.receive()).await {
            Ok(Ok(command)) => command,
            Ok(Err(error)) => return exit(error, drain(&mut lease).await),
            Err(_) => {
                return ControlExit {
                    reason: ControlExitReason::Expired,
                    drainage: drain(&mut lease).await,
                };
            }
        };
        let (reply, shutdown) = match command {
            Message::Status => (Message::State(lease.state()), false),
            Message::Revoke | Message::Shutdown => {
                let shutdown = matches!(command, Message::Shutdown);
                if drain(&mut lease).await != DrainEvidence::TrackedSocketsClosed {
                    return ControlExit {
                        reason: ControlExitReason::Interrupted,
                        drainage: DrainEvidence::OutcomeUnknown,
                    };
                }
                (
                    if shutdown {
                        Message::ShutdownAck
                    } else {
                        Message::Revoked
                    },
                    shutdown,
                )
            }
            _ => {
                return ControlExit {
                    reason: ControlExitReason::Rejected,
                    drainage: drain(&mut lease).await,
                };
            }
        };
        if let Err(error) = send(&mut channel, &reply).await {
            return exit(error, drain(&mut lease).await);
        }
        if shutdown {
            return ControlExit {
                reason: ControlExitReason::Shutdown,
                drainage: DrainEvidence::TrackedSocketsClosed,
            };
        }
    }
}

fn ready(lease: &BrokerLease, generation: &[u8; 32]) -> Option<BrokerReady> {
    Some(BrokerReady {
        port: lease.address().port(),
        credential: Zeroizing::new(lease.credential(generation)?.to_owned()),
        ca_der: lease.public_ca(generation)?.to_vec(),
        leaf_der: lease.public_leaf(generation)?.to_vec(),
        leaf_sha256: *lease.public_leaf_sha256(generation)?,
    })
}

async fn send(channel: &mut Channel, message: &Message) -> Result<(), ControlError> {
    tokio::time::timeout(CONTROL_DEADLINE, channel.send(message))
        .await
        .map_err(|_| ControlError::Interrupted)?
}

async fn drain(lease: &mut BrokerLease) -> DrainEvidence {
    match tokio::time::timeout(DRAIN_DEADLINE, lease.revoke()).await {
        Ok(Ok(())) if lease.state() == BrokerState::Revoked => DrainEvidence::TrackedSocketsClosed,
        _ => DrainEvidence::OutcomeUnknown,
    }
}

fn exit(error: ControlError, drainage: DrainEvidence) -> ControlExit {
    ControlExit {
        reason: match error {
            ControlError::Closed => ControlExitReason::ControlClosed,
            ControlError::Authentication
            | ControlError::InvalidEnvelope
            | ControlError::Rejected => ControlExitReason::Rejected,
            ControlError::Interrupted => ControlExitReason::Interrupted,
        },
        drainage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn configured_lane_requires_selector_before_any_listener_or_identity_effect() {
        let envelope = || BootstrapEnvelope {
            bootstrap: crate::Bootstrap {
                origin: "https://127.0.0.1:4433".into(),
                destinations: vec![([127, 0, 0, 1], 4433).into()],
                ca_der: vec![vec![1]],
                generation: [32; 32],
            },
            limits: crate::BrokerLimits {
                connections: 1,
                lifetime: Duration::from_secs(10),
                exchange_timeout: Duration::from_secs(1),
                exchange_bytes: 65536,
            },
            port: 4434,
        };
        for (configured, reviewed, accepted) in [
            (true, false, false),
            (false, true, false),
            (true, true, true),
            (false, false, true),
        ] {
            let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
            let mut owner = Channel::new(
                owner_stream,
                Zeroizing::new([31; 32]),
                [32; 32],
                Role::Owner,
            )
            .unwrap();
            let mut broker = Channel::new(
                broker_stream,
                Zeroizing::new([31; 32]),
                [32; 32],
                Role::Broker,
            )
            .unwrap();
            let message = if reviewed {
                Message::BootstrapPki(
                    envelope(),
                    crate::PkiSelection {
                        scope_binding: [1; 32],
                        ca_ids: vec![[2; 32]],
                        client: None,
                    },
                )
            } else {
                Message::Bootstrap(envelope())
            };
            owner.send(&message).await.unwrap();
            let result = receive_first(&mut broker, configured).await;
            assert_eq!(result.is_ok(), accepted);
            if let Err(exit) = result {
                assert_eq!(exit.reason, ControlExitReason::Rejected);
                assert_eq!(exit.drainage, DrainEvidence::NoLeaseAllocated);
            }
        }
    }

    #[tokio::test]
    async fn cancelled_client_exchange_fences_gap_after_complete_request_write() {
        let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
        let key = [31; 32];
        let generation = [32; 32];
        let mut owner =
            BrokerClient::from_admitted_stream(owner_stream, Zeroizing::new(key), generation)
                .unwrap();
        owner.bootstrapped = true;
        let mut broker =
            Channel::new(broker_stream, Zeroizing::new(key), generation, Role::Broker).unwrap();
        let (received, observed) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            assert!(matches!(broker.receive().await, Ok(Message::Status)));
            received.send(()).unwrap();
            broker
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), owner.status())
                .await
                .is_err()
        );
        observed.await.unwrap();
        assert_eq!(owner.status().await, Err(ControlError::Interrupted));
        let mut broker = peer.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), broker.receive())
                .await
                .is_err()
        );
        drop(owner);
    }

    #[tokio::test]
    async fn a_second_authenticated_bootstrap_cannot_reconfigure_an_active_lease() {
        use crate::{Bootstrap, BrokerLimits};
        use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
        use std::net::{Ipv4Addr, Ipv6Addr};
        use tokio::net::{TcpListener, TcpStream};
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let ca = params.self_signed(&KeyPair::generate().unwrap()).unwrap();
        let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let generation = [32; 32];
        let envelope = || BootstrapEnvelope {
            bootstrap: Bootstrap {
                origin: "https://localhost".into(),
                destinations: vec![(Ipv4Addr::LOCALHOST, 443).into()],
                ca_der: vec![ca.der().to_vec()],
                generation,
            },
            limits: BrokerLimits {
                connections: 1,
                lifetime: Duration::from_secs(10),
                exchange_timeout: Duration::from_secs(1),
                exchange_bytes: 64 * 1024,
            },
            port,
        };
        let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
        let server = tokio::spawn(serve_control(
            broker_stream,
            Zeroizing::new([31; 32]),
            generation,
        ));
        let mut owner = Channel::new(
            owner_stream,
            Zeroizing::new([31; 32]),
            generation,
            Role::Owner,
        )
        .unwrap();
        owner.send(&Message::Bootstrap(envelope())).await.unwrap();
        assert!(matches!(owner.receive().await, Ok(Message::Ready(_))));
        owner.send(&Message::Bootstrap(envelope())).await.unwrap();
        assert!(matches!(owner.receive().await, Err(ControlError::Closed)));
        let exit = server.await.unwrap();
        assert_eq!(exit.reason, ControlExitReason::Rejected);
        assert_eq!(exit.drainage, DrainEvidence::TrackedSocketsClosed);
        assert!(
            TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
        );
        assert!(
            TcpStream::connect((Ipv6Addr::LOCALHOST, port))
                .await
                .is_err()
        );
    }
}
