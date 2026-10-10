use super::*;
use base64::Engine as _;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

fn limits() -> BrowserEgressLimits {
    BrowserEgressLimits {
        max_connections: 4,
        lifetime: Duration::from_secs(10),
        connection_timeout: Duration::from_secs(5),
        max_connection_bytes: 64 * 1024,
    }
}

async fn lease(origins: Vec<BrowserOrigin>, limits: BrowserEgressLimits) -> BrowserEgressLease {
    BrowserEgressLease::start(
        BrowserSessionId::parse(format!("bs_{}", uuid::Uuid::now_v7().simple())).unwrap(),
        origins,
        limits,
    )
    .await
    .unwrap()
}

fn auth(lease: &BrowserEgressLease) -> String {
    format!(
        "Proxy-Authorization: Basic {}\r\n",
        crate::BASE64.encode(format!("colossus:{}", lease.credential().expose()))
    )
}

async fn exchange(lease: &BrowserEgressLease, request: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(lease.address()).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    response
}

#[tokio::test]
async fn origin_envelope_and_native_secret_are_closed() {
    let origin = BrowserOrigin::parse("https://allowed.example").unwrap();
    for origins in [
        Vec::new(),
        vec![origin.clone(), origin],
        vec![BrowserOrigin::parse("http://public.example").unwrap()],
    ] {
        assert_eq!(
            BrowserEgressLease::start(
                BrowserSessionId::parse(format!("bs_{}", uuid::Uuid::now_v7().simple())).unwrap(),
                origins,
                limits()
            )
            .await
            .unwrap_err(),
            BrowserEgressError::InvalidConfiguration
        );
    }
    let mut proxy = lease(
        vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
        limits(),
    )
    .await;
    assert_eq!(proxy.credential().expose().len(), 64);
    assert!(!format!("{proxy:?}").contains(proxy.credential().expose()));
    let unauthenticated = exchange(
        &proxy,
        "CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example\r\n\r\n",
    )
    .await;
    assert!(unauthenticated.starts_with(b"HTTP/1.1 407"));
    let denied = exchange(
        &proxy,
        &format!(
            "CONNECT denied.example:443 HTTP/1.1\r\nHost: denied.example\r\n{}\r\n",
            auth(&proxy)
        ),
    )
    .await;
    assert!(denied.starts_with(b"HTTP/1.1 403"));
    for authority in [
        "allowed.example:443@denied.example",
        "allowed.example:443/path",
        "allowed.example:443?query",
    ] {
        let response = exchange(
            &proxy,
            &format!(
                "CONNECT {authority} HTTP/1.1\r\nHost: allowed.example\r\n{}\r\n",
                auth(&proxy)
            ),
        )
        .await;
        assert!(response.is_empty());
    }
    proxy.revoke().await.unwrap();
}

#[tokio::test]
async fn session_credentials_cannot_authenticate_another_envelope() {
    let origins = vec![BrowserOrigin::parse("https://allowed.example").unwrap()];
    let mut first = lease(origins.clone(), limits()).await;
    let mut second = lease(origins, limits()).await;
    assert_ne!(first.credential().expose(), second.credential().expose());
    let response = exchange(
        &second,
        &format!(
            "CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example\r\n{}\r\n",
            auth(&first)
        ),
    )
    .await;
    assert!(response.starts_with(b"HTTP/1.1 407"));
    first.revoke().await.unwrap();
    second.revoke().await.unwrap();
}

