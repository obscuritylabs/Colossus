use super::*;
use colossus_contracts::{ProviderFailure, ProviderFailureReason};

fn gateway(profile: &ProviderProfile) -> (EffectGateway, Arc<dyn EventJournal>) {
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(profile.network_origin().unwrap().unwrap())
        .with_post_effect(true);
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [29_u8; 32],
    );
    (gateway, journal)
}

fn assert_safe_failure(error: GatewayError, journal: &dyn EventJournal, http_status: Option<u16>) {
    assert!(matches!(
        error,
        GatewayError::ProviderRejected(ProviderFailure {
            reason: ProviderFailureReason::ContextLimitExceeded,
            http_status: status,
        }) if status == http_status
    ));
    let events = journal.read_global(1, 50).unwrap();
    let failure = events
        .iter()
        .find(|event| event.event_type == "effect.failed.v1")
        .expect("durable categorized failure");
    let payload = journal.decrypt_payload(failure).unwrap();
    assert_eq!(payload["code"], "provider.context_limit_exceeded");
    assert_eq!(payload["recoverable"], false);
    for event in &events {
        let payload = journal.decrypt_payload(event).unwrap();
        assert!(!payload.to_string().contains("PRIVATE"));
    }
}

#[tokio::test]
async fn http_context_rejection_survives_both_protocols_and_execution_modes() {
    for kind in [
        ProviderKind::OpenAiResponses,
        ProviderKind::OpenAiCompatible,
    ] {
        for streaming in [false, true] {
            let body = r#"{"error":{"code":"context_length_exceeded","message":"PRIVATE_REQUEST_DETAILS"}}"#;
            let (base_url, server) = one_status_body_server(400, "Bad Request", body).await;
            let profile = ProviderProfile::new("local", kind, Some(base_url), None, 5_000).unwrap();
            let (gateway, journal) = gateway(&profile);
            let executor = ProviderExecutor::new(profile.clone());
            let mut released = ReleasedItems::default();
            let error = if streaming {
                gateway
                    .execute_stream(provider_request(&profile), &executor, &mut released)
                    .await
            } else {
                gateway.execute(provider_request(&profile), &executor).await
            }
            .expect_err("known context rejection");
            assert_safe_failure(error, journal.as_ref(), Some(400));
            assert!(released.0.is_empty());
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn sse_context_rejection_is_a_known_categorized_failure() {
    for (kind, event) in [
        (
            ProviderKind::OpenAiResponses,
            json!({"type":"response.failed", "response":{"error":{"code":"context_length_exceeded", "message":"PRIVATE_STREAM_DETAILS"}}}),
        ),
        (
            ProviderKind::OpenAiCompatible,
            json!({"error":{"code":"context_length_exceeded", "message":"PRIVATE_STREAM_DETAILS"}}),
        ),
    ] {
        let (base_url, server) = one_sse_server(format!("data: {event}\n\n")).await;
        let profile = ProviderProfile::new("local", kind, Some(base_url), None, 5_000).unwrap();
        let (gateway, journal) = gateway(&profile);
        let executor = ProviderExecutor::new(profile.clone());
        let mut released = ReleasedItems::default();
        let error = gateway
            .execute_stream(provider_request(&profile), &executor, &mut released)
            .await
            .expect_err("known stream rejection");
        assert_safe_failure(error, journal.as_ref(), None);
        assert!(released.0.is_empty());
        server.await.unwrap();
    }
}
