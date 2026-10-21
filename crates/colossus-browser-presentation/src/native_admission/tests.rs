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

fn review() -> NativeClientIdentityReview {
    NativeClientIdentityReview {
        session: BrowserSessionId::parse(format!("bs_{:032x}", 1)).unwrap(),
        challenge: ClientIdentityChallenge {
            key: ClientIdentityKey {
                tab: 1,
                generation: 1,
                request_id: 1,
                control_generation: 0,
            },
            origin: colossus_contracts::BrowserOrigin::parse("https://example.test").unwrap(),
            candidates: vec![crate::ClientIdentityCandidate {
                fingerprint_sha256: "a".repeat(64),
                label: "Test identity".into(),
            }],
            remaining_ms: 45_000,
        },
    }
}

#[test]
fn native_identity_wire_is_strict_and_contains_only_public_metadata() {
    let review = review();
    review.validate().unwrap();
    let wire = serde_json::to_value(&review).unwrap();
    assert_eq!(
        serde_json::from_value::<NativeClientIdentityReview>(wire.clone()).unwrap(),
        review
    );
    let mut forged = wire;
    forged["private_key_path"] = serde_json::json!("untrusted");
    assert!(serde_json::from_value::<NativeClientIdentityReview>(forged).is_err());
    let request = NativeBrowserRequest::SelectClientIdentity {
        session: review.session,
        key: review.challenge.key,
        fingerprint_sha256: Some("a".repeat(64)),
    };
    request.validate().unwrap();
    let mut forged = serde_json::to_value(&request).unwrap();
    forged["SelectClientIdentity"]["origin"] = serde_json::json!("https://foreign.test");
    assert!(serde_json::from_value::<NativeBrowserRequest>(forged).is_err());
    let NativeBrowserRequest::SelectClientIdentity { session, key, .. } = request else {
        unreachable!()
    };
    for fingerprint in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
        assert!(
            NativeBrowserRequest::SelectClientIdentity {
                session: session.clone(),
                key,
                fingerprint_sha256: Some(fingerprint),
            }
            .validate()
            .is_err()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn initial_open_can_finish_review_after_the_ordinary_effect_reply_deadline() {
    let intent = NativeBrowserOpen {
        conversation_id: None,
        url: BrowserUrl::parse("https://example.test").unwrap(),
        width: 800,
        height: 600,
        scale_milli: 1000,
        viewport_generation: 1,
        lease_ms: 1000,
    };
    let (client, server) = tokio::io::duplex(32768);
    let authentication = Zeroizing::new([7; 32]);
    let (client, server) = tokio::join!(
        client_handshake(client, &authentication, [1; 32]),
        server_handshake(server, &authentication, [1; 32]),
    );
    let mut client = client.unwrap();
    let mut server = server.unwrap();
    let request = NativeBrowserRequest::Open(intent);
    let (result, ()) = tokio::join!(client.request(&request), async {
        assert!(matches!(
            server.receive_request().await.unwrap(),
            NativeBrowserRequest::Open(_)
        ));
        tokio::time::sleep(std::time::Duration::from_secs(44)).await;
        server
            .reply(&NativeBrowserReply::Unavailable)
            .await
            .unwrap();
    },);
    assert!(matches!(result.unwrap(), NativeBrowserReply::Unavailable));
}

#[tokio::test(start_paused = true)]
async fn ordinary_admission_operations_keep_the_original_bounded_reply_wait() {
    let (client, server) = tokio::io::duplex(32768);
    let authentication = Zeroizing::new([7; 32]);
    let (client, server) = tokio::join!(
        client_handshake(client, &authentication, [1; 32]),
        server_handshake(server, &authentication, [1; 32]),
    );
    let mut client = client.unwrap();
    let mut server = server.unwrap();
    let start = tokio::time::Instant::now();
    let (result, ()) = tokio::join!(client.request(&NativeBrowserRequest::Probe), async {
        server.receive_request().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(41)).await;
    },);
    assert_eq!(result.unwrap_err(), PresentationError::OutcomeUnknown);
    assert_eq!(start.elapsed(), std::time::Duration::from_secs(41));
}
