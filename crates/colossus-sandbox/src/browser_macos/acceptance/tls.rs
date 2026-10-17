//! Actual independently admitted native TLS broker for the owned-process fixture.

use super::startup::Startup;
use super::{Installation, MacosBrowserTlsConfig, validate_tls};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinChild, DarwinDirectChildCleanup, DarwinProcessIdentity,
    spawn_suspended_parent_channel,
};
use colossus_native_browser_tls_broker::{
    Bootstrap, BootstrapEnvelope, BrokerClient, BrokerLimits, BrokerReady,
};
use colossus_ports::BrowserDriverError;
use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier as _},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest as _, Sha256};
use std::{ffi::OsString, fs::File, io::Read as _, sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt as _;
use zeroize::Zeroizing;

/// Private proxy authentication plus the public exact-leaf bootstrap only.
pub(super) struct HostBootstrap {
    pub port: u16,
    pub credential: Zeroizing<String>,
    pub pin: super::launch::BrokerTls,
}

pub(super) struct Owner {
    config: MacosBrowserTlsConfig,
    executable: std::path::PathBuf,
    port: u16,
    limits: BrokerLimits,
    attempted: bool,
    child: Option<DarwinChild>,
    peer: Option<DarwinAuditPeer>,
    client: Option<BrokerClient>,
    reaped: Option<DarwinDirectChildCleanup>,
    acknowledged: bool,
    drains: Vec<tokio::task::JoinHandle<std::io::Result<u64>>>,
    output_failed: bool,
    closed: bool,
    pki_selection: Option<colossus_native_browser_tls_broker::PkiSelection>,
    custody_binding: Option<super::MacosBrowserCustodyRetirementReceipt>,
}

impl Owner {
    pub(super) fn new(
        installation: &Installation,
        request: &colossus_ports::BrowserDriverOpenRequest,
    ) -> Result<Self, BrowserDriverError> {
        let config = installation
            .tls_config
            .clone()
            .ok_or(BrowserDriverError::Denied)?;
        validate_tls(&config)?;
        let pki_selection = installation
            .pki_registry
            .as_ref()
            .map(|registry| {
                registry
                    .enroll(request)?
                    .ok_or(BrowserDriverError::Denied)?
                    .tls()
            })
            .transpose()?;
        let limits = BrokerLimits {
            connections: installation.egress.max_connections,
            lifetime: installation.egress.lifetime,
            exchange_timeout: installation.egress.connection_timeout,
            exchange_bytes: installation
                .egress
                .max_connection_bytes
                .try_into()
                .map_err(|_| BrowserDriverError::LimitExceeded)?,
        };
        if !(1..=16).contains(&limits.connections)
            || limits.lifetime.is_zero()
            || limits.lifetime > Duration::from_secs(300)
            || limits.exchange_timeout.is_zero()
            || limits.exchange_timeout > limits.lifetime
            || limits.exchange_bytes < 64 * 1024
            || limits.exchange_bytes > 64 * 1024 * 1024
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(Self {
            config,
            executable: installation
                .app
                .join("Contents/MacOS/colossus-native-browser-tls-broker"),
            port: installation.proxy_port,
            limits,
            attempted: false,
            child: None,
            peer: None,
            client: None,
            reaped: None,
            acknowledged: false,
            drains: Vec::new(),
            output_failed: false,
            closed: false,
            pki_selection,
            custody_binding: None,
        })
    }

    /// This owner must already be retained in sessionResources before this call.
    pub(super) async fn start(
        &mut self,
        environment: &[OsString],
        startup: &Startup<'_>,
    ) -> Result<HostBootstrap, BrowserDriverError> {
        startup.check()?;
        if self.attempted || self.closed {
            return Err(BrowserDriverError::Denied);
        }
        self.attempted = true;
        let spawned = spawn_suspended_parent_channel(&self.executable, &[], environment)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let pid = spawned.child.pid();
        // Retain before identity/signature checks, output setup or any await.
        self.child = Some(spawned.child);
        self.drain(spawned.stdout);
        self.drain(spawned.stderr);
        let identity = DarwinProcessIdentity::bind(pid).map_err(|_| BrowserDriverError::Denied)?;
        self.peer = Some(
            DarwinAuditPeer::enroll(&identity, self.config.broker_cdhash)
                .map_err(|_| BrowserDriverError::Denied)?,
        );
        self.peer
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?
            .verify()
            .map_err(|_| BrowserDriverError::Denied)?;
        startup.check()?;
        self.child
            .as_mut()
            .ok_or(BrowserDriverError::OutcomeUnknown)?
            .resume()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        startup.check()?;
        spawned
            .channel
            .set_nonblocking(true)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let mut stream = tokio::net::UnixStream::from_std(spawned.channel)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let mut key = Zeroizing::new([0; 32]);
        let mut generation = [0; 32];
        for _ in 0..4 {
            getrandom::fill(key.as_mut()).map_err(|_| BrowserDriverError::Unavailable)?;
            getrandom::fill(&mut generation).map_err(|_| BrowserDriverError::Unavailable)?;
            if *key != [0; 32] && generation != [0; 32] && *key != generation {
                break;
            }
        }
        if *key == [0; 32] || generation == [0; 32] || *key == generation {
            return Err(BrowserDriverError::Unavailable);
        }
        if self
            .pki_selection
            .as_ref()
            .is_some_and(|selection| selection.client.is_some())
        {
            let parent = DarwinProcessIdentity::bind(std::process::id())
                .map_err(|_| BrowserDriverError::Denied)?;
            let child = self
                .peer
                .as_ref()
                .ok_or(BrowserDriverError::OutcomeUnknown)?
                .identity();
            let generation_sha256: [u8; 32] = Sha256::digest(generation).into();
            self.custody_binding = Some(super::MacosBrowserCustodyRetirementReceipt {
                generation_sha256,
                broker_cdhash: self.config.broker_cdhash,
                broker_pid: child.pid(),
                broker_generation_binding_sha256: child
                    .custody_generation_binding(&generation_sha256),
                parent_generation_binding_sha256: parent
                    .custody_generation_binding(&generation_sha256),
                proxy_port: self.port,
            });
        }
        let mut entry = Zeroizing::new([0; 64]);
        entry[..32].copy_from_slice(key.as_ref());
        entry[32..].copy_from_slice(&generation);
        // The private64bytes are sent only after independently admitted child
        // code and exact retained direct-child identity. Native entry separately
        // verifies independently sealed Core before reading these bytes.
        self.peer
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?
            .verify()
            .map_err(|_| BrowserDriverError::Denied)?;
        startup.check()?;
        tokio::time::timeout(Duration::from_secs(5), stream.write_all(entry.as_ref()))
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        startup.check()?;
        self.client = Some(
            BrokerClient::from_admitted_stream(stream, key, generation)
                .map_err(|_| BrowserDriverError::Denied)?,
        );
        let admitted = tokio::time::Instant::now();
        let envelope = BootstrapEnvelope {
            bootstrap: Bootstrap {
                origin: self.config.origin.as_str().into(),
                destinations: self.config.destinations.clone(),
                ca_der: self.config.public_roots.clone(),
                generation,
            },
            limits: self.limits,
            port: self.port,
        };
        let client = self
            .client
            .as_mut()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        let ready = match self.pki_selection.clone() {
            Some(selection) => client.bootstrap_with_pki(envelope, selection).await,
            None => client.bootstrap(envelope).await,
        }
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        startup.check()?;
        self.peer
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?
            .verify()
            .map_err(|_| BrowserDriverError::Denied)?;
        validate_ready(&ready, &self.config, self.port)?;
        let lifetime_ms: u32 = self
            .limits
            .lifetime
            .saturating_sub(admitted.elapsed())
            .as_millis()
            .try_into()
            .map_err(|_| BrowserDriverError::LimitExceeded)?;
        if lifetime_ms == 0 {
            return Err(BrowserDriverError::Denied);
        }
        Ok(HostBootstrap {
            port: ready.port(),
            credential: Zeroizing::new(ready.credential().into()),
            pin: super::launch::BrokerTls {
                origin: self.config.origin.clone(),
                leaf_der: ready.leaf_der().to_vec(),
                leaf_sha256: *ready.leaf_sha256(),
                lifetime_ms,
            },
        })
    }

    pub(super) async fn close(&mut self) -> Result<(), BrowserDriverError> {
        if self.closed {
            return Ok(());
        }
        let mut unknown = false;
        if !self.acknowledged
            && let Some(client) = self.client.as_mut()
        {
            if client.shutdown().await.is_ok() {
                self.acknowledged = true;
            } else {
                unknown = true;
            }
        }
        self.client.take(); // EOF independently fences/drains an interrupted broker.
        if let Some(child) = self.child.as_mut() {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(10)).await
                    }
                    Ok(None) => break,
                    Err(_) => {
                        unknown = true;
                        break;
                    }
                }
            }
            match child.terminate_and_reap(Duration::from_secs(2)) {
                Ok(receipt) => {
                    eprintln!(
                        "Mac diagnostic TLS cleanup: native_exit={:?}, forced={}, sockets_acknowledged={}",
                        receipt.status().code(),
                        receipt.forced(),
                        self.acknowledged
                    );
                    self.reaped = Some(receipt);
                    self.child.take();
                }
                Err(_) => unknown = true,
            }
        }
        let mut index = 0;
        while index < self.drains.len() {
            match tokio::time::timeout(Duration::from_secs(2), &mut self.drains[index]).await {
                Ok(Ok(Ok(_))) => {
                    drop(self.drains.remove(index));
                }
                Ok(_) => {
                    self.output_failed = true;
                    drop(self.drains.remove(index));
                }
                Err(_) => {
                    unknown = true;
                    index += 1;
                }
            }
        }
        if self.attempted
            && (!self.acknowledged
                || self
                    .reaped
                    .is_none_or(|receipt| !receipt.status().success() || receipt.forced()))
        {
            unknown = true;
        }
        if unknown || self.output_failed {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.closed = true;
        Ok(())
    }

    fn drain(&mut self, mut file: File) {
        self.drains.push(tokio::task::spawn_blocking(move || {
            let mut bytes = [0; 8192];
            let mut count = 0u64;
            loop {
                let read = match file.read(&mut bytes) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    result => result?,
                };
                if read == 0 {
                    return Ok(count);
                }
                count = count.saturating_add(read as u64);
            }
        }));
    }

    pub(super) fn retirement_receipt(&self) -> Option<super::MacosBrowserCustodyRetirementReceipt> {
        (self.closed
            && self.acknowledged
            && !self.output_failed
            && self.drains.is_empty()
            && self
                .reaped
                .as_ref()
                .is_some_and(|receipt| receipt.status().success() && !receipt.forced()))
        .then_some(self.custody_binding)
        .flatten()
    }
}

