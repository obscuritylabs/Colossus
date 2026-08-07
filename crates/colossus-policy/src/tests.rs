use super::{
    AllowApproval, BuiltInPolicy, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit,
    GatewayError, NetworkDestinationMatch, QuarantinedEffectObserver, ReleasedEffectObserver,
    ReleasedEffectResult, SafetyKernel, SandboxBoundaryGate, StreamingEffectExecutor,
    effect_request, network_destination_match, system_actor, with_sandbox_boundary_acknowledgement,
};
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, ApprovalReviewNotice, AutomaticApprovalNotice, DecisionOutcome,
    ExecutionContext, QuarantinedEffectResult, RiskAssessment, RiskLevel, RiskRecommendation,
    RiskReviewFailure, RiskReviewFallbackNotice, RiskStatus, SandboxBoundaryMode,
};
use colossus_ports::{
    ApprovalProvider, EventJournal, PolicyDecisionPoint, PolicyError, RiskEvaluationError,
    RiskEvaluator,
};
use colossus_testkit::InMemoryEventJournal;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

struct CountingExecutor {
    calls: AtomicUsize,
}

struct RiskAutoApproval {
    prompts: AtomicUsize,
    notices: Mutex<Vec<ApprovalReviewNotice>>,
}

#[async_trait]
impl ApprovalProvider for RiskAutoApproval {
    fn risk_auto_enabled(&self) -> bool {
        true
    }

    async fn automatic_approval_granted(&self, notice: AutomaticApprovalNotice) {
        self.notices
            .lock()
            .expect("notices")
            .push(ApprovalReviewNotice::AutomaticApproval { notice });
    }

    async fn risk_review_fallback(&self, notice: RiskReviewFallbackNotice) {
        self.notices
            .lock()
            .expect("notices")
            .push(ApprovalReviewNotice::RiskReviewFallback { notice });
    }

    async fn request_approval(
        &self,
        _request: &colossus_contracts::EffectRequest,
        request_hash: &str,
        _decision: &colossus_contracts::PolicyDecision,
    ) -> Result<Option<colossus_contracts::ApprovalProof>, PolicyError> {
        self.prompts.fetch_add(1, Ordering::AcqRel);
        Ok(Some(super::approval_proof(request_hash, "test-operator")?))
    }
}

struct StaticRiskEvaluator {
    calls: AtomicUsize,
    assessment: Option<RiskAssessment>,
}

struct InvalidRiskEvaluator {
    calls: AtomicUsize,
}

#[async_trait]
impl RiskEvaluator for StaticRiskEvaluator {
    async fn evaluate(
        &self,
        _request: &colossus_contracts::EffectRequest,
        _decision: &colossus_contracts::PolicyDecision,
    ) -> Result<RiskAssessment, RiskEvaluationError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.assessment
            .clone()
            .ok_or_else(|| RiskEvaluationError::Unavailable("test evaluator unavailable".into()))
    }
}

#[async_trait]
impl RiskEvaluator for InvalidRiskEvaluator {
    async fn evaluate(
        &self,
        _request: &colossus_contracts::EffectRequest,
        _decision: &colossus_contracts::PolicyDecision,
    ) -> Result<RiskAssessment, RiskEvaluationError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(RiskEvaluationError::InvalidAssessment(
            "private provider diagnostic must not be released".into(),
        ))
    }
}

struct RiskRecordingPolicy {
    executable: String,
    cwd: String,
    saw_available_risk: AtomicUsize,
}

#[async_trait]
impl PolicyDecisionPoint for RiskRecordingPolicy {
    async fn decide(
        &self,
        request: &colossus_contracts::EffectRequest,
    ) -> Result<colossus_contracts::PolicyDecision, PolicyError> {
        let outcome = if request.phase == colossus_contracts::EffectPhase::PostEffect
            || request.approval.is_some()
        {
            if request.risk.status == RiskStatus::Available {
                self.saw_available_risk.fetch_add(1, Ordering::AcqRel);
            }
            DecisionOutcome::Allow
        } else {
            DecisionOutcome::RequireApproval
        };
        let mut obligations = super::default_obligations();
        obligations.sandbox_backend = "native".into();
        obligations.filesystem = vec![
            colossus_contracts::FilesystemGrant {
                root: self.executable.clone(),
                mode: "execute".into(),
            },
            colossus_contracts::FilesystemGrant {
                root: self.cwd.clone(),
                mode: "read".into(),
            },
        ];
        obligations.require_post_effect = true;
        Ok(colossus_contracts::PolicyDecision {
            decision_id: uuid::Uuid::now_v7().to_string(),
            policy_revision: "risk-test-v1".into(),
            outcome,
            reason: "risk test decision".into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<serde_json::Value, PolicyError> {
        Ok(serde_json::json!({"ready": true}))
    }
}

fn shell_request(
    executable: &std::path::Path,
    cwd: &std::path::Path,
) -> colossus_contracts::EffectRequest {
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::Model,
            id: "risk-test".into(),
        },
        "shell.run",
        executable.display().to_string(),
        serde_json::json!({
            "cwd": cwd,
            "args": ["--version"],
            "environment": {},
            "stdin_base64": null,
            "timeout_ms": null,
            "max_output_bytes": null,
        }),
    );
    request.capabilities = vec!["shell.run".into()];
    request
}

fn network_request(action: &str, content: serde_json::Value) -> colossus_contracts::EffectRequest {
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::Model,
            id: "risk-network-test".into(),
        },
        action,
        "https://example.test/resource",
        content,
    );
    request.capabilities = vec![action.into()];
    request
}

fn mcp_call_request(
    transport: &str,
    resource: &str,
    cwd: Option<&std::path::Path>,
) -> colossus_contracts::EffectRequest {
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {"message": {"type": "string"}},
        "required": ["message"],
        "additionalProperties": false,
    });
    let schema_sha256 =
        super::sha256_hex(&super::canonical_bytes(&input_schema).expect("canonical MCP schema"));
    let url = (transport == "streamable_http").then(|| resource.to_owned());
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::Model,
            id: "risk-mcp-test".into(),
        },
        "mcp.call",
        resource,
        serde_json::json!({
            "operation": {
                "kind": "call_tool",
                "server": "everything",
                "tool": "echo",
                "description": "Echo one bounded message",
                "annotations": {
                    "title": "Echo",
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false,
                },
                "arguments": {"message": "MCP tool test"},
                "input_schema": input_schema,
                "schema_sha256": schema_sha256,
            },
            "transport": transport,
            "cwd": cwd,
            "args": ["--stdio"],
            "environment": {},
            "url": url,
            "headers": {},
            "credential_headers": {},
            "allow_stateless": false,
            "oauth": null,
            "timeout_ms": 30_000,
            "max_output_bytes": 1_048_576,
            "provenance": null,
        }),
    );
    request.capabilities = vec!["mcp.invoke".into()];
    request
}

#[tokio::test]
async fn action_restrictions_remove_undeclared_global_network_and_environment_grants() {
    let policy = BuiltInPolicy::offline_default()
        .with_action("pack.tool.demo.fixed", DecisionOutcome::Allow)
        .with_network_destination("https://example.com")
        .with_environment("GLOBAL_SECRET")
        .with_action_restrictions(
            "pack.tool.demo.fixed",
            vec![colossus_contracts::FilesystemGrant {
                root: "/verified/pack".into(),
                mode: "read".into(),
            }],
            Vec::new(),
            Vec::new(),
        );
    let request = effect_request(
        system_actor("pack-test"),
        "pack.tool.demo.fixed",
        "/verified/pack/tool",
        serde_json::json!({
            "cwd": "/verified/pack",
            "environment": {},
        }),
    );
    let decision = policy.decide(&request).await.expect("decision");
    assert!(decision.obligations.network_destinations.is_empty());
    assert!(decision.obligations.allowed_environment.is_empty());
    assert_eq!(decision.obligations.filesystem.len(), 1);
    assert!(decision.obligations.require_post_effect);
}

