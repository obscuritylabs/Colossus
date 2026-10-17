use super::fixture::*;
use std::time::Duration;

#[tokio::test]
async fn fragmented_chunked_download_is_preserved_and_bounded() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    let payload = vec![b'd'; 128 * 1024];
    let mut chunked = format!("{:x}\r\n", payload.len()).into_bytes();
    chunked.extend_from_slice(&payload);
    chunked.extend_from_slice(b"\r\n3\r\nEND\r\n0\r\n\r\n");
    let mut response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n".to_vec();
    response.extend_from_slice(&chunked);
    let server = fixture.response(response);
    let received = exchange(&broker, &origin, &request(&origin)).await;
    assert!(received.ends_with(&chunked));
    assert!(
        received
            .windows(b"Connection: close".len())
            .any(|bytes| bytes == b"Connection: close")
    );
    server.await.unwrap();
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn ambiguous_bodies_duplicate_authority_upgrades_and_pipelining_are_not_forwarded() {
    let fixture = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    let host = origin.strip_prefix("https://").unwrap();
    for extra in [
        "Content-Length: 0\r\nTransfer-Encoding: chunked\r\n",
        "Content-Length: 0\r\nContent-Length: 1\r\n",
        "Host: foreign.example\r\n",
        "Upgrade: websocket\r\nConnection: Upgrade\r\n",
        "Connection: X-Authority\r\nX-Authority: foreign.example\r\n",
        "Content-Length: 10000000\r\n",
    ] {
        let request = format!("POST / HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n");
        assert!(
            exchange(&broker, &origin, request.as_bytes())
                .await
                .is_empty()
        );
    }
    let mut pipeline = request(&origin);
    pipeline.extend_from_slice(b"GET / HTTP/1.1\r\nHost: foreign.example\r\n\r\n");
    assert!(exchange(&broker, &origin, &pipeline).await.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.listener.accept())
            .await
            .is_err()
    );
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn ambiguous_malformed_or_oversized_upstream_responses_are_quarantined() {
    for response in [
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\nOK".to_vec(),
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nOKX\r\n0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nX-Trailer: hidden\r\n\r\n"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n".to_vec(),
    ] {
        let fixture = Fixture::new("localhost", false).await;
        let origin = fixture.origin();
        let mut broker = fixture.broker().await;
        let server = fixture.response(response);
        assert!(
            exchange(&broker, &origin, &request(&origin))
                .await
                .is_empty()
        );
        server.await.unwrap();
        broker.revoke().await.unwrap();
    }
}