#[tokio::test]
async fn revoke_drains_stalled_handshakes_and_is_idempotent() {
    let mut proxy = lease(
        vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
        limits(),
    )
    .await;
    let mut stream = TcpStream::connect(proxy.address()).await.unwrap();
    stream
        .write_all(
            format!(
                "CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example\r\n{}\r\n",
                auth(&proxy)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut ready = vec![0_u8; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
    stream.read_exact(&mut ready).await.unwrap();
    assert!(ready.starts_with(b"HTTP/1.1 200"));
    proxy.revoke().await.unwrap();
    assert_eq!(proxy.state(), BrowserEgressState::Revoked);
    assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
    assert!(TcpStream::connect(proxy.address()).await.is_err());
    proxy.revoke().await.unwrap();
}

#[tokio::test]
async fn deadline_reaps_background_connections_without_renewal() {
    let mut bounded = limits();
    bounded.lifetime = Duration::from_millis(80);
    bounded.connection_timeout = bounded.lifetime;
    let mut proxy = lease(
        vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
        bounded,
    )
    .await;
    let mut stalled = TcpStream::connect(proxy.address()).await.unwrap();
    stalled.write_all(b"CON").await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stalled.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    proxy.revoke().await.unwrap();
    assert_eq!(proxy.state(), BrowserEgressState::Revoked);
    assert!(response.is_empty());
    assert!(TcpStream::connect(proxy.address()).await.is_err());
}

#[tokio::test]
async fn live_connection_limit_includes_unfinished_requests() {
    let mut bounded = limits();
    bounded.max_connections = 1;
    let mut proxy = lease(
        vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
        bounded,
    )
    .await;
    let mut held = TcpStream::connect(proxy.address()).await.unwrap();
    held.write_all(
        format!(
            "CONNECT allowed.example:443 HTTP/1.1\r\nHost: allowed.example\r\n{}\r\n",
            auth(&proxy)
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut ready = vec![0_u8; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
    held.read_exact(&mut ready).await.unwrap();
    let mut overflow = TcpStream::connect(proxy.address()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), overflow.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.is_empty());
    proxy.revoke().await.unwrap();
    assert_eq!(held.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn plaintext_exchange_strips_proxy_credentials_and_forces_connection_close() {
    let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = server.local_addr().unwrap();
    let origin = format!("http://{address}");
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = server.accept().await.unwrap();
        let mut captured = Vec::new();
        while !captured.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0_u8; 1024];
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            captured.extend_from_slice(&buffer[..count]);
        }
        let text = String::from_utf8(captured.clone()).unwrap();
        assert!(text.starts_with("GET /fixture?x=1 HTTP/1.1\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(!text.to_ascii_lowercase().contains("proxy-authorization"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        let mut extra = Vec::new();
        stream.read_to_end(&mut extra).await.unwrap();
        assert!(extra.is_empty());
        captured
    });
    let mut proxy = lease(vec![BrowserOrigin::parse(&origin).unwrap()], limits()).await;
    let mut stream = TcpStream::connect(proxy.address()).await.unwrap();
    stream.write_all(format!("GET {origin}/fixture?x=1 HTTP/1.1\r\nHost: {address}\r\nConnection: keep-alive\r\n{}\r\n", auth(&proxy)).as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    assert!(response.ends_with(b"OK"));
    assert!(
        !String::from_utf8(fixture.await.unwrap())
            .unwrap()
            .contains(proxy.credential().expose())
    );
    proxy.revoke().await.unwrap();
}

#[tokio::test]
async fn plaintext_rejects_header_smuggling_chunked_and_pipelined_requests_before_upstream() {
    let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = server.local_addr().unwrap();
    let origin = format!("http://{address}");
    let mut proxy = lease(vec![BrowserOrigin::parse(&origin).unwrap()], limits()).await;
    for headers in [
        "Transfer-Encoding: chunked\r\n",
        "Upgrade: websocket\r\n",
        "Content-Length: 0\r\nContent-Length: 1\r\n",
        "Content-Length: +1\r\n",
        "Host: denied.example\r\n",
        "X-Header: harmless\r\n folded: value\r\n",
    ] {
        let response = exchange(
            &proxy,
            &format!(
                "GET {origin}/ HTTP/1.1\r\nHost: {address}\r\n{}{headers}\r\n",
                auth(&proxy)
            ),
        )
        .await;
        assert!(response.is_empty());
    }
    let response = exchange(&proxy, &format!("GET {origin}/ HTTP/1.1\r\nHost: {address}\r\n{}\r\nGET {origin}/second HTTP/1.1\r\nHost: denied.example\r\n\r\n", auth(&proxy))).await;
    assert!(response.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), server.accept())
            .await
            .is_err()
    );
    proxy.revoke().await.unwrap();
}

#[tokio::test]
async fn revoke_drops_live_plaintext_upstream_socket() {
    let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = server.local_addr().unwrap();
    let origin = format!("http://{address}");
    let mut proxy = lease(vec![BrowserOrigin::parse(&origin).unwrap()], limits()).await;
    let mut browser = TcpStream::connect(proxy.address()).await.unwrap();
    browser
        .write_all(
            format!(
                "GET {origin}/ HTTP/1.1\r\nHost: {address}\r\n{}\r\n",
                auth(&proxy)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let (mut upstream, _) = server.accept().await.unwrap();
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0_u8; 1024];
        let count = upstream.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0);
        request.extend_from_slice(&buffer[..count]);
    }
    proxy.revoke().await.unwrap();
    assert_eq!(upstream.read(&mut [0]).await.unwrap(), 0);
    assert_eq!(browser.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn cleanup_wait_can_be_cancelled_without_losing_its_join_obligation() {
    use std::{
        future::Future as _,
        task::{Context, Poll, Waker},
    };
    let mut proxy = lease(
        vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
        limits(),
    )
    .await;
    let mut client = TcpStream::connect(proxy.address()).await.unwrap();
    client.write_all(b"CON").await.unwrap();
    let mut cleanup = Box::pin(proxy.revoke());
    let state = cleanup
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    drop(cleanup);
    if matches!(state, Poll::Pending) {
        assert!(proxy.task.is_some());
        assert_eq!(proxy.state(), BrowserEgressState::Revoking);
    }
    proxy.revoke().await.unwrap();
    assert_eq!(proxy.state(), BrowserEgressState::Revoked);
    let mut response = Vec::new();
    // Aborted clients with unread request bytes may receive reset or EOF; both
    // prove that the socket no longer carries background traffic.
    let _ = client.read_to_end(&mut response).await;
    assert!(TcpStream::connect(proxy.address()).await.is_err());
}

#[tokio::test]
async fn lost_supervisor_retains_unknown_cleanup_instead_of_acknowledging_revocation() {
    let mut proxy = lease(
        vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
        limits(),
    )
    .await;
    proxy.task.as_ref().unwrap().abort();
    assert_eq!(
        proxy.revoke().await,
        Err(BrowserEgressError::OutcomeUnknown)
    );
    assert_eq!(proxy.state(), BrowserEgressState::Interrupted);
    assert_eq!(
        proxy.revoke().await,
        Err(BrowserEgressError::OutcomeUnknown)
    );
}

#[tokio::test]
async fn per_connection_byte_limits_bound_requests_and_response_forwarding() {
    let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = server.local_addr().unwrap();
    let origin = format!("http://{address}");
    let mut proxy = lease(vec![BrowserOrigin::parse(&origin).unwrap()], limits()).await;
    let rejected = exchange(
        &proxy,
        &format!(
            "POST {origin}/ HTTP/1.1\r\nHost: {address}\r\nContent-Length: 65536\r\n{}\r\n",
            auth(&proxy)
        ),
    )
    .await;
    assert!(rejected.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), server.accept())
            .await
            .is_err()
    );
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = server.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0_u8; 1024];
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            header.extend_from_slice(&buffer[..count]);
        }
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n".to_vec();
        response.resize(131072, b'x');
        let _ = stream.write_all(&response).await;
    });
    let bounded = exchange(
        &proxy,
        &format!(
            "GET {origin}/ HTTP/1.1\r\nHost: {address}\r\n{}\r\n",
            auth(&proxy)
        ),
    )
    .await;
    assert_eq!(bounded.len(), limits().max_connection_bytes as usize);
    fixture.await.unwrap();
    proxy.revoke().await.unwrap();
}