#[tokio::test]
async fn action_timeout_retains_restrictions_without_widening_other_actions() {
    let policy = BuiltInPolicy::offline_default()
        .with_limits(30_000, 8_192, 2, 64 * 1024 * 1024, 1)
        .with_action("provider.openai.chat", DecisionOutcome::Allow)
        .with_action("network.http", DecisionOutcome::Allow)
        .with_action_restrictions(
            "provider.openai.chat",
            Vec::new(),
            Vec::new(),
            vec!["https://openrouter.ai".into()],
        )
        .with_action_timeout("provider.openai.chat", 120_000);
    let provider = policy
        .decide(&effect_request(
            system_actor("provider-test"),
            "provider.openai.chat",
            "https://openrouter.ai/api/v1/chat/completions",
            serde_json::json!({}),
        ))
        .await
        .expect("provider decision");
    assert_eq!(provider.obligations.timeout_ms, 120_000);
    assert_eq!(
        provider.obligations.network_destinations,
        vec!["https://openrouter.ai"]
    );

    let network = policy
        .decide(&effect_request(
            system_actor("network-test"),
            "network.http",
            "https://example.com",
            serde_json::json!({}),
        ))
        .await
        .expect("network decision");
    assert_eq!(network.obligations.timeout_ms, 30_000);
}

#[async_trait]
impl EffectExecutor for CountingExecutor {
    async fn execute(
        &self,
        _request: &colossus_contracts::EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(QuarantinedEffectResult {
            media_type: "text/plain".into(),
            bytes: b"ok".to_vec(),
            effect_succeeded: true,
        })
    }
}

#[tokio::test]
async fn every_effect_category_denies_before_adapter_without_a_permit() {
    let categories = [
        ("filesystem", "filesystem.write"),
        ("process", "process.spawn"),
        ("network", "network.http"),
        ("provider", "provider.openai.responses"),
        ("mcp", "mcp.call"),
        ("embedding", "embedding.openai.create"),
        ("memory_index", "memory.chroma.upsert"),
        ("integration", "integration.call"),
        ("memory", "memory.create"),
        ("domain_state", "task.create"),
        ("workflow", "workflow.execute"),
        ("workflow_control", "workflow.start"),
        ("subagent", "subagent.create"),
        ("research", "research.run"),
        ("skill", "skill.install"),
        ("pack", "pack.install"),
        ("bundle", "bundle.install"),
        ("repository", "repository.read"),
        ("context", "context.compact"),
        ("presentation", "presentation.preferences.update"),
        ("audit_export", "audit.export"),
    ];
    for (category, action) in categories {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(BuiltInPolicy::offline_default()),
            Arc::new(AllowApproval {
                approved_by: "operator".into(),
            }),
            SafetyKernel::new([]),
            [31_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let error = gateway
            .execute(
                effect_request(
                    system_actor(format!("{category}-test")),
                    action,
                    format!("{category}:resource"),
                    serde_json::json!({"category": category}),
                ),
                &executor,
            )
            .await
            .expect_err(category);
        assert!(
            matches!(error, GatewayError::Denied(_) | GatewayError::Safety(_)),
            "{category}: {error}"
        );
        assert_eq!(executor.calls.load(Ordering::Acquire), 0, "{category}");
        let events = journal.read_global(1, 20).expect(category);
        assert!(
            events
                .iter()
                .all(|event| event.event_type != "effect.started.v1"),
            "{category}"
        );
    }
}

#[tokio::test]
async fn permits_bind_every_claim_expire_authenticate_and_are_one_use() {
    let policy = Arc::new(
        BuiltInPolicy::offline_default()
            .with_action("echo", DecisionOutcome::Allow)
            .with_limits(5_000, 8_192, 2, 64 * 1024 * 1024, 1),
    );
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::clone(&policy) as Arc<dyn PolicyDecisionPoint>,
        Arc::new(AllowApproval {
            approved_by: "operator".into(),
        }),
        SafetyKernel::new([]),
        [32_u8; 32],
    );
    let request = effect_request(
        system_actor("permit-actor"),
        "echo",
        "echo:resource",
        serde_json::json!({"text": "hello"}),
    );
    let decision = policy.decide(&request).await.expect("decision");
    let mint = || {
        let hash = super::sha256_hex(&super::canonical_bytes(&request).expect("request bytes"));
        gateway
            .mint_permit(&request, hash, &decision)
            .expect("permit")
    };

    let request_mismatch = {
        let mut value = request.clone();
        value.content = serde_json::json!({"text": "changed"});
        value
    };
    assert!(matches!(
        gateway.authenticate_and_consume(&mint(), &request_mismatch, &decision),
        Err(GatewayError::Safety(_))
    ));

    let actor_mismatch = {
        let mut value = request.clone();
        value.actor.id = "another-actor".into();
        value
    };
    assert!(matches!(
        gateway.authenticate_and_consume(&mint(), &actor_mismatch, &decision),
        Err(GatewayError::Safety(_))
    ));

    let mut decision_mismatch = decision.clone();
    decision_mismatch.decision_id = "another-decision".into();
    assert!(matches!(
        gateway.authenticate_and_consume(&mint(), &request, &decision_mismatch),
        Err(GatewayError::Safety(_))
    ));

    let mut obligation_mismatch = decision.clone();
    obligation_mismatch.obligations.max_output_bytes += 1;
    assert!(matches!(
        gateway.authenticate_and_consume(&mint(), &request, &obligation_mismatch),
        Err(GatewayError::Safety(_))
    ));

    let mut expired = mint();
    expired.expires_at_unix_ms = super::now_unix_ms() - 1;
    assert!(matches!(
        gateway.authenticate_and_consume(&expired, &request, &decision),
        Err(GatewayError::Safety(_))
    ));

    let mut unauthenticated = mint();
    unauthenticated.authentication_tag[0] ^= 0xff;
    assert!(matches!(
        gateway.authenticate_and_consume(&unauthenticated, &request, &decision),
        Err(GatewayError::Safety(_))
    ));

    let permit = mint();
    gateway
        .authenticate_and_consume(&permit, &request, &decision)
        .expect("first use");
    assert!(matches!(
        gateway.authenticate_and_consume(&permit, &request, &decision),
        Err(GatewayError::Safety(message)) if message.contains("already been consumed")
    ));
}

#[tokio::test]
async fn mcp_automatic_authority_is_invalidated_by_endpoint_server_tool_schema_or_argument_changes()
{
    let policy = Arc::new(
        BuiltInPolicy::offline_default()
            .with_action("mcp.call", DecisionOutcome::Allow)
            .with_network_destination("http://127.0.0.1:3001"),
    );
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::clone(&policy) as Arc<dyn PolicyDecisionPoint>,
        Arc::new(AllowApproval {
            approved_by: "operator".into(),
        }),
        SafetyKernel::new(["mcp.invoke".into()]),
        [54_u8; 32],
    );
    let request = mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None);
    let decision = policy.decide(&request).await.expect("decision");
    let mint = || {
        let request_hash =
            super::sha256_hex(&super::canonical_bytes(&request).expect("request bytes"));
        gateway
            .mint_permit(&request, request_hash, &decision)
            .expect("permit")
    };

    let mut endpoint = request.clone();
    endpoint.resource = "http://127.0.0.1:3002/mcp".into();
    endpoint.content["url"] = serde_json::json!(endpoint.resource.clone());
    let mut server = request.clone();
    server.content["operation"]["server"] = serde_json::json!("other");
    let mut tool = request.clone();
    tool.content["operation"]["tool"] = serde_json::json!("other_echo");
    let mut schema = request.clone();
    schema.content["operation"]["schema_sha256"] = serde_json::json!("f".repeat(64));
    let mut arguments = request.clone();
    arguments.content["operation"]["arguments"]["message"] = serde_json::json!("changed");

    for (field, changed) in [
        ("endpoint", endpoint),
        ("server", server),
        ("tool", tool),
        ("schema hash", schema),
        ("arguments", arguments),
    ] {
        assert!(
            matches!(
                gateway.authenticate_and_consume(&mint(), &changed, &decision),
                Err(GatewayError::Safety(_))
            ),
            "{field} change must invalidate authority"
        );
    }
}

