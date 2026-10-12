use super::fixture::*;
use crate::{
    Bootstrap, BootstrapEnvelope, BrokerClient, BrokerReady, BrokerState, ControlExitReason,
    DrainEvidence, serve_control,
};
use base64::Engine as _;
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use sha2::{Digest as _, Sha256};
use std::{
    net::{Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream, UnixStream},
};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

const KEY: [u8; 32] = [19; 32];

async fn envelope(fixture: &Fixture) -> BootstrapEnvelope {
    let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    BootstrapEnvelope {
        bootstrap: Bootstrap {
            origin: fixture.origin(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root.clone()],
            generation: GENERATION,
        },
        limits: limits(),
        port,
    }
}

async fn tls(ready: &BrokerReady, origin: &str) -> tokio_rustls::client::TlsStream<TcpStream> {
    let mut stream = TcpStream::connect((Ipv6Addr::LOCALHOST, ready.port()))
        .await
        .unwrap();
    let authority = origin.strip_prefix("https://").unwrap();
    let secret = Zeroizing::new(format!("colossus:{}", ready.credential()));
    let authorization =
        Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(secret.as_bytes()));
    let request = Zeroizing::new(format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic {}\r\n\r\n",
        authorization.as_str()
    ));
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = [0; 39];
    stream.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"HTTP/1.1 200 Connection Established\r\n\r\n");
    let mut roots = RootCertStore::empty();
    roots.add(ready.ca_der().to_vec().into()).unwrap();
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), stream)
        .await
        .unwrap()
}

#[tokio::test]
async fn private_control_enrollment_serves_verified_tls_and_acknowledges_both_listener_drainage() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let bootstrap = envelope(&fixture).await;
    let port = bootstrap.port;
    let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
    let server = tokio::spawn(serve_control(
        broker_stream,
        Zeroizing::new(KEY),
        GENERATION,
    ));
    let mut owner =
        BrokerClient::from_admitted_stream(owner_stream, Zeroizing::new(KEY), GENERATION).unwrap();
    let ready = owner.bootstrap(bootstrap).await.unwrap();
    assert_eq!(ready.port(), port);
    assert!(!format!("{ready:?}").contains(ready.credential()));
    assert_eq!(owner.status().await.unwrap(), BrokerState::Active);
    let upstream =
        fixture.response(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nverified".to_vec());
    let mut page = tls(&ready, &origin).await;
    let leaf = page.get_ref().1.peer_certificates().unwrap()[0].as_ref();
    assert_eq!(ready.leaf_der(), leaf);
    let pin: [u8; 32] = Sha256::digest(leaf).into();
    assert_eq!(ready.leaf_sha256(), &pin);
    page.write_all(&request(&origin)).await.unwrap();
    let mut response = Vec::new();
    page.read_to_end(&mut response).await.unwrap();
    assert!(response.ends_with(b"verified"));
    let received = upstream.await.unwrap();
    assert!(!String::from_utf8_lossy(&received).contains(ready.credential()));
    let mut incomplete_v4 = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let mut incomplete_v6 = TcpStream::connect((Ipv6Addr::LOCALHOST, port))
        .await
        .unwrap();
    incomplete_v4.write_all(b"CONNECT ").await.unwrap();
    incomplete_v6.write_all(b"CONNECT ").await.unwrap();
    owner.revoke().await.unwrap();
    assert_eq!(owner.status().await.unwrap(), BrokerState::Revoked);
    owner.revoke().await.unwrap();
    owner.shutdown().await.unwrap();
    let exit = server.await.unwrap();
    assert_eq!(exit.reason, ControlExitReason::Shutdown);
    assert_eq!(exit.drainage, DrainEvidence::TrackedSocketsClosed);
    for stream in [&mut incomplete_v4, &mut incomplete_v6] {
        let mut byte = [0; 1];
        assert!(!matches!(stream.read(&mut byte).await, Ok(1)));
    }
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

#[tokio::test]
async fn owner_eof_revokes_all_tls_sockets_without_claiming_process_cleanup() {
    let fixture = Fixture::new("localhost", false).await;
    let bootstrap = envelope(&fixture).await;
    let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
    let server = tokio::spawn(serve_control(
        broker_stream,
        Zeroizing::new(KEY),
        GENERATION,
    ));
    let mut owner =
        BrokerClient::from_admitted_stream(owner_stream, Zeroizing::new(KEY), GENERATION).unwrap();
    let ready = owner.bootstrap(bootstrap).await.unwrap();
    let mut page = tls(&ready, &fixture.origin()).await;
    page.write_all(b"GET ").await.unwrap();
    drop(owner);
    let exit = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit.reason, ControlExitReason::ControlClosed);
    assert_eq!(exit.drainage, DrainEvidence::TrackedSocketsClosed);
    let mut byte = [0; 1];
    assert!(!matches!(page.read(&mut byte).await, Ok(1)));
    assert!(
        TcpStream::connect((Ipv4Addr::LOCALHOST, ready.port()))
            .await
            .is_err()
    );
    assert!(
        TcpStream::connect((Ipv6Addr::LOCALHOST, ready.port()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn private_control_absolute_expiry_drains_and_cannot_be_renewed_by_status() {
    let fixture = Fixture::new("localhost", false).await;
    let mut bootstrap = envelope(&fixture).await;
    // X.509 validity is encoded at whole-second precision. Keep admission long
    // enough for normal CA validation at a second boundary; the observed expiry
    // still comes from this immutable lease, without a test sleep or renewal.
    bootstrap.limits.lifetime = Duration::from_secs(2);
    bootstrap.limits.exchange_timeout = Duration::from_secs(1);
    let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
    let server = tokio::spawn(serve_control(
        broker_stream,
        Zeroizing::new(KEY),
        GENERATION,
    ));
    let mut owner =
        BrokerClient::from_admitted_stream(owner_stream, Zeroizing::new(KEY), GENERATION).unwrap();
    let ready = owner.bootstrap(bootstrap).await.unwrap();
    assert_eq!(owner.status().await.unwrap(), BrokerState::Active);
    let mut partial = TcpStream::connect((Ipv4Addr::LOCALHOST, ready.port()))
        .await
        .unwrap();
    partial.write_all(b"C").await.unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(4), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit.reason, ControlExitReason::Expired);
    assert_eq!(exit.drainage, DrainEvidence::TrackedSocketsClosed);
    assert!(owner.status().await.is_err());
    assert!(
        TcpStream::connect((Ipv6Addr::LOCALHOST, ready.port()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn wrong_channel_key_or_generation_allocates_no_listener() {
    let fixture = Fixture::new("localhost", false).await;
    for (key, generation) in [([20; 32], GENERATION), (KEY, [43; 32])] {
        let mut bootstrap = envelope(&fixture).await;
        let port = bootstrap.port;
        bootstrap.bootstrap.generation = generation;
        let (owner_stream, broker_stream) = UnixStream::pair().unwrap();
        let server = tokio::spawn(serve_control(
            broker_stream,
            Zeroizing::new(KEY),
            GENERATION,
        ));
        let mut owner =
            BrokerClient::from_admitted_stream(owner_stream, Zeroizing::new(key), generation)
                .unwrap();
        assert!(owner.bootstrap(bootstrap).await.is_err());
        let exit = server.await.unwrap();
        assert_eq!(exit.reason, ControlExitReason::Rejected);
        assert_eq!(exit.drainage, DrainEvidence::NoLeaseAllocated);
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