fn validate_ready(
    ready: &BrokerReady,
    config: &MacosBrowserTlsConfig,
    port: u16,
) -> Result<(), BrowserDriverError> {
    if ready.port() != port
        || ready.leaf_der().is_empty()
        || ready.leaf_der().len() > 8192
        || ready.leaf_sha256() == &[0; 32]
        || <[u8; 32]>::from(Sha256::digest(ready.leaf_der())) != *ready.leaf_sha256()
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(ready.ca_der().to_vec()))
        .map_err(|_| BrowserDriverError::Denied)?;
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .map_err(|_| BrowserDriverError::Denied)?;
    validate_tls(config)?;
    let name = ServerName::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST).into());
    verifier
        .verify_server_cert(
            &CertificateDer::from(ready.leaf_der()),
            &[],
            &name,
            &[],
            UnixTime::now(),
        )
        .map_err(|_| BrowserDriverError::Denied)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn authenticated_leaf_is_checked_with_normal_ca_and_hostname_verification() {
        use colossus_native_browser_tls_broker::{ControlExitReason, DrainEvidence, serve_control};
        let mut params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let root = params.self_signed(&key).unwrap().der().to_vec();
        let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reserve.local_addr().unwrap().port();
        drop(reserve);
        let generation = [3; 32];
        let config = MacosBrowserTlsConfig {
            broker_cdhash: [1; 20],
            origin: colossus_contracts::BrowserOrigin::parse("https://127.0.0.1:60924").unwrap(),
            destinations: vec!["127.0.0.1:60924".parse().unwrap()],
            public_roots: vec![root.clone()],
            pki_registry_sha256: None,
        };
        let (owner, server) = tokio::net::UnixStream::pair().unwrap();
        let service = tokio::spawn(serve_control(server, Zeroizing::new([2; 32]), generation));
        let mut client =
            BrokerClient::from_admitted_stream(owner, Zeroizing::new([2; 32]), generation).unwrap();
        let ready = client
            .bootstrap(BootstrapEnvelope {
                bootstrap: Bootstrap {
                    origin: config.origin.as_str().into(),
                    destinations: config.destinations.clone(),
                    ca_der: vec![root],
                    generation,
                },
                limits: BrokerLimits {
                    connections: 2,
                    lifetime: Duration::from_secs(5),
                    exchange_timeout: Duration::from_secs(2),
                    exchange_bytes: 64 * 1024,
                },
                port,
            })
            .await
            .unwrap();
        validate_ready(&ready, &config, port).unwrap();
        assert_eq!(
            validate_ready(&ready, &config, port.saturating_sub(1)),
            Err(BrowserDriverError::Denied)
        );
        let mut wrong = config.clone();
        wrong.origin = colossus_contracts::BrowserOrigin::parse("https://localhost:60924").unwrap();
        assert_eq!(
            validate_ready(&ready, &wrong, port),
            Err(BrowserDriverError::Denied)
        );
        client.shutdown().await.unwrap();
        let receipt = service.await.unwrap();
        assert_eq!(receipt.reason, ControlExitReason::Shutdown);
        assert_eq!(receipt.drainage, DrainEvidence::TrackedSocketsClosed);
    }
}