#[tokio::test]
async fn tls_tunnel_preserves_server_verification_and_rejects_wrong_sni_before_upstream() {
    use rustls::{
        ClientConfig, RootCertStore, ServerConfig,
        pki_types::{PrivateKeyDer, ServerName},
    };
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    let key = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_config = ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![key.cert.der().clone()],
            PrivateKeyDer::Pkcs8(key.signing_key.serialize_der().into()),
        )
        .unwrap();
    let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = server.local_addr().unwrap().port();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let (negative_finished, negative_finished_rx) = oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (socket, _) = server.accept().await.unwrap();
        let mut stream = acceptor.accept(socket).await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0_u8; 1024];
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        negative_finished_rx.await.unwrap();
        // There must be no second upstream connect for the mismatched-SNI client.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.accept())
                .await
                .is_err()
        );
        request
    });
    let mut proxy = lease(
        vec![
            BrowserOrigin::parse(format!("https://localhost:{port}")).unwrap(),
            BrowserOrigin::parse(format!("https://127.0.0.1:{port}")).unwrap(),
        ],
        limits(),
    )
    .await;
    let mut roots = RootCertStore::empty();
    roots.add(key.cert.der().clone()).unwrap();
    let client_config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_config));
    for (name, authority, accepted) in [
        ("localhost", "localhost", true),
        ("denied.example", "localhost", false),
        ("localhost", "127.0.0.1", false),
    ] {
        let mut socket = TcpStream::connect(proxy.address()).await.unwrap();
        socket
            .write_all(
                format!(
                    "CONNECT {authority}:{port} HTTP/1.1\r\nHost: {authority}:{port}\r\n{}\r\n",
                    auth(&proxy)
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut ready = vec![0_u8; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
        socket.read_exact(&mut ready).await.unwrap();
        let handshake = connector
            .connect(ServerName::try_from(name).unwrap().to_owned(), socket)
            .await;
        if accepted {
            let mut stream = handshake.unwrap();
            stream
                .write_all(
                    b"GET /verified HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            assert!(response.ends_with(b"OK"));
        } else {
            assert!(handshake.is_err());
        }
    }
    negative_finished.send(()).unwrap();
    let request = String::from_utf8(fixture.await.unwrap()).unwrap();
    assert!(!request.contains(proxy.credential().expose()));
    proxy.revoke().await.unwrap();
}