#[tokio::test]
async fn deny_never_reaches_adapter() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new([]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let error = gateway
        .execute(
            effect_request(
                system_actor("test"),
                "filesystem.write",
                "/tmp/x",
                serde_json::json!({"content":"x"}),
            ),
            &executor,
        )
        .await
        .expect_err("deny");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    let names = journal
        .read_global(1, 20)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(names.contains(&"effect.denied.v1".into()));
}

#[tokio::test]
async fn sensitive_disclosures_always_require_post_effect_release() {
    let actions = [
        "filesystem.read",
        "network.http",
        "web.search",
        "provider.openai.responses",
        "process.spawn",
        "memory.search",
        "registry.pull",
        "registry.push",
    ];
    let mut policy = BuiltInPolicy::offline_default().with_post_effect(false);
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    for action in actions {
        let decision = policy
            .decide(&effect_request(
                system_actor(format!("{action}-test")),
                action,
                "test:resource",
                serde_json::json!({"query": "rust"}),
            ))
            .await
            .expect(action);
        assert_eq!(decision.outcome, DecisionOutcome::Allow, "{action}");
        assert!(decision.obligations.require_post_effect, "{action}");
    }
}

#[tokio::test]
async fn process_environment_and_executable_obligations_fail_closed() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("executable")
        .canonicalize()
        .expect("canonical executable");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("process.spawn", DecisionOutcome::Allow)
        .with_sandbox("native", "test", false)
        .with_filesystem_root(executable.display().to_string(), "execute")
        .with_filesystem_read_root(directory.path().display().to_string());
    let gateway = EffectGateway::new(
        journal,
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new(["process.spawn".into()]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = effect_request(
        system_actor("test"),
        "process.spawn",
        executable.display().to_string(),
        serde_json::json!({
            "cwd": directory.path(),
            "args": [],
            "environment": {"SECRET": "not allowed"},
            "stdin_base64": null,
        }),
    );
    request.capabilities = vec!["process.spawn".into()];
    let error = gateway
        .execute(request, &executor)
        .await
        .expect_err("environment denied");
    assert!(matches!(error, GatewayError::Safety(_)));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn direct_process_backends_require_the_exact_session_acknowledgement() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("executable")
        .canonicalize()
        .expect("canonical executable");
    for mode in [
        SandboxBoundaryMode::External,
        SandboxBoundaryMode::DangerFullAccess,
    ] {
        const INTERACTIVE_CAPABILITY: &str =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let policy = BuiltInPolicy::offline_default()
            .with_action("process.spawn", DecisionOutcome::Allow)
            .with_sandbox(mode.as_backend(), "test", false)
            .with_filesystem_root(executable.display().to_string(), "execute");
        let gate = Arc::new(SandboxBoundaryGate::new(Some(mode), false));
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new(["process.spawn".into()])
                .with_sandbox_boundary_gate(Arc::clone(&gate)),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let request = || {
            let mut request = effect_request(
                system_actor("test"),
                "process.spawn",
                executable.display().to_string(),
                serde_json::json!({
                    "cwd": directory.path(),
                    "args": [],
                    "environment": {},
                    "stdin_base64": null,
                }),
            );
            request.capabilities = vec!["process.spawn".into()];
            request.context = ExecutionContext {
                session_id: Some("session-1".into()),
                ..ExecutionContext::default()
            };
            request
        };

        let error = gateway
            .execute(request(), &executor)
            .await
            .expect_err("unacknowledged direct process execution");
        assert!(error.to_string().contains("is not acknowledged"));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);

        gate.acknowledge_session("another-session", mode)
            .expect("other session acknowledgement");
        assert!(gateway.execute(request(), &executor).await.is_err());
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);

        gate.acknowledge_interactive_client(INTERACTIVE_CAPABILITY, "session-1", mode)
            .expect("interactive client acknowledgement");
        assert!(
            gate.acknowledge_interactive_client(INTERACTIVE_CAPABILITY, "session-1", mode)
                .is_err()
        );
        assert!(gateway.execute(request(), &executor).await.is_err());
        assert!(
            with_sandbox_boundary_acknowledgement(
                Some("wrong-capability".into()),
                gateway.execute(request(), &executor),
            )
            .await
            .is_err()
        );
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
        let mut other_session_request = request();
        other_session_request.context.session_id = Some("unacknowledged-session".into());
        assert!(
            with_sandbox_boundary_acknowledgement(
                Some(INTERACTIVE_CAPABILITY.into()),
                gateway.execute(other_session_request, &executor),
            )
            .await
            .is_err()
        );
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
        with_sandbox_boundary_acknowledgement(
            Some(INTERACTIVE_CAPABILITY.into()),
            gateway.execute(request(), &executor),
        )
        .await
        .expect("client-scoped acknowledgement authorizes its attached operation");
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
        assert!(gateway.execute(request(), &executor).await.is_err());
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);

        gate.acknowledge_session("session-1", mode)
            .expect("active session acknowledgement");
        gateway
            .execute(request(), &executor)
            .await
            .expect("direct process cwd does not require an unenforced filesystem declaration");
        assert_eq!(executor.calls.load(Ordering::Acquire), 2);
    }
}

