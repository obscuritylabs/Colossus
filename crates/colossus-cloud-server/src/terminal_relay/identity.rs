//! Purpose-separated replica authentication and verified TLS channel ownership.
use crate::{certificates::CertificateAuthority, config::Config, server::State};
use colossus_api::HumanTerminalOperation;
use colossus_cloud::storage::ReplicaRoute;
use hmac::{Hmac, Mac};
use rcgen::{CertificateParams, KeyPair};
use sha2::Sha256;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use zeroize::Zeroizing;

struct CachedChannel {
    channel: Channel,
    address: SocketAddr,
    created: Instant,
    used: Instant,
}

pub(crate) struct RelayTransport {
    token: Zeroizing<String>,
    advertised: Mutex<Option<SocketAddr>>,
    channels: Mutex<HashMap<String, CachedChannel>>,
    admission: RelayAdmission,
}

/// Keep control traffic responsive even when long polls fill the read budget.
struct RelayAdmission {
    reads: Arc<tokio::sync::Semaphore>,
    controls: Arc<tokio::sync::Semaphore>,
}

impl RelayAdmission {
    fn new() -> Self {
        Self {
            reads: Arc::new(tokio::sync::Semaphore::new(96)),
            controls: Arc::new(tokio::sync::Semaphore::new(32)),
        }
    }

    fn admit(
        &self,
        operation: &HumanTerminalOperation,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::TryAcquireError> {
        let budget = if matches!(operation, HumanTerminalOperation::Read { .. }) {
            &self.reads
        } else {
            &self.controls
        };
        budget.clone().try_acquire_owned()
    }
}

impl RelayTransport {
    pub(crate) fn new(config: &Config) -> Result<Self, &'static str> {
        let secret = Zeroizing::new(
            std::fs::read(&config.ca_key).map_err(|_| "replica authentication unavailable")?,
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret)
            .map_err(|_| "replica authentication invalid")?;
        mac.update(b"colossus.control-plane.human-terminal-relay.v1");
        let token = Zeroizing::new(hex::encode(mac.finalize().into_bytes()));
        let address = std::env::var("COLOSSUS_CONTROL_PLANE_RELAY_ADDRESS")
            .ok()
            .map(|value| {
                value
                    .parse::<SocketAddr>()
                    .map_err(|_| "invalid replica relay address")
            })
            .transpose()?;
        let address = match address {
            Some(value) => Some(value),
            None => std::env::var("COLOSSUS_CONTROL_PLANE_POD_IP")
                .ok()
                .map(|value| {
                    value
                        .parse::<std::net::IpAddr>()
                        .map(|ip| SocketAddr::new(ip, config.grpc_bind.port()))
                        .map_err(|_| "invalid replica pod address")
                })
                .transpose()?,
        };
        if address.is_some_and(|value| {
            value.ip().is_unspecified() || value.ip().is_multicast() || value.port() == 0
        }) {
            return Err("invalid replica relay address");
        }
        Ok(Self {
            token,
            advertised: Mutex::new(address),
            channels: Mutex::new(HashMap::new()),
            admission: RelayAdmission::new(),
        })
    }
    pub(super) fn admit(
        &self,
        operation: &HumanTerminalOperation,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::TryAcquireError> {
        self.admission.admit(operation)
    }
    pub(super) fn token(&self) -> &str {
        &self.token
    }
    pub(super) fn authenticate(&self, supplied: Option<&str>) -> bool {
        token_matches(&self.token, supplied)
    }
    pub(crate) fn local_listener(&self, address: SocketAddr, development: bool) {
        if development
            && address.ip().is_loopback()
            && let Ok(mut current) = self.advertised.lock()
            && current.is_none()
        {
            *current = Some(address);
        }
    }
    pub(crate) async fn publish(&self, state: &Arc<State>) -> Result<(), &'static str> {
        let address = *self
            .advertised
            .lock()
            .map_err(|_| "replica route unavailable")?;
        if let Some(address) = address {
            let now = crate::http::now();
            state
                .repo
                .storage()
                .publish_replica_route(
                    ReplicaRoute {
                        replica_id: state.replica_id.clone(),
                        address,
                        expires_at: now.saturating_add(30),
                    },
                    now,
                )
                .await
                .map_err(|_| "replica route publication unavailable")?;
        }
        Ok(())
    }
    pub(super) async fn channel(
        &self,
        state: &Arc<State>,
        route: &ReplicaRoute,
    ) -> Result<Channel, &'static str> {
        {
            let mut channels = self
                .channels
                .lock()
                .map_err(|_| "replica channels unavailable")?;
            channels.retain(|_, entry| entry.created.elapsed() < Duration::from_secs(86400));
            if let Some(entry) = channels.get_mut(&route.replica_id)
                && entry.address == route.address
            {
                entry.used = Instant::now();
                return Ok(entry.channel.clone());
            }
        }
        let endpoint =
            url::Url::parse(&state.config.grpc_endpoint).map_err(|_| "invalid replica TLS name")?;
        let domain = endpoint.host_str().ok_or("invalid replica TLS name")?;
        let identity = client_identity(&state.ca)?;
        let channel = Endpoint::from_shared(format!("https://{}", route.address))
            .map_err(|_| "invalid replica address")?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(6))
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(domain)
                    .ca_certificate(Certificate::from_pem(&state.ca.pem))
                    .identity(identity),
            )
            .map_err(|_| "invalid replica TLS policy")?
            .connect_lazy();
        let mut channels = self
            .channels
            .lock()
            .map_err(|_| "replica channels unavailable")?;
        make_room(&mut channels, &route.replica_id);
        channels.insert(
            route.replica_id.clone(),
            CachedChannel {
                channel: channel.clone(),
                address: route.address,
                created: Instant::now(),
                used: Instant::now(),
            },
        );
        Ok(channel)
    }
}

