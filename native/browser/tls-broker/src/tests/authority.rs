use super::fixture::*;
use crate::{Bootstrap, BrokerError, BrokerLease};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[tokio::test]
async fn normal_tls_and_exact_host_reach_pinned_upstream_without_proxy_credentials() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    let server = fixture.response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nVERIFIED".to_vec(),
    );
    let response = exchange(&broker, &origin, &request(&origin)).await;
    assert!(response.ends_with(b"VERIFIED"));
    let observed = String::from_utf8(server.await.unwrap()).unwrap();
    assert!(observed.starts_with("GET /verified HTTP/1.1\r\n"));
    assert!(
        !observed
            .to_ascii_lowercase()
            .contains("proxy-authorization")
    );
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn decrypted_cross_origin_host_is_denied_before_upstream_connect() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    for host in [
        "foreign.example",
        "localhost:443",
        "localhost/path",
        "user@localhost",
        "localhost\\other",
    ] {
        let response = exchange(
            &broker,
            &origin,
            format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(response.is_empty(), "accepted {host}");
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn connect_authority_and_sni_cannot_borrow_an_admitted_origin() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    assert!(connect(&broker, &origin, "foreign.example").await.is_err());
    let mut client = TcpStream::connect(broker.address()).await.unwrap();
    client.write_all(format!("CONNECT foreign.example:443 HTTP/1.1\r\nHost: foreign.example:443\r\nProxy-Authorization: {}\r\n\r\n", authorization(&broker)).as_bytes()).await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn unknown_proxy_credential_cannot_open_tls_or_an_upstream_socket() {
    let fixture = Fixture::new("localhost", false).await;
    let mut broker = fixture.broker().await;
    let authority = fixture
        .origin()
        .strip_prefix("https://")
        .unwrap()
        .to_owned();
    let mut client = TcpStream::connect(broker.address()).await.unwrap();
    client.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic d3Jvbmc=\r\n\r\n").as_bytes()).await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 407"));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn bootstrap_rejects_noncanonical_origins_unpinned_ips_and_invalid_roots() {
    let fixture = Fixture::new("localhost", false).await;
    let address = fixture.listener.local_addr().unwrap();
    for origin in [
        "http://localhost",
        "https://localhost/",
        "https://LOCALHOST",
        "https://localhost/path",
        "https://localhost?query",
        "https://127.0.0.2",
    ] {
        assert_eq!(
            BrokerLease::start(
                Bootstrap {
                    origin: origin.into(),
                    destinations: vec![address],
                    ca_der: vec![fixture.root.clone()],
                    generation: GENERATION
                },
                limits()
            )
            .await
            .unwrap_err(),
            BrokerError::InvalidBootstrap
        );
    }
    for generation in [[0; 32], GENERATION] {
        assert_eq!(
            BrokerLease::start(
                Bootstrap {
                    origin: fixture.origin(),
                    destinations: vec![address],
                    ca_der: vec![b"invalid DER".to_vec()],
                    generation
                },
                limits()
            )
            .await
            .unwrap_err(),
            BrokerError::InvalidBootstrap
        );
    }
}