#[tokio::test]
async fn denied_direct_process_effects_do_not_request_a_boundary_acknowledgement() {
    let policy = BuiltInPolicy::offline_default()
        .with_action("process.spawn", DecisionOutcome::Deny)
        .with_sandbox("external", "test", false);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new(["process.spawn".into()]).with_sandbox_boundary_gate(Arc::new(
            SandboxBoundaryGate::new(Some(SandboxBoundaryMode::External), false),
        )),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = effect_request(
        system_actor("test"),
        "process.spawn",
        "/unavailable",
        serde_json::json!({}),
    );
    request.capabilities = vec!["process.spawn".into()];
    let error = gateway
        .execute(request, &executor)
        .await
        .expect_err("policy denial");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn oci_process_timeout_must_reserve_confirmed_cleanup_time() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("executable")
        .canonicalize()
        .expect("canonical executable");
    let policy = BuiltInPolicy::offline_default()
        .with_action("process.spawn", DecisionOutcome::Allow)
        .with_sandbox("oci", "test", false)
        .with_limits(1_000, 1024, 2, 64 * 1024 * 1024, 1)
        .with_filesystem_root(executable.display().to_string(), "execute")
        .with_filesystem_read_root(directory.path().display().to_string());
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new(["process.spawn".into()]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = effect_request(
        system_actor("test"),
        "process.spawn",
        executable.display().to_string(),
        serde_json::json!({
            "cwd": directory.path(),
            "args": [],
            "environment": {},
            "stdin_base64": null,
        }),
    );
    request.capabilities = vec!["process.spawn".into()];
    assert!(matches!(
        gateway.execute(request, &executor).await,
        Err(GatewayError::Safety(_))
    ));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn networked_oci_process_reserves_proxy_and_container_cleanup_time() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("executable")
        .canonicalize()
        .expect("canonical executable");
    let policy = BuiltInPolicy::offline_default()
        .with_action("process.spawn", DecisionOutcome::Allow)
        .with_sandbox("oci", "test", false)
        .with_limits(9_999, 1024, 2, 64 * 1024 * 1024, 1)
        .with_filesystem_root(executable.display().to_string(), "execute")
        .with_filesystem_read_root(directory.path().display().to_string())
        .with_network_destination("https://example.com");
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new(["process.spawn".into()]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = effect_request(
        system_actor("test"),
        "process.spawn",
        executable.display().to_string(),
        serde_json::json!({
            "cwd": directory.path(),
            "args": [],
            "environment": {},
            "stdin_base64": null,
        }),
    );
    request.capabilities = vec!["process.spawn".into()];
    assert!(matches!(
        gateway.execute(request, &executor).await,
        Err(GatewayError::Safety(_))
    ));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn oci_executable_identity_is_an_exact_normalized_image_path() {
    let directory = tempfile::tempdir().expect("directory");
    let policy = BuiltInPolicy::offline_default()
        .with_action("process.spawn", DecisionOutcome::Allow)
        .with_sandbox("oci", "test", false)
        .with_limits(10_000, 1024, 2, 64 * 1024 * 1024, 1)
        .with_filesystem_root("/image/bin/tool", "execute")
        .with_filesystem_read_root(directory.path().display().to_string());
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new(["process.spawn".into()]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = effect_request(
        system_actor("test"),
        "process.spawn",
        "/image/bin/tool",
        serde_json::json!({
            "cwd": directory.path(),
            "args": [],
            "environment": {},
            "stdin_base64": null,
        }),
    );
    request.capabilities = vec!["process.spawn".into()];
    gateway
        .execute(request, &executor)
        .await
        .expect("exact image path");
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);

    let mut request = effect_request(
        system_actor("test"),
        "process.spawn",
        "/image/../image/bin/tool",
        serde_json::json!({
            "cwd": directory.path(),
            "args": [],
            "environment": {},
            "stdin_base64": null,
        }),
    );
    request.capabilities = vec!["process.spawn".into()];
    assert!(matches!(
        gateway.execute(request, &executor).await,
        Err(GatewayError::Safety(_))
    ));
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn network_origins_not_in_obligations_never_reach_adapters() {
    for action in [
        "network.http",
        "audit.export.worm.write",
        "registry.pull",
        "registry.push",
    ] {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default().with_action(action, DecisionOutcome::Allow);
        let gateway = EffectGateway::new(
            journal,
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new([action.into()]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let mut request = effect_request(
            system_actor("test"),
            action,
            "https://example.com/path",
            serde_json::json!({"method": "GET", "headers": {}}),
        );
        request.capabilities = vec![action.into()];
        assert!(matches!(
            gateway.execute(request, &executor).await,
            Err(GatewayError::Safety(_))
        ));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    }
}

#[tokio::test]
async fn approval_is_reevaluated_before_execution() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("filesystem.write", DecisionOutcome::RequireApproval)
        .with_filesystem_root("/tmp", "write");
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new([]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let result = gateway
        .execute(
            effect_request(
                system_actor("test"),
                "filesystem.write",
                "/tmp/x",
                serde_json::json!({"content":"x"}),
            ),
            &executor,
        )
        .await
        .expect("allow after proof");
    assert_eq!(result.bytes, b"ok");
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    let names = journal
        .read_global(1, 20)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(names.contains(&"approval.granted.v1".into()));
    assert_eq!(
        names
            .iter()
            .filter(|name| name.as_str() == "policy.decided.v1")
            .count(),
        3
    );
}

#[tokio::test]
async fn low_allow_risk_review_auto_approves_and_reaches_policy_as_advisory_input() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("current executable")
        .canonicalize()
        .expect("canonical executable");
    let approvals = Arc::new(RiskAutoApproval {
        prompts: AtomicUsize::new(0),
        notices: Mutex::new(Vec::new()),
    });
    let evaluator = Arc::new(StaticRiskEvaluator {
        calls: AtomicUsize::new(0),
        assessment: Some(RiskAssessment {
            risk_level: RiskLevel::Low,
            recommended_decision: RiskRecommendation::Allow,
            reason: "read-only version inspection".into(),
        }),
    });
    let policy = Arc::new(RiskRecordingPolicy {
        executable: executable.display().to_string(),
        cwd: directory.path().display().to_string(),
        saw_available_risk: AtomicUsize::new(0),
    });
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::clone(&policy) as Arc<dyn PolicyDecisionPoint>,
        Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
        SafetyKernel::new(["shell.run".into()]),
        [42_u8; 32],
    );
    let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
    gateway
        .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
        .expect("bind evaluator");
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };

    gateway
        .execute(shell_request(&executable, directory.path()), &executor)
        .await
        .expect("low-risk effect");

    assert_eq!(evaluator.calls.load(Ordering::Acquire), 1);
    assert_eq!(approvals.prompts.load(Ordering::Acquire), 0);
    assert_eq!(
        *approvals.notices.lock().expect("notices"),
        vec![ApprovalReviewNotice::AutomaticApproval {
            notice: AutomaticApprovalNotice {
                action: "shell.run".into(),
                resource: executable.display().to_string(),
                risk_level: RiskLevel::Low,
                reason: "read-only version inspection".into(),
            },
        }]
    );
    assert_eq!(policy.saw_available_risk.load(Ordering::Acquire), 2);
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    let events = journal.read_global(1, 30).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "risk.review.completed.v1")
    );
    let approval = events
        .iter()
        .find(|event| event.event_type == "approval.granted.v1")
        .expect("approval event");
    let payload = journal.decrypt_payload(approval).expect("approval payload");
    assert_eq!(payload["approved_by"], "risk-evaluator:auto-low-risk");
}

#[tokio::test]
async fn low_risk_read_only_network_review_auto_approves_without_prompting() {
    for (action, content) in [
        (
            "network.http",
            serde_json::json!({"method": "GET", "headers": {"accept": "*/*"}}),
        ),
        (
            "web.search",
            serde_json::json!({"profile": "test", "request": {"query": "rust", "limit": 3}}),
        ),
    ] {
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let evaluator = Arc::new(StaticRiskEvaluator {
            calls: AtomicUsize::new(0),
            assessment: Some(RiskAssessment {
                risk_level: RiskLevel::Low,
                recommended_decision: RiskRecommendation::Allow,
                reason: "read-only configured network request".into(),
            }),
        });
        let policy = Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(action, DecisionOutcome::RequireApproval)
                .with_network_destination("https://example.test")
                .with_post_effect(true),
        );
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            policy as Arc<dyn PolicyDecisionPoint>,
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new([action.into()]),
            [46_u8; 32],
        );
        let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(network_request(action, content), &executor)
            .await
            .expect("low-risk network effect");

        assert_eq!(evaluator.calls.load(Ordering::Acquire), 1, "{action}");
        assert_eq!(approvals.prompts.load(Ordering::Acquire), 0, "{action}");
        assert_eq!(approvals.notices.lock().expect("notices").len(), 1);
        assert_eq!(executor.calls.load(Ordering::Acquire), 1, "{action}");
    }
}

#[tokio::test]
async fn low_allow_mcp_review_auto_approves_stdio_and_streamable_http_calls() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("current executable")
        .canonicalize()
        .expect("canonical executable");
    let cases: Vec<(
        colossus_contracts::EffectRequest,
        Arc<dyn PolicyDecisionPoint>,
    )> = vec![
        (
            mcp_call_request(
                "stdio",
                &executable.display().to_string(),
                Some(directory.path()),
            ),
            Arc::new(RiskRecordingPolicy {
                executable: executable.display().to_string(),
                cwd: directory.path().display().to_string(),
                saw_available_risk: AtomicUsize::new(0),
            }),
        ),
        (
            mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.call", DecisionOutcome::RequireApproval)
                    .with_sandbox("native", "mcp-risk-test", false)
                    .with_network_destination("http://127.0.0.1:3001"),
            ),
        ),
    ];

    for (request, policy) in cases {
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let evaluator = Arc::new(StaticRiskEvaluator {
            calls: AtomicUsize::new(0),
            assessment: Some(RiskAssessment {
                risk_level: RiskLevel::Low,
                recommended_decision: RiskRecommendation::Allow,
                reason: "exact echo call is bounded and non-destructive".into(),
            }),
        });
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            policy,
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new(["mcp.invoke".into()]),
            [49_u8; 32],
        );
        let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(request, &executor)
            .await
            .expect("low-risk MCP call");

        assert_eq!(evaluator.calls.load(Ordering::Acquire), 1);
        assert_eq!(approvals.prompts.load(Ordering::Acquire), 0);
        assert!(matches!(
            approvals.notices.lock().expect("notices").as_slice(),
            [ApprovalReviewNotice::AutomaticApproval {
                notice: AutomaticApprovalNotice {
                    action,
                    risk_level: RiskLevel::Low,
                    ..
                }
            }] if action == "mcp.call"
        ));
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
        let events = journal.read_global(1, 30).expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "approval.granted.v1")
        );
    }
}

