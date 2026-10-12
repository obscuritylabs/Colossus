use super::fixture::*;
use crate::{Bootstrap, BrokerLease, BrokerState};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[tokio::test]
async fn accepted_tls_handshakes_consume_the_connection_bound() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut bound = limits();
    bound.connections = 1;
    let mut broker = BrokerLease::start(
        Bootstrap {
            origin: origin.clone(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root],
            generation: GENERATION,
        },
        bound,
    )
    .await
    .unwrap();
    let mut first = connect(&broker, &origin, "localhost").await.unwrap();
    first.write_all(b"GET ").await.unwrap();
    let mut excess = TcpStream::connect(broker.address()).await.unwrap();
    let _ = excess.write_all(b"CONNECT ").await;
    let mut byte = [0_u8; 1];
    assert!(!matches!(
        tokio::time::timeout(Duration::from_secs(1), excess.read(&mut byte))
            .await
            .unwrap(),
        Ok(1)
    ));
    broker.revoke().await.unwrap();
    assert_eq!(broker.state(), BrokerState::Revoked);
    assert!(!matches!(first.read(&mut byte).await, Ok(1)));
}

#[tokio::test]
async fn exchange_deadline_closes_a_slow_decrypted_body_before_upstream() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut bound = limits();
    bound.exchange_timeout = Duration::from_millis(250);
    let mut broker = BrokerLease::start(
        Bootstrap {
            origin: origin.clone(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root],
            generation: GENERATION,
        },
        bound,
    )
    .await
    .unwrap();
    let mut client = connect(&broker, &origin, "localhost").await.unwrap();
    client
        .write_all(
            format!(
                "POST / HTTP/1.1\r\nHost: {}\r\nContent-Length: 100\r\n\r\none",
                origin.strip_prefix("https://").unwrap()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut byte = [0_u8; 1];
    assert!(!matches!(
        tokio::time::timeout(Duration::from_secs(1), client.read(&mut byte))
            .await
            .unwrap(),
        Ok(1)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn byte_budget_covers_chunk_framing_and_request_header_together_with_body() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut bound = limits();
    bound.exchange_bytes = 64 * 1024;
    let mut broker = BrokerLease::start(
        Bootstrap {
            origin: origin.clone(),
            destinations: vec![fixture.listener.local_addr().unwrap()],
            ca_der: vec![fixture.root.clone()],
            generation: GENERATION,
        },
        bound,
    )
    .await
    .unwrap();
    let mut oversized = format!(
        "POST / HTTP/1.1\r\nHost: {}\r\nContent-Length: 65536\r\n\r\n",
        origin.strip_prefix("https://").unwrap()
    )
    .into_bytes();
    oversized.resize(oversized.len() + 65536, b'x');
    assert!(exchange(&broker, &origin, &oversized).await.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
    let server = fixture
        .response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10000\r\n".to_vec());
    assert!(
        exchange(&broker, &origin, &request(&origin))
            .await
            .is_empty()
    );
    server.await.unwrap();
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn cancelled_revoke_wait_retains_drainage_ownership_for_retry() {
    use std::{
        future::{Future as _, poll_fn},
        pin::pin,
        task::Poll,
    };
    let fixture = Fixture::new("localhost", false).await;
    let mut broker = fixture.broker().await;
    let mut incomplete = TcpStream::connect(broker.address()).await.unwrap();
    incomplete.write_all(b"CONNECT ").await.unwrap();
    {
        let mut revoke = pin!(broker.revoke());
        poll_fn(|context| {
            let _ = revoke.as_mut().poll(context);
            Poll::Ready(())
        })
        .await;
    }
    broker.revoke().await.unwrap();
    assert_eq!(broker.state(), BrokerState::Revoked);
    let mut byte = [0_u8; 1];
    assert!(!matches!(incomplete.read(&mut byte).await, Ok(1)));
}