fn make_room(channels: &mut HashMap<String, CachedChannel>, incoming: &str) {
    if channels.len() >= 64
        && !channels.contains_key(incoming)
        && let Some(oldest) = channels
            .iter()
            .min_by_key(|(_, entry)| entry.used)
            .map(|(id, _)| id.clone())
    {
        channels.remove(&oldest);
    }
}

pub(super) fn token_matches(expected: &str, supplied: Option<&str>) -> bool {
    supplied.is_some_and(|value| {
        value.len() == expected.len() && bool::from(value.as_bytes().ct_eq(expected.as_bytes()))
    })
}

fn client_identity(ca: &CertificateAuthority) -> Result<Identity, &'static str> {
    let key = KeyPair::generate().map_err(|_| "replica TLS key unavailable")?;
    let csr = CertificateParams::default()
        .serialize_request(&key)
        .map_err(|_| "replica TLS request unavailable")?
        .pem()
        .map_err(|_| "replica TLS request unavailable")?;
    let (certificate, _) = ca.sign(&csr)?;
    let pem = Zeroizing::new(key.serialize_pem());
    Ok(Identity::from_pem(certificate, pem.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saturated_read_budget_keeps_control_capacity_available() {
        let admission = RelayAdmission::new();
        let read = HumanTerminalOperation::Read {
            terminal_id: "terminal".into(),
            after_sequence: 0,
            max_bytes: 65536,
            wait_ms: 1000,
        };
        let readers: Vec<_> = (0..96).map(|_| admission.admit(&read).unwrap()).collect();
        assert!(admission.admit(&read).is_err());
        let close = HumanTerminalOperation::Close {
            terminal_id: "terminal".into(),
            writer_epoch: 1,
        };
        let controls: Vec<_> = (0..32).map(|_| admission.admit(&close).unwrap()).collect();
        assert!(admission.admit(&close).is_err());
        drop(readers);
        assert!(admission.admit(&read).is_ok());
        drop(controls);
        assert!(admission.admit(&close).is_ok());
    }
    #[tokio::test]
    async fn rolling_pod_churn_evicts_old_channels_instead_of_blocking_new_owners() {
        let channel = Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let mut channels = HashMap::new();
        let now = Instant::now();
        for number in 0..256 {
            let id = format!("replica-{number}");
            make_room(&mut channels, &id);
            channels.insert(
                id.clone(),
                CachedChannel {
                    channel: channel.clone(),
                    address: "127.0.0.1:8443".parse().unwrap(),
                    created: now,
                    used: now + Duration::from_secs(number),
                },
            );
            assert!(channels.contains_key(&id));
            assert!(channels.len() <= 64);
        }
        assert!(!channels.contains_key("replica-0"));
        assert!(channels.contains_key("replica-255"));
    }
}
