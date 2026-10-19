use super::*;
use colossus_contracts::{ProviderFailure, ProviderFailureReason};

fn terminal_event(code: &str, message: &str) -> Value {
    json!({"type":"response.failed","response":{
        "id":"response-terminal","status":"failed",
        "error":{"code":code,"type":"invalid_request_error","param":"input","message":message},
        "output":[{"type":"reasoning","encrypted_content":"PRIVATE_REASONING","content":[{"text":"PRIVATE_HIDDEN_TEXT"}]}]
    }})
}

#[test]
fn terminal_sse_diagnostic_bounds_redaction_and_logical_rejection_preserve_transport_facts() {
    let mut secrets = executor::RequestSecrets::default();
    secrets.retain("unit-secret");
    for code in ["context_length_exceeded", "UNKNOWN_UPSTREAM_CODE"] {
        let event = terminal_event(
            code,
            &format!("unit-secret {}", "界\u{0001}".repeat(20_000)),
        );
        let diagnostic = stream_diagnostics::capture(
            "https://provider.invalid/responses",
            &json!({"input":"unit-secret", "stream":true}),
            &event,
            200,
            None,
            &secrets,
        )
        .unwrap();
        assert_eq!(diagnostic.status, 200);
        assert!(diagnostic.content_type.is_none()); // Codex can omit its SSE content type.
        assert!(diagnostic.body.len() <= MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES);
        assert!(diagnostic.body_truncated);
        assert_eq!(diagnostic.body_encoding, "utf8");
        let body: Value = serde_json::from_str(&diagnostic.body).unwrap();
        assert_eq!(body["response"]["error"]["code"], code);
        assert!(!diagnostic.body.contains("unit-secret"));
        assert!(!diagnostic.body.contains("PRIVATE_"));
        assert!(
            !diagnostic
                .request_body
                .as_ref()
                .unwrap()
                .to_string()
                .contains("unit-secret")
        );
        let failure = classify_response_diagnostic(&diagnostic).unwrap();
        assert_eq!(failure.http_status, None);
        assert_eq!(
            failure.reason,
            if code == "context_length_exceeded" {
                ProviderFailureReason::ContextLimitExceeded
            } else {
                ProviderFailureReason::Other
            }
        );
        assert!(
            crate::features::feature_rejection(
                &diagnostic,
                Some(&json!({"stream":true,"tools":[{}]}))
            )
            .is_none()
        );
    }
}

fn gateway_for(
    policy: Arc<dyn PolicyDecisionPoint>,
    journal: Arc<dyn EventJournal>,
) -> EffectGateway {
    EffectGateway::new(
        journal,
        policy,
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [37; 32],
    )
}

fn policy_for(profile: &ProviderProfile) -> BuiltInPolicy {
    BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(profile.network_origin().unwrap().unwrap())
        .with_post_effect(true)
}

fn failure_stream() -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\n",
        json!({"type":"response.created","response":{"id":"response-terminal"}}),
        json!({"type":"response.output_text.delta","delta":"accepted partial text"}),
        terminal_event(
            "UNKNOWN_UPSTREAM_CODE",
            "PRIVATE_TERMINAL_DETAIL unit-secret"
        )
    )
}

#[tokio::test]
async fn terminal_sse_diagnostic_is_opt_in_and_never_puts_error_body_in_normal_evidence() {
    for include in [false, true] {
        let (base, server) = one_sse_server(failure_stream()).await;
        let profile = ProviderProfile::new(
            "local",
            ProviderKind::OpenAiResponses,
            Some(base),
            Some("host:provider-main".into()),
            5_000,
        )
        .unwrap();
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = gateway_for(Arc::new(policy_for(&profile)), journal.clone());
        let executor = ProviderExecutor::with_credentials(
            profile.clone(),
            Arc::new(CountingHostCredentialResolver::new("unit-secret")),
        );
        let mut request = provider_request(&profile);
        request.content["include_response_diagnostics"] = json!(include);
        let mut released = ReleasedItems::default();
        let result = gateway
            .execute_stream(request, &executor, &mut released)
            .await;
        assert!(
            matches!(released.0.first(),Some(ProviderStreamItem::Event {event:ProviderEvent::ModelDelta{text}}) if text=="accepted partial text")
        );
        assert!(
            !released
                .0
                .iter()
                .any(|item| matches!(item, ProviderStreamItem::Completed { .. }))
        );
        if include {
            let terminal: ProviderStreamItem =
                serde_json::from_slice(&result.unwrap().bytes).unwrap();
            assert_eq!(released.0.last(), Some(&terminal));
            let ProviderStreamItem::Diagnostic { diagnostic } = terminal else {
                panic!("diagnostic")
            };
            assert_eq!(diagnostic.status, 200);
            assert!(diagnostic.body.contains("PRIVATE_TERMINAL_DETAIL"));
            assert!(!diagnostic.body.contains("unit-secret"));
            assert!(!diagnostic.body.contains("PRIVATE_REASONING"));
            assert_eq!(
                classify_response_diagnostic(&diagnostic),
                Some(ProviderFailure {
                    reason: ProviderFailureReason::Other,
                    http_status: None
                })
            );
        } else {
            assert!(matches!(
                result,
                Err(GatewayError::ProviderRejected(ProviderFailure {
                    reason: ProviderFailureReason::Other,
                    http_status: None
                }))
            ));
            assert_eq!(released.0.len(), 1);
        }
        for event in journal.read_global(1, 100).unwrap() {
            let payload = journal.decrypt_payload(&event).unwrap().to_string();
            assert!(!payload.contains("PRIVATE_TERMINAL_DETAIL"));
            assert!(!payload.contains("unit-secret"));
            assert!(!payload.contains("PRIVATE_REASONING"));
        }
        server.await.unwrap();
    }
}

