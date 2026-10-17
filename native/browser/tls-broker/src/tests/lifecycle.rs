use super::fixture::*;
use crate::{Bootstrap, BrokerLease, BrokerState};
use sha2::{Digest as _, Sha256};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[tokio::test]
async fn generation_rotation_and_revocation_fence_private_bootstrap_material() {
    let fixture = Fixture::new("localhost", false).await;
    let mut first = fixture.broker().await;
    let second = fixture.broker().await;
    assert!(first.credential(&[43; 32]).is_none());
    assert!(first.public_ca(&[43; 32]).is_none());
    assert!(first.public_leaf(&[43; 32]).is_none());
    assert!(first.public_leaf_sha256(&[43; 32]).is_none());
    assert_ne!(
        first.credential(&GENERATION),
        second.credential(&GENERATION)
    );
    assert_ne!(first.public_ca(&GENERATION), second.public_ca(&GENERATION));
    assert_ne!(
        first.public_leaf(&GENERATION),
        second.public_leaf(&GENERATION)
    );
    let tls = connect(&first, &fixture.origin(), "localhost")
        .await
        .unwrap();
    let actual_leaf = tls.get_ref().1.peer_certificates().unwrap()[0].as_ref();
    assert_eq!(first.public_leaf(&GENERATION).unwrap(), actual_leaf);
    let actual_sha256: [u8; 32] = Sha256::digest(actual_leaf).into();
    assert_eq!(first.public_leaf_sha256(&GENERATION), Some(&actual_sha256));
    drop(tls);
    let mut socket = TcpStream::connect(second.address()).await.unwrap();
    let authority = fixture
        .origin()
        .strip_prefix("https://")
        .unwrap()
        .to_owned();
    socket.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: {}\r\n\r\n", authorization(&first)).as_bytes()).await.unwrap();
    let mut response = Vec::new();
    socket.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 407"));
    first.revoke().await.unwrap();
    assert!(first.credential(&GENERATION).is_none());
    assert!(first.public_ca(&GENERATION).is_none());
    assert!(first.public_leaf(&GENERATION).is_none());
    assert!(first.public_leaf_sha256(&GENERATION).is_none());
    assert_eq!(first.state(), BrokerState::Revoked);
    first.revoke().await.unwrap();
    let mut second = second;
    second.revoke().await.unwrap();
}

#[tokio::test]
async fn sealed_port_serves_only_both_loopbacks_and_revoke_drains_both() {
    use std::net::{Ipv4Addr, Ipv6Addr};
    let fixture = Fixture::new("localhost", false).await;
    let reservation = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut broker = BrokerLease::start_on_loopbacks(
        Bootstrap {
            origin: fixture.origin(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root],
            generation: GENERATION,
        },
        limits(),
        port,
    )
    .await
    .unwrap();
    assert_eq!(broker.address(), (Ipv4Addr::LOCALHOST, port).into());
    let mut v4 = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let mut v6 = TcpStream::connect((Ipv6Addr::LOCALHOST, port))
        .await
        .unwrap();
    v4.write_all(b"CONNECT ").await.unwrap();
    v6.write_all(b"CONNECT ").await.unwrap();
    broker.revoke().await.unwrap();
    let mut byte = [0; 1];
    assert!(!matches!(v4.read(&mut byte).await, Ok(1)));
    assert!(!matches!(v6.read(&mut byte).await, Ok(1)));
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
async fn sealed_port_collision_closes_partial_listener_and_never_falls_back() {
    use std::net::{Ipv4Addr, Ipv6Addr};
    let fixture = Fixture::new("localhost", false).await;
    let reservation = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = reservation.local_addr().unwrap().port();
    let result = BrokerLease::start_on_loopbacks(
        Bootstrap {
            origin: fixture.origin(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root],
            generation: GENERATION,
        },
        limits(),
        port,
    )
    .await;
    assert!(matches!(result, Err(crate::BrokerError::Unavailable)));
    let v6 = tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, port))
        .await
        .unwrap();
    drop(v6);
    drop(reservation);
}

#[tokio::test]
async fn sealed_port_rejects_zero_before_allocating_any_listener() {
    let result = BrokerLease::start_on_loopbacks(
        Bootstrap {
            origin: "https://localhost".into(),
            destinations: vec![],
            ca_der: vec![],
            generation: GENERATION,
        },
        limits(),
        0,
    )
    .await;
    assert!(matches!(result, Err(crate::BrokerError::InvalidBootstrap)));
}

#[tokio::test]
async fn revoke_drains_incomplete_connect_and_decrypted_body_sockets() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    let mut incomplete = TcpStream::connect(broker.address()).await.unwrap();
    incomplete.write_all(b"CONNECT ").await.unwrap();
    let mut tls = connect(&broker, &origin, "localhost").await.unwrap();
    tls.write_all(
        format!(
            "POST / HTTP/1.1\r\nHost: {}\r\nContent-Length: 1000\r\n\r\npartial",
            origin.strip_prefix("https://").unwrap()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    broker.revoke().await.unwrap();
    assert_eq!(broker.state(), BrokerState::Revoked);
    let mut byte = [0_u8; 1];
    assert!(!matches!(incomplete.read(&mut byte).await, Ok(1)));
    assert!(!matches!(tls.read(&mut byte).await, Ok(1)));
    assert!(TcpStream::connect(broker.address()).await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn immutable_lifetime_revokes_background_connections_without_renewal() {
    let fixture = Fixture::new("localhost", false).await;
    let mut short = limits();
    short.lifetime = Duration::from_millis(80);
    short.exchange_timeout = short.lifetime;
    let mut broker = BrokerLease::start(
        Bootstrap {
            origin: fixture.origin(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root],
            generation: GENERATION,
        },
        short,
    )
    .await
    .unwrap();
    let mut client = TcpStream::connect(broker.address()).await.unwrap();
    client.write_all(b"C").await.unwrap();
    let mut byte = [0_u8; 1];
    let result = tokio::time::timeout(Duration::from_secs(1), client.read(&mut byte))
        .await
        .unwrap();
    assert!(!matches!(result, Ok(1)));
    broker.revoke().await.unwrap();
    assert_eq!(broker.state(), BrokerState::Revoked);
}
