use super::*;
use zeroize::Zeroizing;

fn enrollment() -> NativeBrowserEnrollment {
    NativeBrowserEnrollment {
        generation: [1; 16],
        instance: [2; 16],
        application_id: "app:desktop".into(),
        workspace_version: 1,
        workspace_digest: [3; 32],
        parent_process_id: 42,
    }
}
#[tokio::test]
async fn exact_native_enrollment_authenticates_both_directions_and_derives_the_same_relay_key() {
    let digest = enrollment().digest().unwrap();
    let (client, server) = tokio::io::duplex(32768);
    let server = tokio::spawn(async move {
        let mut channel = server_handshake(server, &Zeroizing::new([7; 32]), digest)
            .await
            .unwrap();
        assert!(matches!(
            channel.receive_request().await.unwrap(),
            NativeBrowserRequest::Probe
        ));
        channel
            .reply(&NativeBrowserReply::Unavailable)
            .await
            .unwrap();
        channel.into_presentation().1
    });
    let mut client = client_handshake(client, &Zeroizing::new([7; 32]), digest)
        .await
        .unwrap();
    assert!(matches!(
        client.request(&NativeBrowserRequest::Probe).await.unwrap(),
        NativeBrowserReply::Unavailable
    ));
    assert_eq!(*client.into_presentation().1, *server.await.unwrap());
}
#[tokio::test]
async fn worker_key_or_foreign_workspace_cannot_authenticate_the_native_endpoint() {
    for foreign_digest in [false, true] {
        let digest = enrollment().digest().unwrap();
        let (client, server) = tokio::io::duplex(32768);
        let server = tokio::spawn(async move {
            server_handshake(server, &Zeroizing::new([7; 32]), digest).await
        });
        let key = if foreign_digest { [7; 32] } else { [8; 32] };
        let mut client_digest = digest;
        if foreign_digest {
            client_digest[0] ^= 1;
        }
        assert!(matches!(
            client_handshake(client, &Zeroizing::new(key), client_digest).await,
            Err(PresentationError::Unauthenticated)
        ));
        drop(server.await.unwrap());
    }
}
#[tokio::test]
async fn independent_connections_never_reuse_a_presentation_relay_key() {
    let mut keys = Vec::new();
    for _ in 0..2 {
        let digest = enrollment().digest().unwrap();
        let (client, server) = tokio::io::duplex(32768);
        let server = tokio::spawn(async move {
            server_handshake(server, &Zeroizing::new([7; 32]), digest)
                .await
                .unwrap()
        });
        let client = client_handshake(client, &Zeroizing::new([7; 32]), digest)
            .await
            .unwrap();
        keys.push(client.into_presentation().1);
        drop(server.await.unwrap());
    }
    assert_ne!(*keys[0], *keys[1]);
}
#[test]
fn invalid_extreme_native_dimensions_fail_before_arithmetic_or_allocation() {
    let mut intent = NativeBrowserOpen {
        conversation_id: Some("owned-conversation".into()),
        url: BrowserUrl::parse("https://example.test").unwrap(),
        width: u32::MAX,
        height: u32::MAX,
        scale_milli: u32::MAX,
        viewport_generation: 1,
        lease_ms: 1500,
    };
    assert!(intent.validate().is_err());
    intent.width = 800;
    intent.height = 600;
    intent.scale_milli = 1000;
    assert!(intent.validate().is_ok());
    intent.scale_milli = 4000;
    assert!(intent.validate().is_err());
}