struct DenyTerminalDiagnostic {
    policy: BuiltInPolicy,
    posts: AtomicUsize,
}
#[async_trait]
impl PolicyDecisionPoint for DenyTerminalDiagnostic {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<PolicyDecision, colossus_ports::PolicyError> {
        let mut decision = self.policy.decide(request).await?;
        if request.phase == EffectPhase::PostEffect && self.posts.fetch_add(1, Ordering::SeqCst) > 0
        {
            decision.outcome = DecisionOutcome::Deny;
        }
        Ok(decision)
    }
    async fn doctor(&self) -> Result<Value, colossus_ports::PolicyError> {
        self.policy.doctor().await
    }
}

#[tokio::test]
async fn terminal_sse_diagnostic_after_partial_text_must_pass_post_effect_policy() {
    let (base, server) = one_sse_server(failure_stream()).await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiResponses,
        Some(base),
        None,
        5_000,
    )
    .unwrap();
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = gateway_for(
        Arc::new(DenyTerminalDiagnostic {
            policy: policy_for(&profile),
            posts: AtomicUsize::new(0),
        }),
        journal.clone(),
    );
    let mut request = provider_request(&profile);
    request.content["include_response_diagnostics"] = json!(true);
    let mut released = ReleasedItems::default();
    assert!(
        gateway
            .execute_stream(request, &ProviderExecutor::new(profile), &mut released)
            .await
            .is_err()
    );
    assert_eq!(released.0.len(), 1);
    assert!(
        matches!(&released.0[0],ProviderStreamItem::Event{event:ProviderEvent::ModelDelta{text}} if text=="accepted partial text")
    );
    for event in journal.read_global(1, 100).unwrap() {
        assert!(
            !journal
                .decrypt_payload(&event)
                .unwrap()
                .to_string()
                .contains("PRIVATE_TERMINAL_DETAIL")
        );
    }
    server.await.unwrap();
}

#[tokio::test]
async fn collected_terminal_sse_diagnostic_after_partial_text_is_a_diagnostic_not_a_completed_turn()
{
    let (base, server) = one_sse_server_with_content_type(failure_stream(), None).await;
    let mut profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCodex,
        None,
        Some(CODEX_CREDENTIAL_REFERENCE.into()),
        5_000,
    )
    .unwrap();
    profile.base_url = Some(base);
    let directory = codex_auth_tempdir();
    let auth_path = directory.path().join("private").join("auth.json");
    write_test_codex_auth(&auth_path, 4_102_444_800);
    let executor = ProviderExecutor::new(profile.clone())
        .with_codex_auth_store(CodexAuthStore::at_path(auth_path));
    let gateway = gateway_for(
        Arc::new(policy_for(&profile)),
        Arc::new(InMemoryEventJournal::default()),
    );
    let mut request = provider_request(&profile);
    request.content["include_response_diagnostics"] = json!(true);
    let result = gateway.execute(request, &executor).await.unwrap();
    let diagnostic: ProviderResponseDiagnostic = serde_json::from_slice(&result.bytes).unwrap();
    assert_eq!(diagnostic.status, 200);
    assert_eq!(
        classify_response_diagnostic(&diagnostic)
            .unwrap()
            .http_status,
        None
    );
    server.await.unwrap();
}
