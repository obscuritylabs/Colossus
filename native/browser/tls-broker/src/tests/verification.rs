use super::fixture::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn tls12_normal_verification_and_http11_work_on_both_legs() {
    let fixture = Fixture::versions("localhost", false, &[&rustls::version::TLS12]).await;
    let origin = fixture.origin();
    let mut broker = fixture.broker().await;
    let server = fixture.response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nTLS12".to_vec());
    let mut client = connect_versions(&broker, &origin, "localhost", &[&rustls::version::TLS12])
        .await
        .unwrap();
    assert_eq!(
        client.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_2)
    );
    client.write_all(&request(&origin)).await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.ends_with(b"TLS12"));
    server.await.unwrap();
    broker.revoke().await.unwrap();
}

#[tokio::test]
async fn upstream_hostname_and_expiry_verification_remain_enabled() {
    for (host, expired) in [("foreign.example", false), ("localhost", true)] {
        let fixture = Fixture::new(host, expired).await;
        let origin = fixture.origin();
        let mut broker = fixture.broker().await;
        let server = fixture.rejection();
        assert!(
            exchange(&broker, &origin, &request(&origin))
                .await
                .is_empty()
        );
        server.await.unwrap();
        broker.revoke().await.unwrap();
    }
}

#[tokio::test]
async fn unrelated_private_ca_never_becomes_an_upstream_trust_bypass() {
    let fixture = Fixture::new("localhost", false).await;
    let other = Fixture::new("localhost", false).await;
    let origin = fixture.origin();
    let mut broker = fixture.with_roots(vec![other.root]).await;
    let server = fixture.rejection();
    assert!(
        exchange(&broker, &origin, &request(&origin))
            .await
            .is_empty()
    );
    server.await.unwrap();
    broker.revoke().await.unwrap();
}