#[tokio::test]
async fn destructive_or_ambiguous_mcp_assessments_preserve_explicit_approval() {
    for assessment in [
        RiskAssessment {
            risk_level: RiskLevel::Medium,
            recommended_decision: RiskRecommendation::RequireApproval,
            reason: "the tool hints conflict with potentially mutating arguments".into(),
        },
        RiskAssessment {
            risk_level: RiskLevel::High,
            recommended_decision: RiskRecommendation::Deny,
            reason: "the exact call may destroy external state".into(),
        },
    ] {
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let evaluator = Arc::new(StaticRiskEvaluator {
            calls: AtomicUsize::new(0),
            assessment: Some(assessment),
        });
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.call", DecisionOutcome::RequireApproval)
                    .with_sandbox("native", "mcp-risk-test", false)
                    .with_network_destination("http://127.0.0.1:3001"),
            ),
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new(["mcp.invoke".into()]),
            [50_u8; 32],
        );
        let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(
                mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None),
                &executor,
            )
            .await
            .expect("operator-approved MCP call");

        assert_eq!(evaluator.calls.load(Ordering::Acquire), 1);
        assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
        assert!(approvals.notices.lock().expect("notices").is_empty());
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }
}

#[tokio::test]
async fn unavailable_and_malformed_mcp_reviews_warn_and_fall_back_to_explicit_approval() {
    let cases: Vec<(Arc<dyn RiskEvaluator>, RiskReviewFailure)> = vec![
        (
            Arc::new(StaticRiskEvaluator {
                calls: AtomicUsize::new(0),
                assessment: None,
            }),
            RiskReviewFailure::EvaluatorUnavailable,
        ),
        (
            Arc::new(InvalidRiskEvaluator {
                calls: AtomicUsize::new(0),
            }),
            RiskReviewFailure::InvalidAssessment,
        ),
    ];
    for (evaluator, expected_failure) in cases {
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.call", DecisionOutcome::RequireApproval)
                    .with_sandbox("native", "mcp-risk-test", false)
                    .with_network_destination("http://127.0.0.1:3001"),
            ),
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new(["mcp.invoke".into()]),
            [51_u8; 32],
        );
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(
                mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None),
                &executor,
            )
            .await
            .expect("operator-approved MCP fallback");

        assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
        assert!(matches!(
            approvals.notices.lock().expect("notices").as_slice(),
            [ApprovalReviewNotice::RiskReviewFallback {
                notice: RiskReviewFallbackNotice { failure, .. }
            }] if *failure == expected_failure
        ));
        assert!(
            journal
                .read_global(1, 30)
                .expect("events")
                .iter()
                .any(|event| event.event_type == "risk.review.unavailable.v1")
        );
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }
}

#[tokio::test]
async fn unsupported_mcp_review_metadata_skips_evaluation_with_a_durable_prompt_reason() {
    let approvals = Arc::new(RiskAutoApproval {
        prompts: AtomicUsize::new(0),
        notices: Mutex::new(Vec::new()),
    });
    let evaluator = Arc::new(StaticRiskEvaluator {
        calls: AtomicUsize::new(0),
        assessment: Some(RiskAssessment {
            risk_level: RiskLevel::Low,
            recommended_decision: RiskRecommendation::Allow,
            reason: "must not be used for unsupported metadata".into(),
        }),
    });
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("mcp.call", DecisionOutcome::RequireApproval)
                .with_sandbox("native", "mcp-risk-test", false)
                .with_network_destination("http://127.0.0.1:3001"),
        ),
        Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
        SafetyKernel::new(["mcp.invoke".into()]),
        [52_u8; 32],
    );
    let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
    gateway
        .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
        .expect("bind evaluator");
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None);
    request.content["operation"]["schema_sha256"] = serde_json::json!("not-a-schema-hash");

    gateway
        .execute(request, &executor)
        .await
        .expect("operator-approved unsupported metadata");

    assert_eq!(evaluator.calls.load(Ordering::Acquire), 0);
    assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
    assert!(approvals.notices.lock().expect("notices").is_empty());
    let event = journal
        .read_global(1, 30)
        .expect("events")
        .into_iter()
        .find(|event| event.event_type == "risk.review.ineligible.v1")
        .expect("ineligible review event");
    let payload = journal.decrypt_payload(&event).expect("payload");
    assert!(
        payload["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("request-bound discovery metadata"))
    );
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn non_read_only_network_effects_still_require_explicit_approval() {
    for content in [
        serde_json::json!({"method": "POST"}),
        serde_json::json!({"method": "GET", "body_base64": "d3JpdGU="}),
    ] {
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let evaluator = Arc::new(StaticRiskEvaluator {
            calls: AtomicUsize::new(0),
            assessment: Some(RiskAssessment {
                risk_level: RiskLevel::Low,
                recommended_decision: RiskRecommendation::Allow,
                reason: "test evaluator would allow if invoked".into(),
            }),
        });
        let policy = Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("network.http", DecisionOutcome::RequireApproval)
                .with_network_destination("https://example.test")
                .with_post_effect(true),
        );
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            policy as Arc<dyn PolicyDecisionPoint>,
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new(["network.http".into()]),
            [47_u8; 32],
        );
        let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(network_request("network.http", content), &executor)
            .await
            .expect("operator-approved network effect");

        assert_eq!(evaluator.calls.load(Ordering::Acquire), 0);
        assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
        assert!(approvals.notices.lock().expect("notices").is_empty());
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }
}

#[tokio::test]
async fn unavailable_or_non_low_risk_review_requires_explicit_approval() {
    for assessment in [
        None,
        Some(RiskAssessment {
            risk_level: RiskLevel::Medium,
            recommended_decision: RiskRecommendation::RequireApproval,
            reason: "writes repository state".into(),
        }),
        Some(RiskAssessment {
            risk_level: RiskLevel::High,
            recommended_decision: RiskRecommendation::Deny,
            reason: "destructive command".into(),
        }),
    ] {
        let expects_failure_notice = assessment.is_none();
        let directory = tempfile::tempdir().expect("directory");
        let executable = std::env::current_exe()
            .expect("current executable")
            .canonicalize()
            .expect("canonical executable");
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let evaluator = Arc::new(StaticRiskEvaluator {
            calls: AtomicUsize::new(0),
            assessment,
        });
        let policy = Arc::new(RiskRecordingPolicy {
            executable: executable.display().to_string(),
            cwd: directory.path().display().to_string(),
            saw_available_risk: AtomicUsize::new(0),
        });
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            policy as Arc<dyn PolicyDecisionPoint>,
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new(["shell.run".into()]),
            [43_u8; 32],
        );
        let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(shell_request(&executable, directory.path()), &executor)
            .await
            .expect("operator-approved effect");

        assert_eq!(evaluator.calls.load(Ordering::Acquire), 1);
        assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
        let notices = approvals.notices.lock().expect("notices");
        if expects_failure_notice {
            assert!(matches!(
                notices.as_slice(),
                [ApprovalReviewNotice::RiskReviewFallback {
                    notice: RiskReviewFallbackNotice {
                        failure: RiskReviewFailure::EvaluatorUnavailable,
                        ..
                    }
                }]
            ));
        } else {
            assert!(notices.is_empty());
        }
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }
}

