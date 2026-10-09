use super::*;
use rustls::{ServerConfig, pki_types::PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

struct TlsFixture {
    origin: String,
    pem: String,
    task: tokio::task::JoinHandle<Vec<String>>,
}

async fn tls_fixture(responses: impl FnOnce(&str) -> Vec<String>) -> TlsFixture {
    let key =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).expect("test certificate");
    let pem = key.cert.pem();
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![key.cert.der().clone()],
                PrivateKeyDer::Pkcs8(key.signing_key.serialize_der().into()),
            )
            .expect("server TLS");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let origin = format!("https://{}", listener.local_addr().expect("address"));
    let responses = responses(&origin);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .expect("accept deadline")
                .expect("accept");
            // A negative trust test must fail its handshake before sending HTTP.
            let Ok(mut stream) = acceptor.accept(stream).await else {
                continue;
            };
            requests.push(read_headers(&mut stream).await);
            stream
                .write_all(response.as_bytes())
                .await
                .expect("TLS response");
            let _ = stream.shutdown().await;
        }
        requests
    });
    TlsFixture { origin, pem, task }
}

#[tokio::test]
async fn https_redirects_verify_the_certificate_on_every_hop() {
    for trust_target in [false, true] {
        let target = tls_fixture(|_| vec![ok("trusted landing page")]).await;
        let source = tls_fixture(|_| {
            vec![redirect(
                302,
                &format!("{}/saml?SAMLResponse=private-assertion", target.origin),
            )]
        })
        .await;
        let pem = if trust_target {
            format!("{}\n{}", source.pem, target.pem)
        } else {
            source.pem
        };
        let roots =
            AdditionalRootCertificates::from_pem_bundle(pem.as_bytes()).expect("test trust");
        let (gateway, journal) = gateway(policy(&[&source.origin, &target.origin]));
        let result = gateway
            .execute(
                get(&source.origin),
                &HttpExecutor::new().with_tls_roots(roots),
            )
            .await;
        if trust_target {
            assert_eq!(
                result.expect("trusted chain").bytes,
                b"trusted landing page"
            );
        } else {
            let error = result.expect_err("target certificate is untrusted");
            assert!(!error.to_string().contains("private-assertion"));
            let events = journal.read_global(1, 100).expect("events");
            assert!(
                !serde_json::to_string(&events)
                    .expect("JSON")
                    .contains("private-assertion")
            );
            assert!(
                !events
                    .iter()
                    .any(|event| event.event_type == "effect.release_requested.v1")
            );
        }
        assert_eq!(source.task.await.expect("source").len(), 1);
        assert_eq!(
            target.task.await.expect("target").len(),
            usize::from(trust_target)
        );
    }
}

#[tokio::test]
async fn https_redirect_downgrades_are_denied_before_the_plaintext_connection() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.expect("target");
    let target_origin = format!("http://{}", target.local_addr().expect("address"));
    let source = tls_fixture(|_| vec![redirect(302, &target_origin)]).await;
    let roots =
        AdditionalRootCertificates::from_pem_bundle(source.pem.as_bytes()).expect("test trust");
    let (gateway, _) = gateway(policy(&[&source.origin, &target_origin]));
    let error = gateway
        .execute(
            get(&source.origin),
            &HttpExecutor::new().with_tls_roots(roots),
        )
        .await
        .expect_err("downgrade");
    assert!(error.to_string().contains("cannot downgrade"));
    assert!(
        tokio::time::timeout(Duration::from_millis(25), target.accept())
            .await
            .is_err()
    );
    assert_eq!(source.task.await.expect("source").len(), 1);
}

#[tokio::test]
async fn worm_audit_put_is_hash_bound_and_never_replayed_on_redirect() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.expect("target");
    let target_origin = format!("https://{}", target.local_addr().expect("address"));
    let source = tls_fixture(|_| vec![redirect(307, &target_origin)]).await;
    let roots =
        AdditionalRootCertificates::from_pem_bundle(source.pem.as_bytes()).expect("test trust");
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("audit.export.worm.write", DecisionOutcome::Allow)
                .with_network_destination(&source.origin)
                .with_network_destination(&target_origin),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.worm.write".into()]),
        [74_u8; 32],
    );
    let body = br#"{"event":"redacted"}"#;
    let hash = sha256_hex(body);
    let mut request = effect_request(
        system_actor("worm-redirect-test"),
        "audit.export.worm.write",
        format!("{}/event-{hash}.json", source.origin),
        json!({
            "method": "PUT", "create_only": true, "body_base64": BASE64.encode(body), "content_sha256": hash,
        }),
    );
    request.capabilities = vec!["audit.export.worm.write".into()];
    let error = gateway
        .execute(request, &HttpExecutor::new().with_tls_roots(roots))
        .await
        .expect_err("no WORM redirect");
    assert!(error.to_string().contains("307"));
    assert!(
        tokio::time::timeout(Duration::from_millis(25), target.accept())
            .await
            .is_err()
    );
    let requests = source.task.await.expect("source");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!("PUT /event-{hash}.json ")));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("\r\nif-none-match: *\r\n")
    );
}