#[tokio::test]
async fn invalid_risk_review_warns_with_a_sanitized_manual_fallback() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("current executable")
        .canonicalize()
        .expect("canonical executable");
    let approvals = Arc::new(RiskAutoApproval {
        prompts: AtomicUsize::new(0),
        notices: Mutex::new(Vec::new()),
    });
    let evaluator = Arc::new(InvalidRiskEvaluator {
        calls: AtomicUsize::new(0),
    });
    let policy = Arc::new(RiskRecordingPolicy {
        executable: executable.display().to_string(),
        cwd: directory.path().display().to_string(),
        saw_available_risk: AtomicUsize::new(0),
    });
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        policy as Arc<dyn PolicyDecisionPoint>,
        Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
        SafetyKernel::new(["shell.run".into()]),
        [48_u8; 32],
    );
    let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
    gateway
        .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
        .expect("bind evaluator");
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };

    gateway
        .execute(shell_request(&executable, directory.path()), &executor)
        .await
        .expect("operator-approved effect");

    assert_eq!(evaluator.calls.load(Ordering::Acquire), 1);
    assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
    let notices = approvals.notices.lock().expect("notices");
    let [ApprovalReviewNotice::RiskReviewFallback { notice }] = notices.as_slice() else {
        panic!("expected one risk review fallback notice");
    };
    assert_eq!(notice.failure, RiskReviewFailure::InvalidAssessment);
    assert!(notice.reason.contains("strict validation"));
    assert!(!notice.reason.contains("private provider diagnostic"));
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn workflow_lineage_never_receives_risk_auto_approval() {
    let directory = tempfile::tempdir().expect("directory");
    let executable = std::env::current_exe()
        .expect("current executable")
        .canonicalize()
        .expect("canonical executable");
    let approvals = Arc::new(RiskAutoApproval {
        prompts: AtomicUsize::new(0),
        notices: Mutex::new(Vec::new()),
    });
    let evaluator = Arc::new(StaticRiskEvaluator {
        calls: AtomicUsize::new(0),
        assessment: Some(RiskAssessment {
            risk_level: RiskLevel::Low,
            recommended_decision: RiskRecommendation::Allow,
            reason: "would be low risk outside a workflow".into(),
        }),
    });
    let policy = Arc::new(RiskRecordingPolicy {
        executable: executable.display().to_string(),
        cwd: directory.path().display().to_string(),
        saw_available_risk: AtomicUsize::new(0),
    });
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        policy as Arc<dyn PolicyDecisionPoint>,
        Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
        SafetyKernel::new(["shell.run".into()]),
        [45_u8; 32],
    );
    let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
    gateway
        .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
        .expect("bind evaluator");
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = shell_request(&executable, directory.path());
    request.context.workflow_id = Some("workflow-1".into());
    request.context.workflow_hash = Some("sha256:workflow".into());

    gateway
        .execute(request, &executor)
        .await
        .expect("explicitly approved workflow effect");

    assert_eq!(evaluator.calls.load(Ordering::Acquire), 0);
    assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
    assert!(approvals.notices.lock().expect("notices").is_empty());
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn workflow_and_system_mcp_calls_never_receive_risk_auto_approval() {
    let mut workflow = mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None);
    workflow.context.workflow_id = Some("workflow-1".into());
    workflow.context.workflow_hash = Some("sha256:workflow".into());
    let mut system = mcp_call_request("streamable_http", "http://127.0.0.1:3001/mcp", None);
    system.actor = system_actor("mcp-system");

    for request in [workflow, system] {
        let approvals = Arc::new(RiskAutoApproval {
            prompts: AtomicUsize::new(0),
            notices: Mutex::new(Vec::new()),
        });
        let evaluator = Arc::new(StaticRiskEvaluator {
            calls: AtomicUsize::new(0),
            assessment: Some(RiskAssessment {
                risk_level: RiskLevel::Low,
                recommended_decision: RiskRecommendation::Allow,
                reason: "must not be used for this lineage".into(),
            }),
        });
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.call", DecisionOutcome::RequireApproval)
                    .with_sandbox("native", "mcp-risk-test", false)
                    .with_network_destination("http://127.0.0.1:3001"),
            ),
            Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
            SafetyKernel::new(["mcp.invoke".into()]),
            [53_u8; 32],
        );
        let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
        gateway
            .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
            .expect("bind evaluator");
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };

        gateway
            .execute(request, &executor)
            .await
            .expect("explicitly approved MCP call");

        assert_eq!(evaluator.calls.load(Ordering::Acquire), 0);
        assert_eq!(approvals.prompts.load(Ordering::Acquire), 1);
        assert!(approvals.notices.lock().expect("notices").is_empty());
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }
}

#[tokio::test]
async fn workspace_development_obligations_are_removed_by_workflow_lineage() {
    let workspace = tempfile::tempdir().expect("workspace");
    let policy = BuiltInPolicy::offline_default()
        .with_action("shell.run", DecisionOutcome::Allow)
        .with_workspace_development(
            vec![colossus_contracts::FilesystemGrant {
                root: workspace.path().display().to_string(),
                mode: "write".into(),
            }],
            Vec::new(),
            vec!["DEVELOPMENT_TOKEN".into()],
        );
    let mut model = shell_request(
        std::env::current_exe().expect("executable").as_path(),
        workspace.path(),
    );
    let model_decision = policy.decide(&model).await.expect("model decision");
    assert!(
        model_decision
            .obligations
            .filesystem
            .iter()
            .any(|grant| grant.root == workspace.path().display().to_string())
    );
    assert!(
        model_decision
            .obligations
            .allowed_environment
            .contains(&"DEVELOPMENT_TOKEN".into())
    );

    model.context.workflow_id = Some("workflow-1".into());
    model.context.workflow_hash = Some("sha256:workflow".into());
    let workflow_decision = policy.decide(&model).await.expect("workflow decision");
    assert!(workflow_decision.obligations.filesystem.is_empty());
    assert!(
        !workflow_decision
            .obligations
            .allowed_environment
            .contains(&"DEVELOPMENT_TOKEN".into())
    );
    assert!(
        workflow_decision
            .obligations
            .allowed_environment
            .contains(&"PATH".into())
    );
}

#[test]
fn public_network_wildcard_excludes_non_public_and_metadata_origins() {
    let wildcard = vec!["*".into()];
    assert_eq!(
        network_destination_match(&wildcard, "https://example.com/path").expect("public"),
        Some(NetworkDestinationMatch::PublicWildcard)
    );
    for denied in [
        "http://127.0.0.1:8888/search",
        "http://10.0.0.1/",
        "http://100.64.0.1/",
        "http://169.254.169.254/latest/meta-data",
        "http://[::ffff:127.0.0.1]/",
        "http://[2001:db8::1]/",
        "http://metadata.google.internal/",
    ] {
        assert_eq!(
            network_destination_match(&wildcard, denied).expect("valid URL"),
            None
        );
    }
    assert_eq!(
        network_destination_match(
            &["*".into(), "http://127.0.0.1:8888".into()],
            "http://127.0.0.1:8888/search",
        )
        .expect("exact loopback"),
        Some(NetworkDestinationMatch::Exact)
    );
}

#[tokio::test]
async fn streamable_http_mcp_uses_network_not_process_obligations() {
    let origin = "https://splunk.example.com";
    let policy = BuiltInPolicy::offline_default()
        .with_action("mcp.tools", DecisionOutcome::Allow)
        .with_action_restrictions("mcp.tools", Vec::new(), Vec::new(), vec![origin.into()]);
    let request = effect_request(
        system_actor("mcp-http-test"),
        "mcp.tools",
        format!("{origin}/services/mcp"),
        serde_json::json!({
            "transport": "streamable_http",
            "operation": {"kind": "list_tools", "server": "splunk", "cursor": null}
        }),
    );
    let decision = policy.decide(&request).await.expect("decision");
    SafetyKernel::new(Vec::new())
        .validate_decision(&request, &decision)
        .expect("remote MCP is a network effect");

    let denied = effect_request(
        system_actor("mcp-http-test"),
        "mcp.tools",
        "https://other.example.com/services/mcp",
        request.content.clone(),
    );
    let denied_decision = policy.decide(&denied).await.expect("decision");
    assert!(
        SafetyKernel::new(Vec::new())
            .validate_decision(&denied, &denied_decision)
            .is_err()
    );

    let stdio = effect_request(
        system_actor("mcp-http-test"),
        "mcp.tools",
        "/usr/bin/false",
        serde_json::json!({
            "transport": "stdio",
            "cwd": "/tmp",
            "args": [],
            "environment": {},
        }),
    );
    let stdio_decision = policy.decide(&stdio).await.expect("decision");
    assert!(
        SafetyKernel::new(Vec::new())
            .validate_decision(&stdio, &stdio_decision)
            .is_err(),
        "stdio MCP must retain process obligations"
    );
}

#[tokio::test]
async fn deterministic_deny_never_invokes_risk_review_or_approval() {
    let approvals = Arc::new(RiskAutoApproval {
        prompts: AtomicUsize::new(0),
        notices: Mutex::new(Vec::new()),
    });
    let evaluator = Arc::new(StaticRiskEvaluator {
        calls: AtomicUsize::new(0),
        assessment: Some(RiskAssessment {
            risk_level: RiskLevel::Low,
            recommended_decision: RiskRecommendation::Allow,
            reason: "would allow if policy requested approval".into(),
        }),
    });
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(BuiltInPolicy::offline_default().with_sandbox("native", "risk-deny-test", false)),
        Arc::clone(&approvals) as Arc<dyn ApprovalProvider>,
        SafetyKernel::new(["shell.run".into()]),
        [44_u8; 32],
    );
    let evaluator_port: Arc<dyn RiskEvaluator> = evaluator.clone();
    gateway
        .bind_risk_evaluator(Arc::downgrade(&evaluator_port))
        .expect("bind evaluator");
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let executable = std::env::current_exe().expect("current executable");

    assert!(matches!(
        gateway
            .execute(
                shell_request(&executable, std::env::temp_dir().as_path()),
                &executor
            )
            .await,
        Err(GatewayError::Denied(_))
    ));
    assert_eq!(evaluator.calls.load(Ordering::Acquire), 0);
    assert_eq!(approvals.prompts.load(Ordering::Acquire), 0);
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn callers_cannot_spoof_post_effect_phase_to_bypass_approval() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("filesystem.write", DecisionOutcome::RequireApproval)
        .with_filesystem_root("/tmp", "write");
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(super::DenyApproval),
        SafetyKernel::new([]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let mut request = effect_request(
        system_actor("test"),
        "filesystem.write",
        "/tmp/x",
        serde_json::json!({"content":"x"}),
    );
    request.phase = colossus_contracts::EffectPhase::PostEffect;
    assert!(matches!(
        gateway.execute(request, &executor).await,
        Err(GatewayError::Safety(_))
    ));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    assert!(journal.read_global(1, 10).expect("events").is_empty());
}

struct PostDenyPolicy;

#[async_trait]
impl colossus_ports::PolicyDecisionPoint for PostDenyPolicy {
    async fn decide(
        &self,
        request: &colossus_contracts::EffectRequest,
    ) -> Result<colossus_contracts::PolicyDecision, colossus_ports::PolicyError> {
        let mut obligations = super::default_obligations();
        obligations.require_post_effect = true;
        Ok(colossus_contracts::PolicyDecision {
            decision_id: uuid::Uuid::now_v7().to_string(),
            policy_revision: "test".into(),
            outcome: if request.phase == colossus_contracts::EffectPhase::PostEffect {
                DecisionOutcome::Deny
            } else {
                DecisionOutcome::Allow
            },
            reason: "test".into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<serde_json::Value, colossus_ports::PolicyError> {
        Ok(serde_json::json!({"ready":true}))
    }
}

#[derive(Clone)]
struct CategoryPostDenyPolicy {
    filesystem: Vec<colossus_contracts::FilesystemGrant>,
    network_destinations: Vec<String>,
}

#[async_trait]
impl colossus_ports::PolicyDecisionPoint for CategoryPostDenyPolicy {
    async fn decide(
        &self,
        request: &colossus_contracts::EffectRequest,
    ) -> Result<colossus_contracts::PolicyDecision, colossus_ports::PolicyError> {
        let mut obligations = super::default_obligations();
        obligations.sandbox_backend = "native".into();
        obligations.sandbox_profile = "post-deny-conformance".into();
        obligations.filesystem = self.filesystem.clone();
        obligations.network_destinations = self.network_destinations.clone();
        obligations.require_post_effect = true;
        Ok(colossus_contracts::PolicyDecision {
            decision_id: uuid::Uuid::now_v7().to_string(),
            policy_revision: "post-deny-conformance-v1".into(),
            outcome: if request.phase == colossus_contracts::EffectPhase::PostEffect {
                DecisionOutcome::Deny
            } else {
                DecisionOutcome::Allow
            },
            reason: "post-effect conformance decision".into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<serde_json::Value, colossus_ports::PolicyError> {
        Ok(serde_json::json!({"ready": true}))
    }
}

struct SecretExecutor {
    calls: AtomicUsize,
    secret: String,
}

#[async_trait]
impl EffectExecutor for SecretExecutor {
    async fn execute(
        &self,
        _request: &colossus_contracts::EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(QuarantinedEffectResult {
            media_type: "text/plain".into(),
            bytes: self.secret.as_bytes().to_vec(),
            effect_succeeded: true,
        })
    }
}

#[tokio::test]
async fn every_sensitive_output_category_denies_after_execution_before_actor_release() {
    let directory = tempfile::tempdir().expect("directory");
    let file = directory.path().join("private.txt");
    std::fs::write(&file, "filesystem-private-content").expect("file fixture");
    let executable = std::env::current_exe()
        .expect("current executable")
        .canonicalize()
        .expect("canonical executable");
    let origin = "https://example.test";
    let policy = CategoryPostDenyPolicy {
        filesystem: vec![
            colossus_contracts::FilesystemGrant {
                root: directory.path().display().to_string(),
                mode: "read".into(),
            },
            colossus_contracts::FilesystemGrant {
                root: executable.display().to_string(),
                mode: "execute".into(),
            },
        ],
        network_destinations: vec![origin.into()],
    };
    let requests = [
        effect_request(
            system_actor("filesystem-post-deny"),
            "filesystem.read",
            file.display().to_string(),
            serde_json::json!({}),
        ),
        effect_request(
            system_actor("network-post-deny"),
            "network.http",
            format!("{origin}/private"),
            serde_json::json!({"method": "GET"}),
        ),
        effect_request(
            system_actor("provider-post-deny"),
            "provider.openai.responses",
            format!("{origin}/v1/responses"),
            serde_json::json!({"prompt": "private"}),
        ),
        effect_request(
            system_actor("process-post-deny"),
            "process.spawn",
            executable.display().to_string(),
            serde_json::json!({
                "cwd": directory.path(),
                "environment": {},
            }),
        ),
        effect_request(
            system_actor("memory-post-deny"),
            "memory.search",
            "session:test",
            serde_json::json!({"query": "private"}),
        ),
    ];

    for request in requests {
        let action = request.action.clone();
        let secret = format!("{action}-private-content");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy.clone()),
            Arc::new(AllowApproval {
                approved_by: "operator".into(),
            }),
            SafetyKernel::new([]),
            [33_u8; 32],
        );
        let executor = SecretExecutor {
            calls: AtomicUsize::new(0),
            secret: secret.clone(),
        };
        let error = gateway
            .execute(request, &executor)
            .await
            .expect_err(&action);
        assert!(
            matches!(error, GatewayError::Denied(_)),
            "{action}: {error}"
        );
        assert_eq!(executor.calls.load(Ordering::Acquire), 1, "{action}");
        assert!(!error.to_string().contains(&secret), "{action}");

        let events = journal.read_global(1, 30).expect(&action);
        let event_types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"effect.started.v1"), "{action}");
        assert!(
            event_types.contains(&"effect.release_requested.v1"),
            "{action}"
        );
        assert!(
            event_types.contains(&"effect.release_denied.v1"),
            "{action}"
        );
        assert!(!event_types.contains(&"effect.completed.v1"), "{action}");
        assert!(
            !event_types.contains(&"effect.chunk_released.v1"),
            "{action}"
        );
        assert!(
            !serde_json::to_string(&events)
                .expect("event evidence")
                .contains(&secret),
            "{action}"
        );
    }
}

#[tokio::test]
async fn denied_post_effect_content_is_not_released() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        journal,
        Arc::new(PostDenyPolicy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new([]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let error = gateway
        .execute(
            effect_request(
                system_actor("test"),
                "provider.remote",
                "https://example.test",
                serde_json::json!({"prompt":"x"}),
            ),
            &executor,
        )
        .await
        .expect_err("post deny");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert_eq!(executor.calls.load(Ordering::Acquire), 1);
}

struct OneChunkExecutor;

#[async_trait]
impl StreamingEffectExecutor for OneChunkExecutor {
    async fn execute_stream(
        &self,
        _request: &colossus_contracts::EffectRequest,
        _permit: ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let result = QuarantinedEffectResult {
            media_type: "text/plain".into(),
            bytes: b"must-not-release".to_vec(),
            effect_succeeded: true,
        };
        let _ignored = observer.observe(result.clone()).await;
        Ok(result)
    }
}

#[derive(Default)]
struct CountingReleasedObserver(usize);

#[async_trait]
impl ReleasedEffectObserver for CountingReleasedObserver {
    async fn observe(&mut self, _result: ReleasedEffectResult) -> Result<(), ExecutionError> {
        self.0 = self.0.saturating_add(1);
        Ok(())
    }
}

#[tokio::test]
async fn denied_stream_chunk_is_latched_even_when_adapter_ignores_sink_error() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(PostDenyPolicy),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new([]),
        [6_u8; 32],
    );
    let mut observer = CountingReleasedObserver::default();
    let error = gateway
        .execute_stream(
            effect_request(
                system_actor("test"),
                "provider.remote",
                "https://example.test",
                serde_json::json!({"prompt":"x"}),
            ),
            &OneChunkExecutor,
            &mut observer,
        )
        .await
        .expect_err("post-effect policy must deny the stream chunk");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert_eq!(observer.0, 0);
    assert!(
        journal
            .read_global(1, 20)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "effect.release_denied.v1")
    );
}

struct RecordingPolicy {
    request: Arc<Mutex<Option<colossus_contracts::EffectRequest>>>,
}

#[async_trait]
impl colossus_ports::PolicyDecisionPoint for RecordingPolicy {
    async fn decide(
        &self,
        request: &colossus_contracts::EffectRequest,
    ) -> Result<colossus_contracts::PolicyDecision, colossus_ports::PolicyError> {
        *self.request.lock().expect("recording policy lock") = Some(request.clone());
        Ok(colossus_contracts::PolicyDecision {
            decision_id: uuid::Uuid::now_v7().to_string(),
            policy_revision: "recording-v1".into(),
            outcome: DecisionOutcome::Allow,
            reason: "test allow".into(),
            obligations: super::default_obligations(),
        })
    }

    async fn doctor(&self) -> Result<serde_json::Value, colossus_ports::PolicyError> {
        Ok(serde_json::json!({"ready":true}))
    }
}

#[tokio::test]
async fn hard_secrets_are_hashed_and_structured_credential_references_are_preserved() {
    let seen = Arc::new(Mutex::new(None));
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(RecordingPolicy {
            request: Arc::clone(&seen),
        }),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new([]),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    gateway
        .execute(
            effect_request(
                system_actor("test"),
                "provider.remote",
                "provider:test",
                serde_json::json!({
                    "message": "safe",
                    "api_key": "must-not-leak",
                    "headers": {"authorization": "Bearer secret"},
                    "credential_references": {"password": "env:SAFE_PASSWORD_REF"},
                    "credential_headers": {
                        "Authorization": {
                            "scheme": "Bearer",
                            "reference": "env:SPLUNK_MCP_TOKEN"
                        }
                    },
                    "invalid_credential_header": {
                        "authorization": {
                            "scheme": "Bearer",
                            "reference": "env:SPLUNK_MCP_TOKEN",
                            "value": "must-not-leak"
                        }
                    },
                    "password": {
                        "scheme": "must-not-leak-scheme",
                        "reference": "env:SPLUNK_MCP_TOKEN"
                    },
                    "arguments": {
                        "credential_headers": {
                            "password": {
                                "scheme": "must-not-leak-scheme",
                                "reference": "env:SPLUNK_MCP_TOKEN"
                            }
                        }
                    }
                }),
            ),
            &executor,
        )
        .await
        .expect("allowed");
    let request = seen
        .lock()
        .expect("seen lock")
        .clone()
        .expect("policy request");
    assert_eq!(request.content["message"], "safe");
    assert_eq!(request.content["api_key"]["redacted"], true);
    assert_eq!(
        request.content["headers"]["authorization"]["redacted"],
        true
    );
    assert_eq!(
        request.content["credential_references"]["password"],
        "env:SAFE_PASSWORD_REF"
    );
    assert_eq!(
        request.content["credential_headers"]["Authorization"],
        serde_json::json!({
            "scheme": "Bearer",
            "reference": "env:SPLUNK_MCP_TOKEN"
        })
    );
    assert_eq!(
        request.content["invalid_credential_header"]["authorization"]["redacted"],
        true
    );
    assert_eq!(request.content["password"]["redacted"], true);
    assert_eq!(
        request.content["arguments"]["credential_headers"]["password"]["redacted"],
        true
    );
    assert!(
        !serde_json::to_string(&request)
            .expect("request json")
            .contains("must-not-leak")
    );
}

#[tokio::test]
async fn oversized_request_is_audited_and_fails_closed() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(AllowApproval {
            approved_by: "user".into(),
        }),
        SafetyKernel::new([]).with_policy_input_limit(256),
        [9_u8; 32],
    );
    let executor = CountingExecutor {
        calls: AtomicUsize::new(0),
    };
    let error = gateway
        .execute(
            effect_request(
                system_actor("test"),
                "provider.echo",
                "provider:echo",
                serde_json::json!({"message": "x".repeat(1024)}),
            ),
            &executor,
        )
        .await
        .expect_err("oversized deny");
    assert!(matches!(error, GatewayError::Policy(_)));
    assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    let names = journal
        .read_global(1, 10)
        .expect("audit events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["effect.requested.v1", "effect.denied.v1"]);
}

fn one_shot_opa(response: serde_json::Value) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("OPA test listener");
    let address = listener.local_addr().expect("OPA test address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("OPA request");
        let mut request = [0_u8; 16 * 1024];
        let read = stream.read(&mut request).expect("read OPA request");
        assert!(String::from_utf8_lossy(&request[..read]).contains("/v1/data/colossus/effect"));
        let body = serde_json::to_vec(&response).expect("OPA response JSON");
        write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("OPA response headers");
        stream.write_all(&body).expect("OPA response body");
    });
    (format!("http://{address}/"), handle)
}

fn local_opa_config(base_url: String) -> super::OpaConfig {
    super::OpaConfig {
        base_url,
        decision_path: "colossus/effect".into(),
        ca_pem: None,
        tls_roots: Default::default(),
        identity_pem: None,
        full_content_disclosure_acknowledged: true,
        decision_log_masking_verified: false,
        timeout: Duration::from_secs(2),
    }
}

#[tokio::test]
async fn opa_adapter_accepts_strict_decisions_and_rejects_invalid_responses() {
    let decision = colossus_contracts::PolicyDecision {
        decision_id: "opa-decision".into(),
        policy_revision: "bundle-42".into(),
        outcome: DecisionOutcome::Allow,
        reason: "test".into(),
        obligations: super::default_obligations(),
    };
    let (url, server) = one_shot_opa(serde_json::json!({"result": decision}));
    let policy = super::OpaPolicy::new(local_opa_config(url)).expect("OPA policy");
    let result = colossus_ports::PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("test"),
            "provider.echo",
            "provider:echo",
            serde_json::json!({"message":"ok"}),
        ),
    )
    .await
    .expect("strict OPA decision");
    server.join().expect("OPA server");
    assert_eq!(result.policy_revision, "bundle-42");

    let (url, server) = one_shot_opa(serde_json::json!({
        "result": {"decision_id":"missing-everything-else"}
    }));
    let policy = super::OpaPolicy::new(local_opa_config(url)).expect("OPA policy");
    let error = colossus_ports::PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("test"),
            "provider.echo",
            "provider:echo",
            serde_json::json!({"message":"ok"}),
        ),
    )
    .await
    .expect_err("invalid response");
    server.join().expect("OPA server");
    assert!(matches!(
        error,
        colossus_ports::PolicyError::InvalidDecision(_)
    ));
}

#[test]
fn remote_opa_requires_disclosure_https_pinned_trust_and_mtls() {
    let mut config = local_opa_config("https://opa.example.test/".into());
    config.full_content_disclosure_acknowledged = false;
    assert!(matches!(
        super::OpaPolicy::new(config),
        Err(colossus_ports::PolicyError::InvalidDecision(_))
    ));

    let config = local_opa_config("http://opa.example.test/".into());
    assert!(matches!(
        super::OpaPolicy::new(config),
        Err(colossus_ports::PolicyError::InvalidDecision(_))
    ));

    let config = local_opa_config("https://opa.example.test/".into());
    assert!(matches!(
        super::OpaPolicy::new(config),
        Err(colossus_ports::PolicyError::InvalidDecision(_))
    ));
}
