use super::composition::research_run_timeout_ms;
use super::extensions::extension_path;
use super::{
    AuditExporterConfig, ContextEffectExecutor, ContextToolExecutor, DiscoverableToolExecutor,
    GatewayMemoryRetriever, GatewayRiskEvaluator, GatewayToolExecutor, GatewayWorkflowEffects,
    InstructionSnapshotStore, InteractiveToolExecutor, JournalExternalWorkQueue,
    LOOPBACK_PROVIDER_TIMEOUT_MS, MAX_FILE_SUMMARY_OUTPUT_BYTES, MemoryEffectExecutor,
    MemoryEmbeddingConfig, MemoryOperation, ModelCapabilities, ModelProfileConfig,
    PackProcessDeclaration, PackProcessExecutor, PackToolEffectInput, PresentationEffectExecutor,
    PresentationOperation, ProviderProfileConfig, REMOTE_PROVIDER_TIMEOUT_MS, ReasoningEffort,
    Runtime, RuntimeConfig, RuntimeError, RuntimeOpenOptions, SearchConfig, SearchProfileConfig,
    SemanticMemoryConfig, SkillEffectExecutor, SkillOperation, SkillScaffoldResult, StorageAdapter,
    TraceToolExecutor, WorkEffectExecutor, configure_shell_environment, derive_development_sandbox,
    goal_objective_from_plan, model_resource_path, model_workspace_path, provider_profile,
    push_bounded_mcp_discovery_tool, recover_interrupted_subagents, recover_unknown_effects,
    redacted_risk_metadata, reject_reserved_shell_environment, reject_shell_startup_profiles,
    shell_command_arguments, terminal_actor,
};
use crate::test_support::private_tempdir;
use colossus_contracts::{
    Actor, ActorType, CredentialReference, DecisionOutcome, EffectPhase, EffectRequest,
    EventClassification, ExecutionContext, FilesystemGrant, GoalStatus, MemoryScope, MemoryStatus,
    ModelContent, ModelContentPart, ModelImageDetail, ModelImageReference, ModelLimits,
    ModelMessage, ModelMessageRole, ModelRequest, ModelToolCall, NewEvent, PlanRecord, PlanStatus,
    PlanStep, PolicyDecision, ProjectionBatch, ProjectionMutation, ProviderEvent,
    ProviderResponseDiagnostic, ProviderRoute, ProviderTurn, QuarantinedEffectResult,
    ResourceAuthority, RiskLevel, RiskRecommendation, RunBranchContextMode, SandboxBoundaryMode,
    SessionMessageAppend, StartupVerificationMode, SubagentStatus, TaskStatus, TerminalPreferences,
    ToolCall,
};
use colossus_home::{ColossusHome, HomeSurface, detect_workspace_identity};
use colossus_mcp::{
    MAX_MCP_TOOLS, McpCredentialHeaderConfig, McpOAuthConfig, McpResearchToolConfig,
    McpServerConfig, McpToolSummary, McpTransportKind,
};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS, SafetyKernel,
    effect_request,
};
use colossus_ports::{
    EventJournal, ExternalWorkQueue, ModelProvider, ModelProviderError, PolicyDecisionPoint,
    PresentationRepository, ProjectionStore, RiskEvaluationError, RiskEvaluator, SkillRepository,
    ToolExecutor,
};
use colossus_presentation::EventSourcedPresentationRepository;
use colossus_projection::EFFECT_RECOVERY_PROJECTION;
use colossus_provider::{ChatCompletionsOutputTokenParameter, ProviderKind};
use colossus_skills::{
    FilesystemSkillRepository, SkillAuthoringService, SkillResourceService, SkillRoot,
};
use colossus_testkit::{InMemoryEventJournal, InMemoryProjectionStore};
use colossus_tools::MCP_TOOLS_MAX_OUTPUT_BYTES;
use colossus_workflow::{WorkflowEffect, WorkflowEffectRunner};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::tempdir;

#[test]
fn research_outer_timeout_contains_every_bounded_nested_operation() {
    assert_eq!(research_run_timeout_ms(300_000, 30_000, 20, 4), 6_750_000);
    assert!(research_run_timeout_ms(300_000, 30_000, 20, 4) > 30_000);
}

#[test]
fn image_capability_echo_and_bounds_fail_at_the_pre_effect_route_gate() {
    let image = ModelImageReference {
        artifact_id: format!("artifact-{}", "a".repeat(64)),
        file_name: "input.png".into(),
        media_type: "image/png".into(),
        size_bytes: 1,
        sha256: "b".repeat(64),
        width_pixels: 640,
        height_pixels: 480,
        detail: ModelImageDetail::Auto,
    };
    let request = ModelRequest {
        instructions: "test".into(),
        messages: vec![ModelMessage {
            role: ModelMessageRole::User,
            content: ModelContent::Parts(vec![ModelContentPart::Image {
                image: image.clone(),
            }]),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        tools: Vec::new(),
        max_output_tokens: None,
    };
    let mut route = ProviderRoute {
        role: "primary".into(),
        profile: "vision".into(),
        model_profile: "vision".into(),
        provider_profile: "openai".into(),
        provider: "open_ai_responses".into(),
        model: "gpt-5.6-sol".into(),
        limits: ModelLimits {
            context_window_tokens: 128_000,
            max_output_tokens: 8_192,
            safety_margin_tokens: 8_192,
            input_budget_tokens: 111_616,
        },
        capabilities: ModelCapabilities {
            tool_calls: true,
            streaming: true,
            image_inputs: false,
        },
        reasoning_effort: None,
    };

    let error = super::provider_gateway::validate_route_image_inputs(
        &route,
        ProviderKind::OpenAiResponses,
        &request,
    )
    .expect_err("capability gate");
    assert!(error.to_string().contains("does not enable image inputs"));

    route.capabilities.image_inputs = true;
    let error =
        super::provider_gateway::validate_route_image_inputs(&route, ProviderKind::Echo, &request)
            .expect_err("Echo gate");
    assert!(error.to_string().contains("Echo provider"));

    let mut oversized = request;
    let ModelContent::Parts(parts) = &mut oversized.messages[0].content else {
        panic!("multipart image request");
    };
    let ModelContentPart::Image { image } = &mut parts[0] else {
        panic!("image part");
    };
    image.size_bytes = 16 * 1_048_576 + 1;
    let error = super::provider_gateway::validate_route_image_inputs(
        &route,
        ProviderKind::OpenAiResponses,
        &oversized,
    )
    .expect_err("image bound");
    assert!(error.to_string().contains("dimension, or pixel bound"));
}

fn external_work_queue(journal: Arc<dyn EventJournal>) -> Arc<dyn ExternalWorkQueue> {
    let store: Arc<dyn ProjectionStore> = Arc::new(InMemoryProjectionStore::default());
    Arc::new(JournalExternalWorkQueue::new(journal, store))
}

fn configure_primary_model(
    config: &mut RuntimeConfig,
    profile: &str,
    provider_profile: &str,
    model: &str,
) {
    config.models.profiles.insert(
        profile.into(),
        ModelProfileConfig {
            provider_profile: provider_profile.into(),
            model: model.into(),
            context_window_tokens: 32_768,
            max_output_tokens: 4_096,
            capabilities: ModelCapabilities {
                tool_calls: true,
                streaming: true,
                image_inputs: false,
            },
            reasoning_effort: None,
        },
    );
    config.models.roles.insert("primary".into(), profile.into());
}

struct SecretEchoProcess;

struct PrivateOutputProcess;

struct RuntimePostDenyPolicy(BuiltInPolicy);

struct RecordingDenyPolicy {
    inner: BuiltInPolicy,
    requests: Arc<Mutex<Vec<EffectRequest>>>,
}

#[async_trait::async_trait]
impl PolicyDecisionPoint for RuntimePostDenyPolicy {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<PolicyDecision, colossus_ports::PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "runtime content denied by post-effect policy".into();
        }
        Ok(decision)
    }

    async fn doctor(&self) -> Result<Value, colossus_ports::PolicyError> {
        self.0.doctor().await
    }
}

#[async_trait::async_trait]
impl PolicyDecisionPoint for RecordingDenyPolicy {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<PolicyDecision, colossus_ports::PolicyError> {
        self.requests
            .lock()
            .expect("recorded policy requests")
            .push(request.clone());
        self.inner.decide(request).await
    }

    async fn doctor(&self) -> Result<Value, colossus_ports::PolicyError> {
        self.inner.doctor().await
    }
}

struct UnusedToolExecutor;

struct FixedUserPrompt;

#[test]
fn provider_diagnostic_display_prioritizes_response_and_dotted_tool_names() {
    let diagnostic = ProviderResponseDiagnostic {
        request_method: "POST".into(),
        request_url: "http://127.0.0.1:9000/v1/chat/completions".into(),
        request_body: Some(json!({
            "tools": [
                {"type": "function", "function": {"name": "tool.with.dots"}},
                {"type": "function", "name": "responses.tool"}
            ],
            "messages": [{"role": "tool", "content": "continuation"}]
        })),
        status: 400,
        content_type: Some("application/json".into()),
        body: r#"{"error":"dotted tool rejected"}"#.into(),
        body_encoding: "utf8".into(),
        body_truncated: false,
    };
    let rendered = super::format_provider_response_diagnostic(&diagnostic);
    assert!(rendered.contains("HTTP 400"));
    assert!(rendered.contains(r#"{"error":"dotted tool rejected"}"#));
    assert!(rendered.contains("tool.with.dots, responses.tool"));
    assert!(rendered.contains("\"role\": \"tool\""));

    let error = super::RuntimeError::Agent(super::AgentError::Provider(
        ModelProviderError::ResponseDiagnostic {
            diagnostic: Box::new(diagnostic),
        },
    ));
    assert_eq!(
        error
            .provider_response_diagnostic()
            .map(|diagnostic| diagnostic.status),
        Some(400)
    );
    assert!(!error.to_string().contains("dotted tool rejected"));
}

#[tokio::test]
async fn provider_diagnostic_roles_exclude_automatic_agent_instructions() {
    const HOME_SENTINEL: &str = "private-home-agents-diagnostic-sentinel";
    const WORKSPACE_SENTINEL: &str = "private-workspace-agents-diagnostic-sentinel";

    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let home = ColossusHome::ensure_at(root.join("home")).expect("Colossus home");
    fs::write(home.root().join("AGENTS.md"), HOME_SENTINEL).expect("home instructions");
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    fs::write(workspace.join("AGENTS.md"), WORKSPACE_SENTINEL).expect("workspace instructions");

    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.providers.profiles.insert(
        "diagnostic".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("https://diagnostics.invalid/v1".into()),
            credential_reference: None,
            timeout_ms: Some(1_000),
            chat_completions_output_token_parameter: None,
        },
    );
    configure_primary_model(&mut config, "diagnostic", "diagnostic", "diagnostic-model");
    config
        .sandbox
        .network_destinations
        .push("https://diagnostics.invalid".into());
    let mut runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(&workspace)
            .expect("workspace options")
            .with_colossus_home(home.root())
            .expect("home binding"),
    )
    .expect("runtime");

    let requests = Arc::new(Mutex::new(Vec::new()));
    runtime.gateway = Arc::new(EffectGateway::new(
        Arc::clone(&runtime.journal),
        Arc::new(RecordingDenyPolicy {
            inner: BuiltInPolicy::offline_default(),
            requests: Arc::clone(&requests),
        }),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [29_u8; 32],
    ));

    runtime
        .provider_doctor(Some("diagnostic"))
        .await
        .expect("provider diagnostic result");
    runtime
        .model_doctor(Some("diagnostic"))
        .await
        .expect("model diagnostic result");

    let requests = requests.lock().expect("recorded policy requests");
    assert_eq!(requests.len(), 2);
    let provider_probe = &requests[0];
    assert_eq!(provider_probe.actor.id, "provider-diagnostics");
    assert_eq!(provider_probe.action, "provider.models");
    assert!(provider_probe.content["request"].is_null());
    let model_probe = &requests[1];
    assert_eq!(model_probe.actor.id, "model-diagnostics");
    assert_eq!(model_probe.action, "provider.openai.chat");
    assert_eq!(
        model_probe.content["request"]["instructions"],
        "This is a model readiness probe. Reply with exactly: ok"
    );
    for request in requests.iter() {
        let content = request.content.to_string();
        assert!(!content.contains(HOME_SENTINEL));
        assert!(!content.contains(WORKSPACE_SENTINEL));
    }
}

#[cfg(unix)]
#[test]
fn long_worker_ipc_endpoints_use_stable_private_short_paths() {
    let long_root = std::path::PathBuf::from("/tmp")
        .join("managed-local")
        .join("a".repeat(160));
    let mut config = RuntimeConfig::offline_template(long_root.join("state.redb"));
    let workspace = std::path::Path::new("/tmp/colossus-worker-endpoint-test");
    let first = config
        .worker_ipc_endpoint_at(workspace)
        .expect("first short endpoint");
    let repeated = config
        .worker_ipc_endpoint_at(workspace)
        .expect("stable short endpoint");
    assert_eq!(first, repeated);

    let first = std::path::PathBuf::from(first);
    assert_eq!(
        first.parent(),
        Some(super::workspace_lease::worker_coordination_root().as_path())
    );
    assert!(std::os::unix::net::SocketAddr::from_pathname(&first).is_ok());
    assert_eq!(
        first
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::len),
        Some("ipc-v2-".len() + 43 + ".sock".len())
    );
    assert!(!first.to_string_lossy().contains(&"a".repeat(32)));

    config.storage.path = long_root.join("other-state.redb");
    let second = config
        .worker_ipc_endpoint_at(workspace)
        .expect("distinct short endpoint");
    assert_ne!(first, std::path::PathBuf::from(second));
}

#[cfg(unix)]
#[test]
fn short_worker_ipc_endpoint_keeps_the_state_adjacent_contract() {
    let state = std::path::PathBuf::from("/tmp/colossus-short-state.redb");
    let config = RuntimeConfig::offline_template(&state);
    assert_eq!(
        config
            .worker_ipc_endpoint_at(std::path::Path::new("/tmp/workspace"))
            .expect("worker endpoint"),
        format!("{}.worker.sock", state.display())
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_worker_state_paths_hash_native_bytes_without_aliasing() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    let workspace = std::path::Path::new("/tmp/workspace");
    let state = |byte| {
        let mut path = b"/tmp/colossus-non-utf8-".to_vec();
        path.push(byte);
        path.extend_from_slice(b".redb");
        std::path::PathBuf::from(OsString::from_vec(path))
    };
    let first = RuntimeConfig::offline_template(state(0x80))
        .worker_ipc_endpoint_at(workspace)
        .expect("first native-byte endpoint");
    let second = RuntimeConfig::offline_template(state(0x81))
        .worker_ipc_endpoint_at(workspace)
        .expect("second native-byte endpoint");

    assert_ne!(first, second);
    assert_eq!(
        std::path::Path::new(&first).parent(),
        Some(super::workspace_lease::worker_coordination_root().as_path())
    );
    assert!(!first.contains(char::REPLACEMENT_CHARACTER));
    assert!(!second.contains(char::REPLACEMENT_CHARACTER));
}

#[tokio::test]
async fn subworkflow_start_and_compensation_are_independent_gateway_effects() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("workflow.start", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["workflow.execute".into()]),
        [44_u8; 32],
    );
    let runner = GatewayWorkflowEffects {
        gateway: Arc::new(gateway),
        agent: None,
        agent_max_turns: 1,
    };
    for compensation in [false, true] {
        runner
            .run(WorkflowEffect {
                kind: "workflow".into(),
                action: "workflow.start".into(),
                content: json!({"workflow": "child", "version": "1.0.0", "inputs": {}}),
                idempotency: Some(format!("call-{compensation}")),
                credential_references: Vec::new(),
                allowed_tools: Vec::new(),
                run_id: "parent-run".into(),
                step_id: if compensation {
                    "rollback-child".into()
                } else {
                    "launch-child".into()
                },
                definition_step_id: if compensation {
                    "rollback-child".into()
                } else {
                    "launch-child".into()
                },
                workflow_hash: "parent-hash".into(),
                attempt: 1,
                compensation,
            })
            .await
            .expect("authorized workflow control effect");
    }
    let resources = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .filter(|event| event.event_type == "effect.requested.v1")
        .map(|event| {
            journal.decrypt_payload(&event).expect("payload")["resource"]
                .as_str()
                .expect("resource")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resources,
        [
            "workflow-step:launch-child",
            "workflow-compensation-step:rollback-child"
        ]
    );
}

#[tokio::test]
async fn webhook_ingress_uses_gateway_with_a_credential_reference_only() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("workflow.webhook.ingest", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["workflow.execute".into()]),
        [45_u8; 32],
    );
    GatewayWorkflowEffects {
        gateway: Arc::new(gateway),
        agent: None,
        agent_max_turns: 1,
    }
    .run(WorkflowEffect {
        kind: "workflow".into(),
        action: "workflow.webhook.ingest".into(),
        content: json!({"body_sha256": "digest", "body": {"event": "push"}}),
        idempotency: Some("webhook:hook:delivery".into()),
        credential_references: vec![CredentialReference {
            reference: "env:COLOSSUS_WEBHOOK_SECRET".into(),
            value_hash: Some("key-digest".into()),
        }],
        allowed_tools: Vec::new(),
        run_id: "webhook-run".into(),
        step_id: "$webhook".into(),
        definition_step_id: "$webhook".into(),
        workflow_hash: "workflow-digest".into(),
        attempt: 1,
        compensation: false,
    })
    .await
    .expect("authorized webhook ingress");

    let requested = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .find(|event| event.event_type == "effect.requested.v1")
        .expect("requested event");
    let payload = journal.decrypt_payload(&requested).expect("payload");
    assert_eq!(payload["action"], "workflow.webhook.ingest");
    assert_eq!(
        payload["credential_references"][0]["reference"],
        "env:COLOSSUS_WEBHOOK_SECRET"
    );
    assert!(!payload.to_string().contains("actual-secret-value"));
}

#[tokio::test]
async fn subscription_dispatch_uses_the_ordinary_gateway() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("workflow.subscription.dispatch", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["workflow.execute".into()]),
        [46_u8; 32],
    );
    GatewayWorkflowEffects {
        gateway: Arc::new(gateway),
        agent: None,
        agent_max_turns: 1,
    }
    .run(WorkflowEffect {
        kind: "workflow".into(),
        action: "workflow.subscription.dispatch".into(),
        content: json!({
            "subscription_id": "new-tasks",
            "event": {"event_id": "event-1", "payload": {"title": "review"}},
        }),
        idempotency: Some("subscription:new-tasks:event-1".into()),
        credential_references: Vec::new(),
        allowed_tools: Vec::new(),
        run_id: "subscription-run".into(),
        step_id: "$subscription".into(),
        definition_step_id: "$subscription".into(),
        workflow_hash: "workflow-digest".into(),
        attempt: 1,
        compensation: false,
    })
    .await
    .expect("authorized subscription dispatch");

    let requested = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .find(|event| event.event_type == "effect.requested.v1")
        .expect("requested event");
    let payload = journal.decrypt_payload(&requested).expect("payload");
    assert_eq!(payload["action"], "workflow.subscription.dispatch");
    let content_fields = payload["content_fields"]
        .as_array()
        .expect("content fields");
    assert_eq!(content_fields.len(), 2);
    assert!(content_fields.contains(&json!("event")));
    assert!(content_fields.contains(&json!("subscription_id")));
}

#[tokio::test]
async fn workflow_agent_steps_use_the_normal_agent_runtime_with_durable_lineage() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let provider = Arc::new(WorkScriptedProvider {
        turns: Mutex::new(VecDeque::from([ProviderTurn {
            profile: "scripted".into(),
            model_profile: "scripted".into(),
            provider_profile: "scripted-provider".into(),
            provider: "test".into(),
            model: "test-model".into(),
            response_id: Some("workflow-response".into()),
            events: vec![ProviderEvent::FinalOutput {
                text: "workflow agent finished".into(),
            }],
        }])),
        requests: Mutex::new(Vec::new()),
    });
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    let agent = Arc::new(colossus_agent::AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            colossus_tools::StaticToolRegistry::new(
                colossus_tools::builtin_specs()
                    .into_iter()
                    .filter(|tool| tool.name == "echo"),
            )
            .expect("workflow tool registry"),
        ),
        Arc::new(UnusedToolExecutor),
        sessions,
    ));
    let gateway = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("agent.run", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["workflow.execute".into()]),
        [47_u8; 32],
    );
    let released = GatewayWorkflowEffects {
        gateway: Arc::new(gateway),
        agent: Some(agent),
        agent_max_turns: 2,
    }
    .run(WorkflowEffect {
        kind: "agent".into(),
        action: "agent.run".into(),
        content: json!({"prompt": "Review the release boundary"}),
        idempotency: None,
        credential_references: Vec::new(),
        allowed_tools: vec!["echo".into()],
        run_id: "workflow-run-agent".into(),
        step_id: "agent-review".into(),
        definition_step_id: "agent-review".into(),
        workflow_hash: "workflow-hash-agent".into(),
        attempt: 2,
        compensation: false,
    })
    .await
    .expect("workflow agent step");

    let agent_result: Value =
        serde_json::from_str(released["text"].as_str().expect("released JSON text"))
            .expect("agent result JSON");
    assert_eq!(agent_result["output"], "workflow agent finished");
    assert_eq!(
        provider.requests.lock().expect("requests")[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["echo"],
        "the pinned workflow capability ceiling is the agent's exact tool ceiling"
    );
    let model_event = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .find(|event| event.event_type == "model.request.prepared.v1")
        .expect("model request event");
    assert_eq!(
        model_event.context.workflow_id.as_deref(),
        Some("workflow-run-agent")
    );
    assert_eq!(
        model_event.context.workflow_hash.as_deref(),
        Some("workflow-hash-agent")
    );
    assert_eq!(model_event.context.step_id.as_deref(), Some("agent-review"));
    assert_eq!(model_event.context.attempt, Some(2));
    assert_eq!(model_event.context.offered_tools, vec!["echo"]);
}

#[tokio::test]
async fn presentation_mutation_is_denied_before_repository_and_allowed_with_permit() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn PresentationRepository> = Arc::new(
        EventSourcedPresentationRepository::new(Arc::clone(&journal)),
    );
    let executor = PresentationEffectExecutor {
        repository: Arc::clone(&repository),
    };
    let preferences = TerminalPreferences {
        theme: colossus_contracts::ThemeName::HighContrast,
        ..TerminalPreferences::default()
    };
    let operation = PresentationOperation::Save {
        preferences: preferences.clone(),
    };
    let request = || {
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            "presentation:repl",
            serde_json::to_value(&operation).expect("operation"),
        );
        request.capabilities = vec![operation.action().into()];
        request
    };

    let denied_gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new([operation.action().into()]),
        [61_u8; 32],
    );
    assert!(denied_gateway.execute(request(), &executor).await.is_err());
    assert_eq!(
        repository.load().expect("unchanged"),
        TerminalPreferences::default()
    );

    let allowed_gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(operation.action(), DecisionOutcome::Allow),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new([operation.action().into()]),
        [62_u8; 32],
    );
    allowed_gateway
        .execute(request(), &executor)
        .await
        .expect("authorized update");
    assert_eq!(repository.load().expect("updated"), preferences);
    assert_eq!(
        journal
            .read_stream("presentation:repl")
            .expect("preference stream")
            .len(),
        1
    );

    let history_operation = PresentationOperation::AppendHistory {
        entry: "secret prompt".into(),
    };
    let mut history_request = effect_request(
        terminal_actor(),
        history_operation.action(),
        history_operation.resource(),
        serde_json::to_value(&history_operation).expect("history operation"),
    );
    history_request.capabilities = vec![history_operation.action().into()];
    let history_gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(history_operation.action(), DecisionOutcome::Allow),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new([history_operation.action().into()]),
        [63_u8; 32],
    );
    history_gateway
        .execute(history_request, &executor)
        .await
        .expect("authorized encrypted history append");
    assert_eq!(
        repository.list_history(10).expect("history"),
        ["secret prompt"]
    );
    let history_event = journal
        .read_stream("presentation:history")
        .expect("history stream")
        .into_iter()
        .next()
        .expect("history event");
    assert_eq!(history_event.event_type, "presentation.history.appended.v1");
    assert!(
        !serde_json::to_string(&history_event)
            .expect("history envelope")
            .contains("secret prompt")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn interactive_presentation_mutations_retain_the_session_boundary_acknowledgement() {
    use std::os::unix::fs::PermissionsExt as _;

    let temporary = tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let home = ColossusHome::ensure_at(root.join("home")).expect("Colossus home");
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.sandbox.backend = "danger_full_access".into();
    config.sandbox.acknowledge_danger_full_access = false;
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(&workspace)
            .expect("workspace options")
            .with_colossus_home(home.root())
            .expect("home binding"),
    )
    .expect("runtime");
    let session = runtime
        .create_session(Some("interactive presentation"))
        .expect("session");

    let unacknowledged = runtime
        .append_terminal_history_for_session(&session.id, "before acknowledgement")
        .await
        .expect_err("unacknowledged history must fail");
    assert!(
        unacknowledged.to_string().contains("is not acknowledged"),
        "{unacknowledged}"
    );

    runtime
        .acknowledge_sandbox_boundary(&session.id, SandboxBoundaryMode::DangerFullAccess)
        .expect("session acknowledgement");
    assert!(
        runtime
            .append_terminal_history("missing session binding")
            .await
            .is_err(),
        "a session acknowledgement must not become process-global"
    );
    runtime
        .append_terminal_history_for_session(&session.id, "persisted input")
        .await
        .expect("session-bound history");
    let preferences = TerminalPreferences {
        theme: colossus_contracts::ThemeName::HighContrast,
        ..TerminalPreferences::default()
    };
    assert_eq!(
        runtime
            .save_presentation_preferences_for_session(&session.id, preferences.clone())
            .await
            .expect("session-bound preferences"),
        preferences
    );
    assert_eq!(
        runtime.terminal_history(10).expect("history"),
        ["persisted input"]
    );
}

#[async_trait::async_trait]
impl colossus_ports::UserPromptProvider for FixedUserPrompt {
    async fn prompt(
        &self,
        request: colossus_contracts::UserPromptRequest,
    ) -> Result<colossus_contracts::UserPromptResponse, colossus_ports::ToolError> {
        assert_eq!(request.question, "Choose a runtime");
        assert_eq!(request.choices, ["Rust", "Python"]);
        assert!(!request.allow_free_form);
        Ok(colossus_contracts::UserPromptResponse {
            answer: "Rust".into(),
            selected_index: Some(0),
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for UnusedToolExecutor {
    async fn execute(
        &self,
        _call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<colossus_contracts::ToolResult, colossus_ports::ToolError> {
        panic!("tool.search must not delegate")
    }
}

#[async_trait::async_trait]
impl colossus_policy::EffectExecutor for SecretEchoProcess {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: colossus_policy::ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, colossus_policy::ExecutionError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        let spec: colossus_sandbox::ProcessSpec =
            serde_json::from_value(request.content.clone())
                .map_err(|error| colossus_policy::ExecutionError::Failed(error.to_string()))?;
        let secret = spec.environment.get("PACK_SECRET").ok_or_else(|| {
            colossus_policy::ExecutionError::Failed("resolved secret is absent".into())
        })?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({
                "stdout_base64": BASE64.encode(secret),
                "stderr_base64": BASE64.encode([]),
                "exit_code": 0,
                "truncated": false
            }))
            .map_err(|error| colossus_policy::ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

#[async_trait::async_trait]
impl colossus_policy::EffectExecutor for PrivateOutputProcess {
    async fn execute(
        &self,
        _request: &EffectRequest,
        _permit: colossus_policy::ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, colossus_policy::ExecutionError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({
                "stdout_base64": BASE64.encode("process-private-content"),
                "stderr_base64": BASE64.encode([]),
                "exit_code": 0,
                "output_truncated": false,
            }))
            .map_err(|error| colossus_policy::ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

#[test]
fn strict_config_rejects_unknown_fields() {
    let yaml = r#"
schemaVersion: 2
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
storage:
  path: state.redb
  keys:
    kind: platform
    service: test
    journal_key_id: journal
    signing_key_id: signing
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: .colossus/workflows
  user: workflows
surprise: true
"#;
    assert!(RuntimeConfig::from_yaml(yaml).is_err());
}

#[test]
fn sparse_schema_materializes_recursive_defaults_and_show_expands_them() {
    let yaml = r#"
schemaVersion: 2
storage:
  path: state.redb
access: {}
network: {}
audit: {}
observability:
  traces:
    enabled: false
  otlp: {}
workflows:
  repository: custom-workflows
providers:
  profiles:
    echo:
      kind: echo
models: {}
agent: {}
subagents: {}
context:
  compactAtPercent: 60
memory:
  retrievalLimit: 4
research:
  maxWorkers: 2
search: {}
mcp: {}
skills:
  disabled:
    - legacy
packs: {}
sandbox:
  timeoutMs: 45000
"#;
    let config = RuntimeConfig::from_yaml(yaml).expect("sparse configuration");
    assert_eq!(
        config.access.profile,
        colossus_access::AccessProfile::AllowAll
    );
    assert!(matches!(config.policy, super::PolicyConfig::BuiltIn { .. }));
    assert_eq!(
        config.workflows.repository,
        PathBuf::from("custom-workflows")
    );
    assert_eq!(config.workflows.user, PathBuf::from("workflows"));
    assert!(config.providers.profiles.contains_key("echo"));
    assert!(config.models.profiles.contains_key("echo"));
    assert_eq!(config.agent, super::AgentConfig::default());
    assert_eq!(config.subagents, super::SubagentConfig::default());
    assert_eq!(config.context.compact_at_percent, 60);
    assert_eq!(config.context.target_percent, 45);
    assert_eq!(config.memory.retrieval_limit, 4);
    assert!(config.memory.index_enabled);
    assert_eq!(config.research.max_workers, 2);
    assert_eq!(config.research.max_sources, 20);
    assert_eq!(config.skills.disabled, vec!["legacy".to_owned()]);
    assert_eq!(config.packs.install_root, PathBuf::from(".colossus/packs"));
    assert_eq!(config.sandbox.backend, "danger_full_access");
    assert!(config.sandbox.acknowledge_danger_full_access);
    assert_eq!(config.sandbox.timeout_ms, 45_000);
    assert_eq!(config.sandbox.max_output_bytes, 4 * 1024 * 1024);
    assert_eq!(config.sandbox.max_memory_bytes, 1024 * 1024 * 1024);

    let expanded: Value = serde_saphyr::from_str(
        &config
            .to_resolved_yaml()
            .expect("expanded configuration YAML"),
    )
    .expect("expanded configuration value");
    for key in [
        "access",
        "network",
        "audit",
        "observability",
        "policy",
        "workflows",
        "providers",
        "models",
        "agent",
        "subagents",
        "context",
        "memory",
        "research",
        "search",
        "mcp",
        "skills",
        "packs",
        "sandbox",
    ] {
        assert!(expanded.get(key).is_some(), "expanded output omitted {key}");
    }
    assert_eq!(
        expanded["sandbox"]["maxOutputBytes"].as_u64(),
        Some(4_194_304)
    );
    assert_eq!(
        expanded["sandbox"]["maxMemoryBytes"].as_u64(),
        Some(1_073_741_824)
    );
}

#[test]
fn previous_expanded_schema_v2_configuration_retains_its_explicit_posture() {
    let yaml = r#"
schemaVersion: 2
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
storage:
  location: home_workspace
  path: state.redb
  adapter: redb
  startupVerification: incremental
  keys:
    kind: none
network:
  caBundlePath: null
audit:
  exporter:
    kind: disabled
observability:
  enabled: false
  serviceName: colossus
  resourceAttributes: {}
  traces:
    enabled: false
    sampleRatio: 1.0
  metrics:
    enabled: false
    exportIntervalMs: 60000
  logs:
    otlp: false
    stdoutJson: false
    journalPayloads: disabled
    acknowledgeSensitiveContent: false
  otlp:
    endpoint: null
    protocol: grpc
    timeoutMs: 10000
    acknowledgeInsecureTransport: false
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: .colossus/workflows
  user: workflows
providers:
  profiles:
    echo:
      kind: echo
      baseUrl: null
      credentialReference: null
models:
  profiles:
    echo:
      providerProfile: echo
      model: echo
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: echo
context:
  autoCompaction: true
  compactAtPercent: 70
  targetPercent: 45
  preserveRecentMessages: 8
  modelAssisted: true
memory:
  indexEnabled: true
  indexPath: null
  retrievalLimit: 6
  semantic:
    kind: disabled
research:
  maxSources: 20
  maxWorkers: 4
search:
  profiles: {}
  roles: {}
mcp:
  oauthCredentialStore: auto
  servers: {}
skills:
  enabled: true
  allowUserOverrides: false
  bundled: bundled-skills
  repository: .colossus/skills
  user: skills
  disabled: []
packs:
  installRoot: .colossus/packs
sandbox:
  backend: native
  profile: workspace-development
  allowBrokerFallback: false
  acknowledgeExternalBoundary: false
  acknowledgeDangerFullAccess: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 30000
  maxOutputBytes: 1048576
  maxProcesses: 16
  maxMemoryBytes: 268435456
  maxConcurrency: 1
"#;

    let config = RuntimeConfig::from_yaml(yaml).expect("previous expanded schema-v2 config");
    assert_eq!(
        config.access.profile,
        colossus_access::AccessProfile::Development
    );
    assert_eq!(config.sandbox.backend, "native");
    assert_eq!(config.sandbox.profile, "workspace-development");
    assert!(!config.sandbox.acknowledge_danger_full_access);
    assert_eq!(config.sandbox.max_output_bytes, 1_048_576);
    assert_eq!(config.sandbox.max_memory_bytes, 268_435_456);
    assert_eq!(
        config.storage.location,
        super::StorageLocation::HomeWorkspace
    );
    assert_eq!(config.storage.path, Path::new("state.redb"));
}

#[test]
fn explicit_sandbox_resource_overrides_remain_unchanged() {
    for (max_output_bytes, max_memory_bytes) in [
        (64 * 1024_u64, 128 * 1024 * 1024_u64),
        (8 * 1024 * 1024_u64, 2 * 1024 * 1024 * 1024_u64),
    ] {
        let config = RuntimeConfig::from_yaml(&format!(
            "schemaVersion: 2\nstorage:\n  path: state.redb\nsandbox:\n  maxOutputBytes: {max_output_bytes}\n  maxMemoryBytes: {max_memory_bytes}\n"
        ))
        .expect("explicit sandbox resource overrides");
        assert_eq!(config.sandbox.max_output_bytes, max_output_bytes);
        assert_eq!(config.sandbox.max_memory_bytes, max_memory_bytes);
    }
}

#[test]
fn sandbox_defaults_are_contextual_and_reject_stale_acknowledgements() {
    let parse = |sandbox: &str| {
        RuntimeConfig::from_yaml(&format!(
            "schemaVersion: 2\nstorage:\n  path: state.redb\n{sandbox}"
        ))
    };

    for sandbox in [
        "",
        "sandbox: {}\n",
        "sandbox:\n  backend: danger_full_access\n",
    ] {
        let config = parse(sandbox).expect("dangerous default sandbox");
        assert_eq!(config.sandbox.backend, "danger_full_access");
        assert!(config.sandbox.acknowledge_danger_full_access);
        assert_eq!(config.sandbox.profile, "offline-default");
    }

    let safe = parse("sandbox:\n  backend: native\n").expect("explicit safe backend");
    assert!(!safe.sandbox.acknowledge_danger_full_access);
    assert!(
        parse("sandbox:\n  backend: native\n  acknowledgeDangerFullAccess: true\n").is_err(),
        "a stale danger acknowledgement must fail"
    );

    let unacknowledged =
        parse("sandbox:\n  backend: danger_full_access\n  acknowledgeDangerFullAccess: false\n")
            .expect("explicit unacknowledged direct backend");
    assert!(!unacknowledged.sandbox.acknowledge_danger_full_access);
}

#[test]
fn sparse_schema_still_requires_storage_and_explicit_union_tags() {
    assert!(RuntimeConfig::from_yaml("schemaVersion: 2\n").is_err());
    assert!(
        RuntimeConfig::from_yaml("schemaVersion: 2\nstorage:\n  path: state.redb\npolicy: {}\n")
            .is_err()
    );
    assert!(
        RuntimeConfig::from_yaml(
            "schemaVersion: 2\nstorage:\n  path: state.redb\nstorageExtra: true\n"
        )
        .is_err()
    );
}

#[test]
fn storage_keys_default_to_explicit_plaintext_none() {
    let config = RuntimeConfig::offline_template("state.redb");
    assert!(matches!(config.storage.keys, super::KeyConfig::None));
    assert!(config.to_yaml().expect("YAML").contains("kind: none"));

    let mut document: Value =
        serde_saphyr::from_str(&config.to_yaml().expect("YAML")).expect("YAML value");
    document["storage"]
        .as_object_mut()
        .expect("storage mapping")
        .remove("keys");
    let parsed = RuntimeConfig::from_yaml(&serde_saphyr::to_string(&document).expect("YAML"))
        .expect("configuration without keys");
    assert!(matches!(parsed.storage.keys, super::KeyConfig::None));
}

#[test]
fn default_runtime_limits_are_implicit_in_serialized_configuration() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    let yaml = config.to_yaml().expect("configuration YAML");
    let document: Value = serde_saphyr::from_str(&yaml).expect("configuration value");
    let root = document.as_object().expect("configuration mapping");

    assert!(!root.contains_key("agent"));
    assert!(!root.contains_key("subagents"));

    let parsed = RuntimeConfig::from_yaml(&yaml).expect("configuration with implicit limits");
    assert_eq!(parsed.agent, super::AgentConfig::default());
    assert_eq!(parsed.subagents, super::SubagentConfig::default());

    config.agent.max_turns = 7;
    config.subagents.max_concurrent = 3;
    let overridden_yaml = config.to_yaml().expect("configuration YAML");
    let overridden: Value = serde_saphyr::from_str(&overridden_yaml).expect("configuration value");
    let root = overridden.as_object().expect("configuration mapping");
    assert!(root.contains_key("agent"));
    assert!(root.contains_key("subagents"));
}

#[test]
fn resolved_configuration_yaml_states_default_runtime_limits() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    let yaml = config
        .to_resolved_yaml()
        .expect("resolved configuration YAML");
    let document: Value = serde_saphyr::from_str(&yaml).expect("configuration value");
    assert_eq!(
        document["agent"]["maxTurns"].as_u64(),
        Some(u64::from(super::DEFAULT_MAX_TURNS))
    );
    assert_eq!(document["subagents"]["maxConcurrent"].as_u64(), Some(10));
    RuntimeConfig::from_yaml(&yaml).expect("resolved configuration parses");

    config.agent.max_turns = 7;
    config.subagents.max_concurrent = 3;
    let overridden = config
        .to_resolved_yaml()
        .expect("resolved configuration YAML");
    assert_eq!(
        overridden,
        config.to_yaml().expect("configuration YAML"),
        "explicit overrides must not be restated"
    );
    let parsed = RuntimeConfig::from_yaml(&overridden).expect("overridden configuration parses");
    assert_eq!(parsed.agent.max_turns, 7);
    assert_eq!(parsed.subagents.max_concurrent, 3);
}

#[test]
fn storage_location_defaults_to_workspace_for_existing_configurations() {
    let config = RuntimeConfig::offline_template("state.redb");
    let mut document: Value =
        serde_saphyr::from_str(&config.to_yaml().expect("YAML")).expect("YAML value");
    document["storage"]
        .as_object_mut()
        .expect("storage mapping")
        .remove("location");
    let parsed = RuntimeConfig::from_yaml(&serde_saphyr::to_string(&document).expect("YAML"))
        .expect("legacy storage location");
    assert_eq!(parsed.storage.location, super::StorageLocation::Workspace);
}

#[test]
fn home_workspace_storage_is_confined_and_resolves_private_paths() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home_workspace_temporary = private_tempdir();
    let home_workspace = home_workspace_temporary
        .path()
        .canonicalize()
        .expect("canonical home workspace");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(&home_workspace, fs::Permissions::from_mode(0o700))
            .expect("private home workspace permissions");
    }
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.storage.location = super::StorageLocation::HomeWorkspace;
    config.use_environment_storage("secure-anchor.json");

    let resolved = config
        .resolve_storage_paths(workspace.path(), &home_workspace)
        .expect("resolved storage");
    assert_eq!(resolved.storage.path, home_workspace.join("state.redb"));
    assert_eq!(resolved.storage.location, super::StorageLocation::Workspace);
    let super::KeyConfig::Environment { anchor_path, .. } = resolved.storage.keys else {
        panic!("environment key config");
    };
    assert_eq!(anchor_path, home_workspace.join("secure-anchor.json"));

    let absolute_unsafe_path = std::env::current_dir()
        .expect("cwd")
        .join("state.redb")
        .display()
        .to_string();
    for unsafe_path in ["../state.redb", absolute_unsafe_path.as_str(), "."] {
        config.storage.path = unsafe_path.into();
        assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("unsafe YAML")).is_err());
        assert!(
            config
                .resolve_storage_paths(workspace.path(), &home_workspace)
                .is_err()
        );
    }
}

#[test]
fn distinct_workspaces_hold_distinct_home_writer_leases_and_surfaces() {
    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let home = ColossusHome::ensure_at(root.join("home")).expect("Colossus home");
    let workspace_one = root.join("workspace-one");
    let workspace_two = root.join("workspace-two");
    fs::create_dir(&workspace_one).expect("first workspace");
    fs::create_dir(&workspace_two).expect("second workspace");
    let identity_one = detect_workspace_identity(&workspace_one).expect("first identity");
    let identity_two = detect_workspace_identity(&workspace_two).expect("second identity");
    let cli_one = home
        .workspace_surface_dir(
            identity_one.canonical_path(),
            identity_one.as_ref(),
            HomeSurface::Cli,
        )
        .expect("first CLI surface");
    let cli_two = home
        .workspace_surface_dir(
            identity_two.canonical_path(),
            identity_two.as_ref(),
            HomeSurface::Cli,
        )
        .expect("second CLI surface");
    let desktop_one = home
        .workspace_surface_dir(
            identity_one.canonical_path(),
            identity_one.as_ref(),
            HomeSurface::Desktop,
        )
        .expect("first Desktop surface");
    assert_ne!(cli_one, cli_two);
    assert_ne!(cli_one, desktop_one);

    let mut source = RuntimeConfig::offline_template("state.redb");
    source.storage.location = super::StorageLocation::HomeWorkspace;
    let config_one = source
        .resolve_storage_paths(&workspace_one, &cli_one)
        .expect("first resolved configuration");
    let config_two = source
        .resolve_storage_paths(&workspace_two, &cli_two)
        .expect("second resolved configuration");
    let runtime_one = Runtime::open_with_options(
        &config_one,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(&workspace_one)
            .expect("first workspace options")
            .with_colossus_home(home.root())
            .expect("first home binding"),
    )
    .expect("first runtime");
    let runtime_two = Runtime::open_with_options(
        &config_two,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(&workspace_two)
            .expect("second workspace options")
            .with_colossus_home(home.root())
            .expect("second home binding"),
    )
    .expect("second runtime while first writer lease remains held");

    let doctor_one = runtime_one.state_doctor().expect("first state doctor");
    let doctor_two = runtime_two.state_doctor().expect("second state doctor");
    let lease_one = doctor_one["writer_lease"]["path"]
        .as_str()
        .expect("first writer lease path");
    let lease_two = doctor_two["writer_lease"]["path"]
        .as_str()
        .expect("second writer lease path");
    assert_eq!(Path::new(lease_one), cli_one.join("state.redb.writer.lock"));
    assert_eq!(Path::new(lease_two), cli_two.join("state.redb.writer.lock"));
    assert_ne!(lease_one, lease_two);
}

#[test]
fn unresolved_home_workspace_is_rejected_by_runtime_composition() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.storage.location = super::StorageLocation::HomeWorkspace;

    let result = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    );
    assert!(
        matches!(result, Err(RuntimeError::Config(message)) if message.contains("trusted host"))
    );
}

#[cfg(unix)]
#[test]
fn home_workspace_rejects_symlink_escape_and_desktop_state_alias() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let workspace = root.join("workspace");
    let cli = root.join("cli");
    let desktop = root.join("desktop");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&cli).expect("CLI state root");
    fs::create_dir(&desktop).expect("Desktop state root");
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).expect("CLI permissions");
    fs::set_permissions(&desktop, fs::Permissions::from_mode(0o700)).expect("Desktop permissions");

    let mut config = RuntimeConfig::offline_template("nested/state.redb");
    config.storage.location = super::StorageLocation::HomeWorkspace;
    symlink(&desktop, cli.join("nested")).expect("parent escape link");
    assert!(config.resolve_storage_paths(&workspace, &cli).is_err());

    fs::remove_file(cli.join("nested")).expect("remove parent link");
    config.storage.path = "state.redb".into();
    let desktop_state = desktop.join("state.redb");
    fs::write(&desktop_state, []).expect("Desktop state");
    fs::set_permissions(&desktop_state, fs::Permissions::from_mode(0o600))
        .expect("Desktop state permissions");
    symlink(&desktop_state, cli.join("state.redb")).expect("Desktop alias");
    assert!(config.resolve_storage_paths(&workspace, &cli).is_err());
}

#[cfg(unix)]
#[test]
fn runtime_revalidates_home_state_after_resolution_before_open() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let workspace = root.join("workspace");
    let cli = root.join("cli");
    let desktop = root.join("desktop");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&cli).expect("CLI state root");
    fs::create_dir(&desktop).expect("Desktop state root");
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).expect("CLI permissions");
    fs::set_permissions(&desktop, fs::Permissions::from_mode(0o700)).expect("Desktop permissions");
    let mut source = RuntimeConfig::offline_template("state.redb");
    source.storage.location = super::StorageLocation::HomeWorkspace;
    let resolved = source
        .resolve_storage_paths(&workspace, &cli)
        .expect("resolved configuration");

    let desktop_state = desktop.join("state.redb");
    fs::write(&desktop_state, []).expect("Desktop state");
    fs::set_permissions(&desktop_state, fs::Permissions::from_mode(0o600))
        .expect("Desktop state permissions");
    symlink(&desktop_state, cli.join("state.redb")).expect("state replacement link");
    let result = Runtime::open_with_options(
        &resolved,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(&workspace).expect("workspace options"),
    );
    assert!(matches!(result, Err(RuntimeError::Config(message)) if message.contains("unsafe")));
}

#[cfg(unix)]
#[test]
fn derived_home_runtime_paths_reject_cli_to_desktop_links() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let temporary = private_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let workspace = root.join("workspace");
    let cli = root.join("cli");
    let desktop = root.join("desktop");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&cli).expect("CLI root");
    fs::create_dir(&desktop).expect("Desktop root");
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).expect("CLI permissions");
    fs::set_permissions(&desktop, fs::Permissions::from_mode(0o700)).expect("Desktop permissions");

    symlink(&desktop, cli.join("state.memory-index")).expect("memory index alias");
    let mut source = RuntimeConfig::offline_template("state.redb");
    source.storage.location = super::StorageLocation::HomeWorkspace;
    assert!(source.resolve_storage_paths(&workspace, &cli).is_err());
    fs::remove_file(cli.join("state.memory-index")).expect("remove memory alias");

    let resolved = source
        .resolve_storage_paths(&workspace, &cli)
        .expect("resolved configuration");
    let desktop_auth = desktop.join("worker-auth");
    fs::write(&desktop_auth, []).expect("Desktop auth placeholder");
    fs::set_permissions(&desktop_auth, fs::Permissions::from_mode(0o600))
        .expect("Desktop auth permissions");
    symlink(&desktop_auth, cli.join("state.redb.worker-auth")).expect("worker auth alias");
    assert!(resolved.worker_ipc_auth_path_at(&workspace).is_err());
}

#[test]
fn observability_is_disabled_by_default_and_full_payloads_require_acknowledgement() {
    let config = RuntimeConfig::offline_template("state.redb");
    assert!(!config.observability.enabled);
    assert_eq!(
        config.observability.logs.journal_payloads,
        super::JournalPayloadMode::Disabled
    );

    let mut document: Value =
        serde_saphyr::from_str(&config.to_yaml().expect("YAML")).expect("YAML value");
    document["observability"] = serde_json::json!({
        "enabled": true,
        "logs": {
            "stdoutJson": true,
            "journalPayloads": "full",
            "acknowledgeSensitiveContent": false
        }
    });
    let yaml = serde_saphyr::to_string(&document).expect("YAML");
    assert!(RuntimeConfig::from_yaml(&yaml).is_err());
    document["observability"]["logs"]["acknowledgeSensitiveContent"] = Value::Bool(true);
    RuntimeConfig::from_yaml(&serde_saphyr::to_string(&document).expect("YAML"))
        .expect("acknowledged sensitive logging");
}

#[test]
fn security_posture_reports_plaintext_storage_and_effective_oauth_state() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    assert_eq!(
        config.security_posture().findings[0].code,
        "storage.plaintext"
    );
    config.mcp.servers.insert(
        "remote".into(),
        McpServerConfig {
            transport: McpTransportKind::StreamableHttp,
            command: PathBuf::new(),
            args: Vec::new(),
            working_directory: None,
            environment: BTreeMap::new(),
            url: Some("https://mcp.example.com/".into()),
            headers: BTreeMap::new(),
            credential_headers: BTreeMap::new(),
            allow_stateless: false,
            oauth: Some(McpOAuthConfig {
                client_id: "colossus".into(),
                client_secret_reference: None,
                callback_port: 8765,
                scopes: Vec::new(),
            }),
            allowed_tools: vec!["*".into()],
            research_tools: Vec::new(),
            timeout_ms: None,
            max_output_bytes: None,
            effect_action_prefix: None,
            provenance: None,
        },
    );
    let report = config.security_posture();
    assert_eq!(
        report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<Vec<_>>(),
        ["storage.plaintext", "credentials.mcp_oauth_plaintext"]
    );

    config.use_platform_storage();
    assert!(config.security_posture().is_hardened());
}

#[test]
fn security_posture_reports_process_local_ephemeral_storage() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.use_ephemeral_storage();
    let report = config.security_posture();
    assert_eq!(
        report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<Vec<_>>(),
        ["storage.ephemeral", "storage.plaintext"]
    );
    assert!(report.findings[0].summary.contains("only for this process"));
}

#[test]
fn security_posture_reports_danger_full_access_even_when_acknowledged() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.use_platform_storage();
    config.sandbox.backend = SandboxBoundaryMode::DangerFullAccess.as_backend().into();
    config.sandbox.acknowledge_danger_full_access = true;

    let report = config.security_posture();
    assert_eq!(report.finding_count(), 1);
    assert_eq!(report.findings[0].code, "sandbox.danger_full_access");
    assert!(
        report.findings[0]
            .summary
            .contains("ambient host authority")
    );
    assert!(report.findings[0].remediation.contains("isolation"));
    assert!(report.findings[0].remediation.contains("detached"));
}

#[test]
fn worker_authentication_path_is_adjacent_to_local_state() {
    let config = RuntimeConfig::offline_template(".colossus/state.redb");
    assert_eq!(
        config
            .worker_ipc_auth_path_at(PathBuf::from("/workspace").as_path())
            .expect("worker auth path"),
        PathBuf::from("/workspace/.colossus/state.redb.worker-auth")
    );
}

#[test]
fn direct_sandbox_backends_have_distinct_explicit_acknowledgements() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    for backend in ["external", "danger_full_access"] {
        config.sandbox.backend = backend.into();
        config.sandbox.acknowledge_external_boundary = false;
        config.sandbox.acknowledge_danger_full_access = false;
        assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());

        if backend == "external" {
            config.sandbox.acknowledge_external_boundary = true;
        } else {
            config.sandbox.acknowledge_danger_full_access = true;
        }
        assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());
    }

    config.sandbox.backend = "native".into();
    config.sandbox.acknowledge_external_boundary = true;
    config.sandbox.acknowledge_danger_full_access = false;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.sandbox.acknowledge_external_boundary = false;
    config.sandbox.acknowledge_danger_full_access = true;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
}

#[test]
fn sandbox_profile_defaults_but_direct_backends_cannot_use_workspace_development() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = "external".into();
    let mut document: Value =
        serde_saphyr::from_str(&config.to_yaml().expect("YAML")).expect("YAML value");
    document["sandbox"]
        .as_object_mut()
        .expect("sandbox object")
        .remove("profile");
    let parsed = RuntimeConfig::from_yaml(&serde_saphyr::to_string(&document).expect("YAML"))
        .expect("default sandbox profile");
    assert_eq!(parsed.sandbox.profile, "offline-default");

    config.sandbox.profile = "workspace-development".into();
    let workspace = tempdir().expect("workspace");
    assert!(derive_development_sandbox(&config, workspace.path()).is_err());
    config.sandbox.backend = "danger_full_access".into();
    assert!(derive_development_sandbox(&config, workspace.path()).is_err());
}

#[test]
fn runtime_wide_ca_bundle_path_round_trips_and_rejects_an_empty_path() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.network.ca_bundle_path = Some(PathBuf::from(".colossus/certs/company-ca.pem"));
    let yaml = config.to_yaml().expect("configuration YAML");
    let parsed = RuntimeConfig::from_yaml(&yaml).expect("configuration");
    assert_eq!(
        parsed.network.ca_bundle_path,
        Some(PathBuf::from(".colossus/certs/company-ca.pem"))
    );

    config.network.ca_bundle_path = Some(PathBuf::new());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("configuration YAML"))
            .expect_err("empty CA path")
            .to_string()
            .contains("network.caBundlePath")
    );
}

#[test]
fn model_reasoning_effort_round_trips_strictly() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    for effort in [
        ReasoningEffort::None,
        ReasoningEffort::Minimal,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::XHigh,
        ReasoningEffort::Max,
        ReasoningEffort::Ultra,
    ] {
        config
            .models
            .profiles
            .get_mut("echo")
            .expect("default model profile")
            .reasoning_effort = Some(effort);
        let yaml = config.to_yaml().expect("configuration YAML");
        let parsed = RuntimeConfig::from_yaml(&yaml).expect("configuration");
        assert_eq!(
            parsed.models.profiles["echo"].reasoning_effort,
            Some(effort)
        );
    }

    let yaml = config
        .to_yaml()
        .expect("configuration YAML")
        .replace("reasoningEffort: ultra", "reasoningEffort: extreme");
    assert!(RuntimeConfig::from_yaml(&yaml).is_err());
}

#[test]
fn schema_v1_is_rejected_with_model_profile_regeneration_guidance() {
    let legacy = r#"
schemaVersion: 1
providers:
  profiles:
    echo:
      kind: echo
      model: echo
  roles:
    primary: echo
"#;
    let error = RuntimeConfig::from_yaml(legacy).expect_err("schema v1 must fail");
    let message = error.to_string();
    assert!(message.contains("provider connections and model profiles are now separate"));
    assert!(message.contains("config init"));
}

#[test]
fn omitted_access_defaults_to_allow_all_and_removed_fields_are_rejected() {
    let active = RuntimeConfig::offline_template("state.redb");
    let mut document: Value = serde_saphyr::from_str(&active.to_yaml().expect("active YAML"))
        .expect("configuration value");
    {
        let root = document.as_object_mut().expect("configuration mapping");
        root.insert("agent".into(), json!({"tools": ["echo", "task.list"]}));
        let policy = root
            .get_mut("policy")
            .and_then(Value::as_object_mut)
            .expect("policy mapping");
        policy.insert("allow_actions".into(), json!(["task.list"]));
        policy.insert("approval_actions".into(), json!(["filesystem.write"]));
    }
    let mixed = serde_saphyr::to_string(&document).expect("mixed YAML");
    assert!(
        RuntimeConfig::from_yaml(&mixed)
            .expect_err("removed fields must fail")
            .to_string()
            .contains("are not supported")
    );
    {
        let root = document.as_object_mut().expect("configuration mapping");
        root.remove("access");
        root.remove("agent");
        let policy = root
            .get_mut("policy")
            .and_then(Value::as_object_mut)
            .expect("policy mapping");
        policy.remove("allow_actions");
        policy.remove("approval_actions");
    }
    let yaml = serde_saphyr::to_string(&document).expect("configuration YAML");
    let parsed = RuntimeConfig::from_yaml(&yaml).expect("missing access uses compiled defaults");
    assert_eq!(
        parsed.access.profile,
        colossus_access::AccessProfile::AllowAll
    );
}

#[test]
fn built_in_registry_classifies_every_tool_once() {
    let specs = colossus_tools::builtin_specs();
    let names = specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), specs.len(), "duplicate built-in tool");
    let tool_descriptors = specs
        .iter()
        .map(|spec| colossus_access::builtin_tool_descriptor(&spec.name))
        .collect::<Result<Vec<_>, _>>()
        .expect("complete built-in tool metadata");
    let resolution = colossus_access::resolve_access(
        &colossus_access::AccessConfig {
            profile: colossus_access::AccessProfile::AllowAll,
            ..colossus_access::AccessConfig::default()
        },
        &specs,
        colossus_access::builtin_action_descriptors(),
        tool_descriptors,
        &colossus_access::AccessContext {
            filesystem_read: true,
            filesystem_write: true,
            git_executable: true,
            any_executable: true,
            network_destination: true,
            model_network_tools: true,
            agent_search_route: true,
            interactive: true,
            mcp_configured: true,
        },
        false,
    )
    .expect("complete built-in registry");
    assert_eq!(resolution.active_tool_names().len(), specs.len());
    assert!(
        resolution
            .tools
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
    );
}

#[test]
fn runtime_host_can_keep_provider_network_without_offering_general_fetch_tools() {
    let ordinary_workspace = tempdir().expect("ordinary workspace");
    let offline_workspace = tempdir().expect("offline workspace");
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.providers.profiles.insert(
        "managed".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("https://provider.example/v1".into()),
            credential_reference: None,
            timeout_ms: Some(30_000),
            chat_completions_output_token_parameter: None,
        },
    );
    configure_primary_model(&mut config, "managed", "managed", "test-model");
    config
        .sandbox
        .network_destinations
        .push("https://provider.example".into());

    let ordinary = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(ordinary_workspace.path()).expect("ordinary options"),
    )
    .expect("ordinary isolated runtime");
    let ordinary_tools = ordinary
        .tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<BTreeSet<_>>();
    for expected in ["network.http", "web.fetch", "docs.fetch"] {
        assert!(ordinary_tools.contains(expected), "{expected}");
    }

    let provider_only = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(offline_workspace.path())
            .expect("offline options")
            .without_model_network_tools(),
    )
    .expect("provider-only network runtime");
    assert!(
        provider_only
            .provider_profiles()
            .iter()
            .any(|profile| profile.profile == "managed"),
        "configured provider remains available"
    );
    let provider_only_tools = provider_only
        .tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<BTreeSet<_>>();
    for hidden in ["network.http", "web.fetch", "docs.fetch"] {
        assert!(!provider_only_tools.contains(hidden), "{hidden}");
    }
}

#[test]
fn unacknowledged_danger_reports_declared_configured_resource_authority() {
    let workspace = tempdir().expect("workspace");
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = SandboxBoundaryMode::DangerFullAccess.as_backend().into();
    config.sandbox.acknowledge_danger_full_access = false;

    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("runtime options"),
    )
    .expect("unacknowledged runtime");

    let effective = runtime.effective_access();
    assert_eq!(effective["sandbox"]["resource_authority"], "declared");
    for resource in ["process", "filesystem", "network", "environment"] {
        assert_eq!(
            effective["sandbox"]["resource_matrix"][resource], "configured",
            "{resource}"
        );
    }
    assert_eq!(
        effective["sandbox"]["workspace_role"],
        "repository context, relative-path anchor, state identity, and configured resource boundary"
    );

    let doctor = runtime.sandbox_doctor();
    assert_eq!(doctor.resource_authority, ResourceAuthority::Declared);
    assert!(!doctor.direct_execution_globally_acknowledged);
    for resource in ["process", "filesystem", "network", "environment"] {
        assert_eq!(
            doctor.resource_matrix.get(resource).map(String::as_str),
            Some("configured"),
            "{resource}"
        );
    }
}

#[test]
fn storage_adapter_requires_exact_postgres_configuration_pairing() {
    use colossus_journal_postgres::{PostgresJournalConfig, PostgresTlsConfig};

    let mut config = RuntimeConfig::offline_template("state.redb");
    config.storage.adapter = StorageAdapter::Postgres;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.storage.postgres = Some(
        PostgresJournalConfig::new(
            "COLOSSUS_DATABASE_URL",
            "colossus_runtime",
            PostgresTlsConfig::WebpkiRoots,
        )
        .expect("PostgreSQL config"),
    );
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());
    config.storage.adapter = StorageAdapter::Redb;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.storage.adapter = StorageAdapter::Ephemeral;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.storage.postgres = None;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());
    config.use_platform_storage();
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
}

#[test]
fn ephemeral_runtime_uses_fresh_process_local_state_without_storage_files() {
    let workspace = private_tempdir();
    let state = workspace.path().join("absent/state.redb");
    let mut config = RuntimeConfig::offline_template(&state);
    config.use_ephemeral_storage();

    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    )
    .expect("ephemeral runtime");
    assert!(!state.exists());
    assert!(!state.with_extension("memory-index").exists());
    assert!(!PathBuf::from(format!("{}.writer.lock", state.display())).exists());

    runtime.create_session(Some("ephemeral")).expect("session");
    assert!(runtime.journal().head().expect("head").0 > 0);
    let doctor = runtime.state_doctor().expect("state doctor");
    assert_eq!(doctor["storage"]["adapter"], "ephemeral");
    assert!(doctor["storage"]["path"].is_null());
    assert_eq!(doctor["storage"]["persistence"], "process");
    assert_eq!(doctor["writer_lease"]["reason"], "process-local-ephemeral");
    drop(runtime);

    let reopened = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    )
    .expect("fresh ephemeral runtime");
    assert_eq!(reopened.journal().head().expect("fresh head").0, 0);
    assert!(!workspace.path().join("absent").exists());
}

#[tokio::test]
async fn background_projection_maintenance_advances_during_long_host_operations() {
    let workspace = private_tempdir();
    let mut config = RuntimeConfig::offline_template("unused.redb");
    config.use_ephemeral_storage();
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    )
    .expect("runtime");

    runtime
        .create_session(Some("maintenance"))
        .expect("create session");
    assert!(
        runtime
            .projection_status()
            .expect("lagged projections")
            .iter()
            .any(|projection| projection.lag > 0)
    );

    runtime
        .run_with_background_projection_maintenance(tokio::time::sleep(
            std::time::Duration::from_millis(1_100),
        ))
        .await;

    assert!(
        runtime
            .projection_status()
            .expect("maintained projections")
            .iter()
            .all(|projection| projection.ready)
    );
}

#[test]
fn mcp_agent_tool_results_preserve_reported_error_status() {
    let successful: colossus_mcp::McpCallOutput = serde_json::from_value(json!({
        "server": "fixture",
        "tool": "lookup",
        "result": {"content": [], "isError": false}
    }))
    .expect("successful MCP result");
    let failed: colossus_mcp::McpCallOutput = serde_json::from_value(json!({
        "server": "fixture",
        "tool": "lookup",
        "result": {
            "content": [{"type": "text", "text": "project not found"}],
            "isError": true
        }
    }))
    .expect("failed MCP result");

    let successful = crate::mcp_agent_tool_result(successful).expect("successful agent result");
    let failed = crate::mcp_agent_tool_result(failed).expect("failed agent result");
    assert_eq!(successful.exit_code, 0);
    assert_eq!(failed.exit_code, 1);
    assert!(failed.output.contains("project not found"));
}

#[test]
fn mcp_model_input_errors_are_recoverable_but_allowlist_denials_remain_terminal() {
    let unknown = crate::mcp_runtime_tool_error(RuntimeError::Mcp(
        colossus_mcp::McpError::UnknownServer("unknown".into()),
    ));
    let invalid = crate::mcp_runtime_tool_error(RuntimeError::Mcp(
        colossus_mcp::McpError::InvalidArguments("bad dynamic arguments".into()),
    ));
    let denied = crate::mcp_runtime_tool_error(RuntimeError::Mcp(
        colossus_mcp::McpError::ToolDenied("gitlab:delete_project".into()),
    ));

    assert!(matches!(
        unknown,
        colossus_ports::ToolError::InvalidArguments { .. }
    ));
    assert!(matches!(
        invalid,
        colossus_ports::ToolError::InvalidArguments { .. }
    ));
    assert!(matches!(denied, colossus_ports::ToolError::Denied(_)));
}

#[test]
fn unadvertised_mcp_tools_include_bounded_related_name_guidance() {
    let error = crate::unadvertised_mcp_tool_error(
        "gitlab",
        "merge_request_diffs",
        &[
            "get_merge_request_diffs".into(),
            "list_merge_request_diffs".into(),
        ],
    );
    let message = error.to_string();

    assert!(message.contains("get_merge_request_diffs"));
    assert!(message.contains("list_merge_request_diffs"));
    assert!(!message.contains("list_projects"));
}

#[test]
fn exact_mcp_tool_lookup_is_independent_of_catalog_output_budget() {
    let target = McpToolSummary {
        server: "fixture".into(),
        name: "target".into(),
        title: None,
        description: None,
        annotations: None,
        input_schema: json!({"type": "object"}),
        schema_sha256: "target-digest".into(),
    };
    let mut lookup = crate::McpToolLookup::new("fixture", "target");
    lookup.visit(target.clone());

    let mut presentation = Vec::new();
    let mut serialized_output_bytes = 2_usize;
    let mut presentation_rejected = false;
    for index in 0..16 {
        let tool = McpToolSummary {
            server: "fixture".into(),
            name: format!("later_tool_{index:02}"),
            title: None,
            description: None,
            annotations: None,
            input_schema: json!({
                "type": "object",
                "description": "x".repeat(128 * 1024),
            }),
            schema_sha256: format!("digest-{index:02}"),
        };
        lookup.visit(tool.clone());
        if !presentation_rejected
            && push_bounded_mcp_discovery_tool(
                &mut presentation,
                &mut serialized_output_bytes,
                tool,
            )
            .is_err()
        {
            presentation_rejected = true;
        }
    }

    assert!(
        presentation_rejected,
        "fixture must exceed mcp.tools budget"
    );
    assert_eq!(
        lookup.finish().expect("exact tool remains callable"),
        target
    );
}

#[test]
fn paginated_mcp_discovery_rejects_before_its_aggregate_output_limit() {
    let mut tools = Vec::new();
    let mut serialized_output_bytes = 2_usize;
    let mut rejected = None;

    for index in 0..MAX_MCP_TOOLS {
        let tool = McpToolSummary {
            server: "fixture".into(),
            name: format!("tool_{index:04}"),
            title: None,
            description: None,
            annotations: None,
            input_schema: json!({
                "type": "object",
                "description": "x".repeat(255 * 1024),
            }),
            schema_sha256: format!("digest-{index:04}"),
        };
        if let Err(error) =
            push_bounded_mcp_discovery_tool(&mut tools, &mut serialized_output_bytes, tool)
        {
            rejected = Some(error);
            break;
        }
    }

    let error = rejected.expect("aggregate discovery must reject oversized pagination");
    assert!(
        error
            .to_string()
            .contains("mcp.tools 1048576-byte output bound")
    );
    let output = serde_json::to_vec(&tools).expect("bounded discovery JSON");
    assert_eq!(serialized_output_bytes, output.len());
    assert!(output.len() <= MCP_TOOLS_MAX_OUTPUT_BYTES);
    assert!(tools.len() < MAX_MCP_TOOLS);
}

#[tokio::test]
async fn run_input_reads_honor_image_size_without_widening_ordinary_effects() {
    let workspace = private_tempdir();
    let candidate = workspace.path().join("candidate.bin");
    let bytes = vec![0x5a; 5 * 1_048_576];
    fs::write(&candidate, &bytes).expect("write run-input fixture");
    let mut config = RuntimeConfig::offline_template("unused.redb");
    config.use_ephemeral_storage();
    config.sandbox.filesystem.push(FilesystemGrant {
        root: workspace.path().display().to_string(),
        mode: "read".into(),
    });
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    )
    .expect("runtime");

    let ordinary = runtime
        .read_file_bytes(&candidate)
        .await
        .expect_err("ordinary reads retain the 4 MiB ceiling");
    assert!(
        ordinary.to_string().contains("permitted output bound"),
        "{ordinary}"
    );
    assert_eq!(
        runtime
            .read_run_input_file_bytes(&candidate)
            .await
            .expect("run-input read"),
        bytes
    );

    let before_render = runtime.journal().head().expect("journal head");
    let rendered = runtime
        .prompt_with_text_attachment_bytes(
            "Inspect",
            &[(PathBuf::from("note.txt"), b"one authorized read".to_vec())],
        )
        .expect("render already-read text");
    assert!(rendered.contains("one authorized read"));
    assert_eq!(
        runtime.journal().head().expect("journal head after render"),
        before_render,
        "rendering already-authorized bytes must not create a second effect"
    );
}

#[test]
fn canonical_session_branch_is_bounded_idempotent_and_detects_drift() {
    let workspace = private_tempdir();
    let mut config = RuntimeConfig::offline_template("unused.redb");
    config.use_ephemeral_storage();
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    )
    .expect("runtime");
    let actor = Actor {
        actor_type: ActorType::Application,
        id: "desktop-test".into(),
    };
    runtime
        .sessions
        .create_session("source-session", Some("Parent"), actor.clone())
        .expect("source session");
    runtime
        .sessions
        .append_messages(
            "source-session",
            "source-run",
            vec![
                SessionMessageAppend {
                    message: ModelMessage {
                        role: ModelMessageRole::User,
                        content: "first".into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    actor: actor.clone(),
                },
                SessionMessageAppend {
                    message: ModelMessage {
                        role: ModelMessageRole::Assistant,
                        content: "second".into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    actor: actor.clone(),
                },
            ],
        )
        .expect("source messages");

    let first = runtime
        .ensure_session_branch(
            "source-session",
            1,
            RunBranchContextMode::Exact,
            "branch-session",
            "branch-run",
            actor.clone(),
        )
        .expect("branch");
    assert_eq!(first.message_count, 1);
    let replay = runtime
        .ensure_session_branch(
            "source-session",
            1,
            RunBranchContextMode::Exact,
            "branch-session",
            "branch-run",
            actor.clone(),
        )
        .expect("idempotent branch replay");
    assert_eq!(replay.message_count, 1);
    assert_eq!(
        runtime
            .session_messages("branch-session")
            .expect("messages")
            .len(),
        1
    );

    assert!(
        runtime
            .ensure_session_branch(
                "source-session",
                2,
                RunBranchContextMode::Exact,
                "branch-session",
                "branch-run",
                actor,
            )
            .is_err(),
        "an existing branch cannot silently widen its snapshot"
    );
}

#[test]
fn conversation_session_branch_keeps_visible_messages_without_tool_traffic() {
    let workspace = private_tempdir();
    let mut config = RuntimeConfig::offline_template("unused.redb");
    config.use_ephemeral_storage();
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
    )
    .expect("runtime");
    let actor = Actor {
        actor_type: ActorType::Application,
        id: "desktop-test".into(),
    };
    runtime
        .sessions
        .create_session("conversation-source", Some("Parent"), actor.clone())
        .expect("source session");
    runtime
        .sessions
        .append_messages(
            "conversation-source",
            "source-run",
            vec![
                SessionMessageAppend {
                    message: ModelMessage {
                        role: ModelMessageRole::User,
                        content: "Inspect the workspace.".into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    actor: actor.clone(),
                },
                SessionMessageAppend {
                    message: ModelMessage {
                        role: ModelMessageRole::Assistant,
                        content: String::new().into(),
                        tool_call_id: None,
                        tool_calls: vec![ModelToolCall {
                            call_id: "call-1".into(),
                            name: "filesystem.list".into(),
                            arguments: json!({"path": "."}),
                        }],
                    },
                    actor: actor.clone(),
                },
                SessionMessageAppend {
                    message: ModelMessage {
                        role: ModelMessageRole::Tool,
                        content: "private tool payload".into(),
                        tool_call_id: Some("call-1".into()),
                        tool_calls: Vec::new(),
                    },
                    actor: actor.clone(),
                },
                SessionMessageAppend {
                    message: ModelMessage {
                        role: ModelMessageRole::Assistant,
                        content: "The workspace contains one project.".into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    actor: actor.clone(),
                },
            ],
        )
        .expect("source messages");

    let branch = runtime
        .ensure_session_branch(
            "conversation-source",
            4,
            RunBranchContextMode::Conversation,
            "conversation-branch",
            "branch-run",
            actor,
        )
        .expect("conversation branch");
    assert_eq!(branch.message_count, 2);
    let messages = runtime
        .session_messages("conversation-branch")
        .expect("branch messages")
        .into_iter()
        .map(|record| record.message)
        .collect::<Vec<_>>();
    assert_eq!(messages[0].role, ModelMessageRole::User);
    assert_eq!(messages[0].content, "Inspect the workspace.");
    assert_eq!(messages[1].role, ModelMessageRole::Assistant);
    assert_eq!(messages[1].content, "The workspace contains one project.");
    assert!(messages.iter().all(|message| message.tool_calls.is_empty()));
    assert!(
        messages
            .iter()
            .all(|message| message.tool_call_id.is_none())
    );
    assert!(
        messages
            .iter()
            .all(|message| !message.content.contains("private tool payload"))
    );
}

#[test]
fn startup_verification_defaults_to_incremental_and_accepts_explicit_full() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    assert_eq!(
        config.storage.startup_verification,
        StartupVerificationMode::Incremental
    );
    config.storage.startup_verification = StartupVerificationMode::Full;
    let yaml = config.to_yaml().expect("configuration YAML");
    assert!(yaml.contains("startupVerification: full"));
    assert_eq!(
        RuntimeConfig::from_yaml(&yaml)
            .expect("full verification configuration")
            .storage
            .startup_verification,
        StartupVerificationMode::Full
    );

    let mut document: Value = serde_saphyr::from_str(&yaml).expect("YAML value");
    document["storage"]
        .as_object_mut()
        .expect("storage object")
        .remove("startupVerification");
    let without_field = serde_saphyr::to_string(&document).expect("YAML");
    assert_eq!(
        RuntimeConfig::from_yaml(&without_field)
            .expect("default verification configuration")
            .storage
            .startup_verification,
        StartupVerificationMode::Incremental
    );

    document["storage"]["startupVerification"] = json!("sometimes");
    assert!(
        RuntimeConfig::from_yaml(&serde_saphyr::to_string(&document).expect("invalid YAML"))
            .is_err()
    );
}

#[test]
fn worm_audit_config_requires_https_origin_and_credential_grants() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.audit.exporter = AuditExporterConfig::WormHttp {
        endpoint: "https://worm.example/retained/".into(),
        credential_reference: Some("env:WORM_TOKEN".into()),
    };
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.sandbox.network_destinations = vec!["https://worm.example".into()];
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.sandbox.environment = vec!["WORM_TOKEN".into()];
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());
    config.audit.exporter = AuditExporterConfig::WormHttp {
        endpoint: "http://worm.example/retained/".into(),
        credential_reference: None,
    };
    config.sandbox.network_destinations = vec!["http://worm.example".into()];
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
}

#[test]
fn public_wildcard_config_allows_public_origins_but_not_loopback_routes() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.providers.profiles.insert(
        "public".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("https://api.example.com/v1".into()),
            credential_reference: None,
            timeout_ms: Some(30_000),
            chat_completions_output_token_parameter: None,
        },
    );
    configure_primary_model(&mut config, "public", "public", "test");
    config.sandbox.network_destinations = vec!["*".into()];
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());

    config.search = SearchConfig {
        profiles: BTreeMap::from([(
            "local".into(),
            SearchProfileConfig::Searxng {
                endpoint: "http://127.0.0.1:8888/search".into(),
                credential_reference: None,
                auth_header: "X-Searxng-Key".into(),
                user_agent: "colossus-test".into(),
                timeout_ms: 30_000,
            },
        )]),
        roles: BTreeMap::from([("agent".into(), "local".into())]),
    };
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:8888".into());
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());

    config.sandbox.network_destinations.push("*".into());
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
}

#[test]
fn selected_danger_backend_accepts_private_plaintext_profiles_before_acknowledgement() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = "danger_full_access".into();
    config.sandbox.acknowledge_danger_full_access = false;
    config.providers.profiles.insert(
        "private".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("http://10.0.0.8:8080/v1".into()),
            credential_reference: None,
            timeout_ms: Some(30_000),
            chat_completions_output_token_parameter: None,
        },
    );
    configure_primary_model(&mut config, "private", "private", "test");
    config.search = SearchConfig {
        profiles: BTreeMap::from([(
            "metadata".into(),
            SearchProfileConfig::Searxng {
                endpoint: "http://169.254.169.254/search".into(),
                credential_reference: None,
                auth_header: "X-Searxng-Key".into(),
                user_agent: "colossus-test".into(),
                timeout_ms: 30_000,
            },
        )]),
        roles: BTreeMap::from([("agent".into(), "metadata".into())]),
    };
    config.memory.semantic = SemanticMemoryConfig::Chroma {
        base_url: "http://10.0.0.9:8000".into(),
        tenant: "tenant".into(),
        database: "database".into(),
        collection: "collection".into(),
        credential_reference: None,
        timeout_ms: 5_000,
        position_path: None,
        embedding: Box::new(MemoryEmbeddingConfig::Local { dimensions: 128 }),
    };
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("ambient YAML")).is_ok(),
        "selected danger authority should validate profiles before session acknowledgement"
    );

    config.sandbox.backend = "native".into();
    config.sandbox.acknowledge_danger_full_access = false;
    config.sandbox.network_destinations = vec![
        "http://10.0.0.8:8080".into(),
        "http://169.254.169.254".into(),
        "http://10.0.0.9:8000".into(),
    ];
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("declared YAML")).is_err(),
        "declared authority must keep non-loopback HTTPS requirements"
    );
}

#[test]
fn shell_helpers_enforce_noninteractive_isolated_execution() {
    let shell = if cfg!(target_os = "windows") {
        std::path::Path::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
    } else {
        std::path::Path::new("/bin/sh")
    };
    let arguments = shell_command_arguments(shell, "pwd").expect("shell command arguments");
    if cfg!(target_os = "windows") {
        assert!(arguments.contains(&"-NoProfile".into()));
        assert!(arguments.contains(&"-NonInteractive".into()));
    } else {
        assert_eq!(arguments, ["-c", "pwd"]);
    }

    let call = ToolCall {
        call_id: "shell".into(),
        name: "shell.run".into(),
        arguments: json!({}),
    };
    assert!(
        reject_shell_startup_profiles(&call, &["--login".into()]).is_err(),
        "login shell was accepted"
    );
    assert!(
        reject_reserved_shell_environment(
            &call,
            &BTreeMap::from([("PATH".into(), "/untrusted".into())]),
        )
        .is_err()
    );
    let isolated = tempdir().expect("isolated directory");
    let mut environment = BTreeMap::new();
    configure_shell_environment(&mut environment, isolated.path(), "/bin:/usr/bin");
    #[cfg(not(target_os = "windows"))]
    {
        assert_eq!(environment["HOME"], isolated.path().display().to_string());
        assert_eq!(environment["TMPDIR"], isolated.path().display().to_string());
        assert_eq!(environment["PATH"], "/bin:/usr/bin");
    }
}

#[test]
fn model_workspace_path_normalizes_absolute_paths_inside_workspace() {
    let workspace = tempdir().expect("workspace");
    let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
    let requested = workspace.join("pcap/capture.pcap");

    assert_eq!(
        model_workspace_path(&workspace, &requested.to_string_lossy())
            .expect("inside absolute path"),
        requested
    );
}

#[test]
fn model_workspace_path_preserves_colossus_exclusion_for_absolute_paths() {
    let workspace = tempdir().expect("workspace");
    let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
    let requested = workspace.join(".colossus/state.redb");

    assert!(matches!(
        model_workspace_path(&workspace, &requested.to_string_lossy()),
        Err(colossus_ports::ToolError::Denied(message))
            if message.contains("outside .colossus")
    ));
}

#[test]
fn model_workspace_path_rejects_absolute_paths_outside_workspace() {
    let workspace = tempdir().expect("workspace");
    let outside = tempdir().expect("outside");
    let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
    let requested = fs::canonicalize(outside.path())
        .expect("canonical outside")
        .join("capture.pcap");

    assert!(matches!(
        model_workspace_path(&workspace, &requested.to_string_lossy()),
        Err(colossus_ports::ToolError::Denied(_))
    ));
}

#[test]
fn ambient_model_resource_paths_accept_absolute_control_state_and_parent_traversal() {
    let workspace = tempdir().expect("workspace");
    let outside = tempdir().expect("outside");
    let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
    let outside = fs::canonicalize(outside.path()).expect("canonical outside");

    assert_eq!(
        model_resource_path(
            &workspace,
            &outside.join(".colossus/data").to_string_lossy(),
            true
        )
        .expect("absolute ambient path"),
        outside.join(".colossus/data")
    );
    assert_eq!(
        model_resource_path(&workspace, "../outside/.git/config", true).expect("ambient traversal"),
        workspace.join("../outside/.git/config")
    );
    assert!(model_resource_path(&workspace, "../outside", false).is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn model_workspace_path_accepts_conventional_windows_absolute_spellings() {
    // Canonicalized workspaces use the extended-length spelling while models emit the
    // conventional drive-letter form.
    let workspace = std::path::Path::new(r"\\?\C:\repo");

    assert_eq!(
        model_workspace_path(workspace, r"C:\repo\src\lib.rs").expect("conventional spelling"),
        workspace.join(r"src\lib.rs")
    );
    assert_eq!(
        model_workspace_path(workspace, r"c:\REPO\src\lib.rs").expect("case-insensitive spelling"),
        workspace.join(r"src\lib.rs")
    );
    assert!(matches!(
        model_workspace_path(workspace, r"C:\other\src\lib.rs"),
        Err(colossus_ports::ToolError::Denied(_))
    ));
    assert!(matches!(
        model_workspace_path(workspace, r"C:\repo\.colossus\state.redb"),
        Err(colossus_ports::ToolError::Denied(_))
    ));
}

#[test]
fn extension_paths_resolve_against_the_embedded_runtime_workspace() {
    let workspace = tempdir().expect("workspace");
    let relative = std::path::Path::new("artifacts/demo.pack");
    assert_eq!(
        extension_path(workspace.path(), relative),
        workspace.path().join(relative).display().to_string()
    );

    let absolute = workspace.path().join("absolute.pack");
    assert_eq!(
        extension_path(workspace.path(), &absolute),
        absolute.display().to_string()
    );
}

#[tokio::test]
async fn pack_process_resolves_credentials_only_after_permit_and_redacts_output() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use colossus_policy::{
        BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, effect_request,
    };
    use colossus_ports::PolicyDecisionPoint;

    let secret = std::env::var("PATH").expect("PATH");
    let executable = fs::canonicalize(std::env::current_exe().expect("current executable"))
        .expect("canonical executable");
    let cwd = executable.parent().expect("executable parent").to_owned();
    let action = "pack.tool.demo.secret".to_owned();
    let declaration = PackProcessDeclaration {
        pack: "demo".into(),
        version: "1.0.0".into(),
        manifest_sha256: "a".repeat(64),
        tool: "demo.secret".into(),
        action: action.clone(),
        executable: executable.clone(),
        cwd: cwd.clone(),
        args: Vec::new(),
        environment: BTreeMap::from([("PACK_SECRET".into(), "env:PATH".into())]),
        permissions: vec!["process".into(), "credentials".into()],
    };
    let executor = PackProcessExecutor::new(
        BTreeMap::from([("demo.secret".into(), declaration.clone())]),
        Arc::new(SecretEchoProcess),
    );
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy: Arc<dyn PolicyDecisionPoint> = Arc::new(
        BuiltInPolicy::offline_default()
            .with_action(&action, DecisionOutcome::Allow)
            .with_post_effect(true)
            .with_sandbox("native", "pack-secret-test", false)
            .with_filesystem_root(executable.display().to_string(), "execute")
            .with_filesystem_root(cwd.display().to_string(), "read")
            .with_environment("PACK_SECRET"),
    );
    let gateway = EffectGateway::new(
        journal,
        policy,
        Arc::new(DenyApproval),
        SafetyKernel::new([action.clone()]),
        [42_u8; 32],
    );
    let input = PackToolEffectInput {
        pack: declaration.pack.clone(),
        version: declaration.version.clone(),
        manifest_sha256: declaration.manifest_sha256.clone(),
        tool: declaration.tool.clone(),
        executable: executable.clone(),
        cwd,
        args: Vec::new(),
        environment: declaration.environment.clone(),
        permissions: declaration.permissions.clone(),
    };
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::User,
            id: "pack-test".into(),
        },
        &action,
        executable.display().to_string(),
        serde_json::to_value(input).expect("input"),
    );
    request.capabilities = vec![action];
    request.credential_references = vec![CredentialReference {
        reference: "env:PATH".into(),
        value_hash: None,
    }];
    let released = gateway.execute(request, &executor).await.expect("execute");
    let value: serde_json::Value = serde_json::from_slice(&released.bytes).expect("result JSON");
    let stdout = BASE64
        .decode(value["stdout_base64"].as_str().expect("stdout"))
        .expect("stdout base64");
    assert_eq!(stdout, b"[REDACTED]");
    assert!(
        !released
            .bytes
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    );
}

#[test]
fn approved_plan_goal_objective_preserves_contract_and_mutation_labels() {
    let objective = goal_objective_from_plan(&PlanRecord {
        id: "plan-1".into(),
        session_id: "session-1".into(),
        prompt: "Ship Rust".into(),
        status: PlanStatus::Approved,
        revision: 2,
        content: "# Plan".into(),
        steps: vec![PlanStep {
            index: 1,
            title: "Implement".into(),
            detail: "Use the gateway".into(),
            requires_mutation: true,
        }],
        created_at: "created".into(),
        updated_at: "updated".into(),
        approved_at: Some("approved".into()),
        executed_run_id: None,
    });
    assert!(objective.contains("Execute approved plan plan-1."));
    assert!(objective.contains("Original request:\nShip Rust"));
    assert!(objective.contains("Approved plan:\n# Plan"));
    assert!(objective.contains("1. Implement [mutation] — Use the gateway"));
}

#[test]
fn agent_config_rejects_unbounded_turns_and_invalid_runtime_limits() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.agent.max_turns = 101;
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "unbounded model turn count was accepted"
    );

    config.agent.max_turns = super::DEFAULT_MAX_TURNS;
    config.memory.retrieval_limit = 0;
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "zero memory retrieval limit was accepted"
    );
    config.memory.retrieval_limit = 6;
    config.subagents.max_concurrent = 0;
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "zero subagent concurrency was accepted"
    );
}

#[test]
fn provider_neutral_search_requires_explicit_valid_routes() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.search = SearchConfig {
        profiles: std::collections::BTreeMap::from([(
            "local".into(),
            SearchProfileConfig::Searxng {
                endpoint: "http://127.0.0.1:8888/search".into(),
                credential_reference: None,
                auth_header: "X-Searxng-Key".into(),
                user_agent: "colossus-test".into(),
                timeout_ms: 30_000,
            },
        )]),
        roles: std::collections::BTreeMap::from([
            ("agent".into(), "local".into()),
            ("research".into(), "local".into()),
        ]),
    };
    config
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:8888".into());
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());

    config.search.roles.remove("agent");
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());
}

#[test]
fn semantic_memory_requires_enabled_index_secure_origins_and_valid_profiles() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.memory.semantic = SemanticMemoryConfig::Chroma {
        base_url: "http://127.0.0.1:8000".into(),
        tenant: "default_tenant".into(),
        database: "default_database".into(),
        collection: "colossus-memory".into(),
        credential_reference: None,
        timeout_ms: 5_000,
        position_path: None,
        embedding: Box::new(MemoryEmbeddingConfig::Local { dimensions: 256 }),
    };
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:8000".into());
    let yaml = config.to_yaml().expect("YAML");
    assert!(yaml.contains("baseUrl:"));
    assert!(yaml.contains("timeoutMs:"));
    assert!(RuntimeConfig::from_yaml(&yaml).is_ok());

    config.memory.index_enabled = false;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config.memory.index_enabled = true;
    config.memory.semantic = SemanticMemoryConfig::Chroma {
        base_url: "http://127.0.0.1:8000".into(),
        tenant: "default_tenant".into(),
        database: "default_database".into(),
        collection: "colossus-memory".into(),
        credential_reference: None,
        timeout_ms: 5_000,
        position_path: None,
        embedding: Box::new(MemoryEmbeddingConfig::OpenAiCompatible {
            profile: "local-embedding".into(),
            model: "embedding-model".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            credential_reference: None,
            timeout_ms: 5_000,
            dimensions: Some(768),
        }),
    };
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:11434".into());
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());
}

#[test]
fn mcp_config_requires_exact_process_identity_refs_and_allowlists() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    let command = std::env::current_exe()
        .expect("test executable")
        .canonicalize()
        .expect("canonical test executable");
    config.sandbox.executables.push(command.clone());
    config.sandbox.filesystem.push(FilesystemGrant {
        root: std::env::current_dir().expect("cwd").display().to_string(),
        mode: "read".into(),
    });
    config.sandbox.environment.push("CHILD_TOKEN".into());
    config.mcp.servers.insert(
        "fixture".into(),
        McpServerConfig {
            transport: colossus_mcp::McpTransportKind::Stdio,
            command,
            args: Vec::new(),
            working_directory: None,
            environment: BTreeMap::from([("CHILD_TOKEN".into(), "env:HOST_TOKEN".into())]),
            url: None,
            headers: BTreeMap::new(),
            credential_headers: BTreeMap::new(),
            allow_stateless: false,
            oauth: None,
            allowed_tools: vec!["search".into()],
            research_tools: vec![McpResearchToolConfig {
                tool: "search".into(),
                title: None,
                arguments: json!({"query": "{query}"}),
            }],
            timeout_ms: Some(5_000),
            max_output_bytes: Some(64 * 1024),
            effect_action_prefix: None,
            provenance: None,
        },
    );
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok());

    config
        .mcp
        .servers
        .get_mut("fixture")
        .expect("fixture")
        .allow_stateless = true;
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config
        .mcp
        .servers
        .get_mut("fixture")
        .expect("fixture")
        .allow_stateless = false;

    config
        .mcp
        .servers
        .get_mut("fixture")
        .expect("fixture")
        .environment
        .insert("CHILD_TOKEN".into(), "raw-secret-is-never-valid".into());
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config
        .mcp
        .servers
        .get_mut("fixture")
        .expect("fixture")
        .environment
        .insert("CHILD_TOKEN".into(), "env:HOST_TOKEN".into());
    config
        .mcp
        .servers
        .get_mut("fixture")
        .expect("fixture")
        .allowed_tools = vec!["search".into(), "search".into()];
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
    config
        .mcp
        .servers
        .get_mut("fixture")
        .expect("fixture")
        .allowed_tools = Vec::new();
    assert!(RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err());
}

#[test]
fn remote_mcp_stateless_opt_in_round_trips_and_defaults_off() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.environment.push("SPLUNK_MCP_TOKEN".into());
    config
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:18000".into());
    config.mcp.servers.insert(
        "splunk".into(),
        McpServerConfig {
            transport: McpTransportKind::StreamableHttp,
            command: PathBuf::new(),
            args: Vec::new(),
            working_directory: None,
            environment: BTreeMap::new(),
            url: Some("http://127.0.0.1:18000/services/mcp".into()),
            headers: BTreeMap::new(),
            credential_headers: BTreeMap::from([(
                "Authorization".into(),
                McpCredentialHeaderConfig {
                    scheme: Some("Bearer".into()),
                    reference: "env:SPLUNK_MCP_TOKEN".into(),
                },
            )]),
            allow_stateless: true,
            oauth: None,
            allowed_tools: vec!["*".into()],
            research_tools: Vec::new(),
            timeout_ms: Some(5_000),
            max_output_bytes: Some(64 * 1024),
            effect_action_prefix: None,
            provenance: None,
        },
    );

    let yaml = config.to_yaml().expect("YAML");
    assert!(yaml.contains("allowStateless: true"));
    let parsed = RuntimeConfig::from_yaml(&yaml).expect("stateless remote config");
    assert!(parsed.mcp.servers["splunk"].allow_stateless);

    let default_yaml = yaml.replacen("      allowStateless: true\n", "", 1);
    let defaulted = RuntimeConfig::from_yaml(&default_yaml).expect("default stateful config");
    assert!(!defaulted.mcp.servers["splunk"].allow_stateless);
}

#[test]
fn provider_config_requires_secure_origin_grants_and_known_roles() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.providers.profiles.insert(
        "local".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("http://127.0.0.1:12434/v1".into()),
            credential_reference: None,
            timeout_ms: Some(5_000),
            chat_completions_output_token_parameter: None,
        },
    );
    configure_primary_model(&mut config, "local", "local", "local-model");
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "provider origin without a sandbox grant was accepted"
    );

    config
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:12434".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
        "loopback provider with an exact origin grant was rejected"
    );

    config
        .models
        .roles
        .insert("surprise".into(), "local".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "unknown provider role was accepted"
    );
}

#[test]
fn provider_timeout_defaults_are_host_aware_and_explicit_values_win() {
    let remote = ProviderProfileConfig {
        kind: ProviderKind::OpenAiCompatible,
        base_url: Some("https://models.example.test/v1".into()),
        credential_reference: None,
        timeout_ms: None,
        chat_completions_output_token_parameter: None,
    };
    let loopback = ProviderProfileConfig {
        base_url: Some("http://[::1]:11434/v1".into()),
        ..remote.clone()
    };
    let explicit = ProviderProfileConfig {
        timeout_ms: Some(42_000),
        ..loopback.clone()
    };

    assert_eq!(remote.effective_timeout_ms(), REMOTE_PROVIDER_TIMEOUT_MS);
    assert_eq!(
        loopback.effective_timeout_ms(),
        LOOPBACK_PROVIDER_TIMEOUT_MS
    );
    assert_eq!(explicit.effective_timeout_ms(), 42_000);
    assert!(
        !serde_json::to_string(&remote)
            .expect("provider YAML")
            .contains("timeoutMs"),
        "automatic timeout should remain omitted from canonical YAML"
    );
}

#[test]
fn chat_completions_output_token_parameter_is_explicit_and_kind_scoped() {
    let legacy = ProviderProfileConfig {
        kind: ProviderKind::OpenAiCompatible,
        base_url: Some("https://models.example.test/v1".into()),
        credential_reference: None,
        timeout_ms: None,
        chat_completions_output_token_parameter: None,
    };
    assert_eq!(
        provider_profile("legacy", &legacy)
            .expect("legacy-compatible provider profile")
            .chat_completions_output_token_parameter,
        ChatCompletionsOutputTokenParameter::MaxTokens
    );
    assert!(
        !serde_json::to_string(&legacy)
            .expect("provider configuration")
            .contains("chatCompletionsOutputTokenParameter"),
        "default wire mode should remain omitted from canonical configuration"
    );

    let modern = ProviderProfileConfig {
        chat_completions_output_token_parameter: Some(
            ChatCompletionsOutputTokenParameter::MaxCompletionTokens,
        ),
        ..legacy.clone()
    };
    assert_eq!(
        provider_profile("modern", &modern)
            .expect("modern provider profile")
            .chat_completions_output_token_parameter,
        ChatCompletionsOutputTokenParameter::MaxCompletionTokens
    );
    assert!(
        serde_json::to_string(&modern)
            .expect("provider configuration")
            .contains("\"chatCompletionsOutputTokenParameter\":\"max_completion_tokens\"")
    );

    let responses = ProviderProfileConfig {
        kind: ProviderKind::OpenAiResponses,
        chat_completions_output_token_parameter: Some(ChatCompletionsOutputTokenParameter::Omit),
        ..legacy
    };
    assert!(
        provider_profile("responses", &responses).is_err(),
        "Responses profile accepted a Chat Completions-only field"
    );
}

#[test]
fn remote_provider_http_fails_closed_and_responses_credentials_are_optional() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.providers.profiles.insert(
        "remote".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCompatible,
            base_url: Some("http://example.com/v1".into()),
            credential_reference: None,
            timeout_ms: Some(5_000),
            chat_completions_output_token_parameter: None,
        },
    );
    configure_primary_model(&mut config, "remote", "remote", "remote-model");
    config
        .sandbox
        .network_destinations
        .push("http://example.com".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "remote plaintext provider URL was accepted"
    );

    config.providers.profiles.insert(
        "remote".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiResponses,
            base_url: Some("https://api.openai.com/v1".into()),
            credential_reference: None,
            timeout_ms: Some(5_000),
            chat_completions_output_token_parameter: None,
        },
    );
    config.sandbox.network_destinations = vec!["https://api.openai.com".into()];
    configure_primary_model(&mut config, "remote", "remote", "gpt-test");
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
        "OpenAI Responses profile without a credential reference was rejected"
    );

    config.providers.profiles.insert(
        "remote".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiCodex,
            base_url: None,
            credential_reference: Some("codex:default".into()),
            timeout_ms: Some(5_000),
            chat_completions_output_token_parameter: None,
        },
    );
    config.sandbox.network_destinations = vec![
        "https://chatgpt.com".into(),
        "https://auth.openai.com".into(),
    ];
    configure_primary_model(&mut config, "remote", "remote", "gpt-test");
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
        "Codex/ChatGPT provider profile was rejected"
    );
    config.sandbox.network_destinations = vec!["https://chatgpt.com".into()];
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "Codex profile without its token refresh origin was accepted"
    );

    config.providers.profiles.insert(
        "remote".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiResponses,
            base_url: Some("https://api.openai.com/v1".into()),
            credential_reference: None,
            timeout_ms: Some(5_000),
            chat_completions_output_token_parameter: None,
        },
    );
    config.sandbox.network_destinations = vec!["https://api.openai.com".into()];

    config
        .providers
        .profiles
        .get_mut("remote")
        .expect("remote profile")
        .credential_reference = Some("host:provider-main".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
        "managed-runtime host credential reference was rejected"
    );

    config
        .providers
        .profiles
        .get_mut("remote")
        .expect("remote profile")
        .credential_reference = Some("host:provider/main".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "unsafe managed-runtime host credential reference was accepted"
    );
}

#[test]
fn oci_config_requires_cleanup_budget_digest_and_safe_environment_names() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = "oci".into();
    config.sandbox.timeout_ms = 4_999;
    config.sandbox.oci_image = Some(format!("python@sha256:{}", "a".repeat(64)));
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "short OCI cleanup budget was accepted"
    );

    config.sandbox.timeout_ms = 5_000;
    config.sandbox.oci_image = Some("python:latest".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "mutable OCI image was accepted"
    );

    config.sandbox.oci_image = Some(format!("python@sha256:{}", "a".repeat(64)));
    config.sandbox.network_destinations = vec!["https://example.com".into()];
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "networked OCI sandbox without a proxy image was accepted"
    );

    config.sandbox.oci_proxy_image = Some("colossus-proxy:latest".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "mutable OCI proxy image was accepted"
    );

    config.sandbox.oci_proxy_image = Some(format!("sha256:{}", "b".repeat(64)));
    config.sandbox.timeout_ms = 9_999;
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "networked OCI cleanup budget was accepted"
    );

    config.sandbox.timeout_ms = 10_000;
    #[cfg(not(windows))]
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
        "valid networked OCI proxy configuration was rejected"
    );
    #[cfg(windows)]
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "Windows accepted OCI before live path-mapping acceptance"
    );

    config.sandbox.network_destinations.clear();
    config.sandbox.oci_proxy_image = None;
    config.sandbox.environment = vec!["BAD-NAME".into()];
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "unsafe environment name was accepted"
    );

    config.sandbox.environment.clear();
    config.sandbox.oci_runtime = Some("/usr/bin/container-runtime".into());
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "unknown OCI runtime was accepted"
    );
}

#[test]
fn windows_job_config_reserves_confirmed_cleanup_time() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = "windows_job".into();
    config.sandbox.timeout_ms = MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS - 1;
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_err(),
        "short Windows cleanup budget was accepted"
    );

    config.sandbox.timeout_ms = MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS;
    assert!(
        RuntimeConfig::from_yaml(&config.to_yaml().expect("YAML")).is_ok(),
        "valid Windows cleanup budget was rejected"
    );
}

#[test]
fn startup_marks_started_effects_unknown_without_retrying() {
    let journal = Arc::new(InMemoryEventJournal::default());
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "effect:request-1".into(),
            expected_stream_version: 0,
            classification: EventClassification::Effect,
            event_type: "effect.started.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext {
                correlation_id: "correlation".into(),
                ..ExecutionContext::default()
            },
            payload: json!({}),
        })
        .expect("started event");
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "effect:request-1".into(),
            expected_stream_version: 1,
            classification: EventClassification::Effect,
            event_type: "effect.release_denied.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext {
                correlation_id: "correlation".into(),
                ..ExecutionContext::default()
            },
            payload: json!({}),
        })
        .expect("nonterminal event after start");
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "effect:denied-before-start".into(),
            expected_stream_version: 0,
            classification: EventClassification::Effect,
            event_type: "effect.denied.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({}),
        })
        .expect("denied event");
    assert_eq!(
        recover_unknown_effects(journal.as_ref()).expect("recover"),
        1
    );
    assert_eq!(
        recover_unknown_effects(journal.as_ref()).expect("idempotent"),
        0
    );
    let events = journal
        .read_stream("effect:request-1")
        .expect("effect stream");
    assert_eq!(
        events.last().expect("terminal event").event_type,
        "effect.outcome_unknown.v1"
    );
    assert_eq!(
        journal
            .read_stream("effect:denied-before-start")
            .expect("denied effect stream")
            .len(),
        1
    );
}

#[test]
fn startup_effect_recovery_finds_a_started_effect_missing_from_the_current_projection() {
    let journal = InMemoryEventJournal::default();
    let projections = InMemoryProjectionStore::default();
    let started = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "effect:request-1".into(),
            expected_stream_version: 0,
            classification: EventClassification::Effect,
            event_type: "effect.started.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext {
                correlation_id: "correlation".into(),
                ..ExecutionContext::default()
            },
            payload: json!({}),
        })
        .expect("started event");
    projections
        .apply(ProjectionBatch {
            projection: EFFECT_RECOVERY_PROJECTION.into(),
            expected_position: 0,
            through_sequence: started.global_sequence,
            mutations: vec![ProjectionMutation::Delete {
                key: started.stream_id.clone(),
            }],
        })
        .expect("projection cursor without a record");
    assert_eq!(
        projections
            .position(EFFECT_RECOVERY_PROJECTION)
            .expect("projection position"),
        journal.head().expect("journal head").0
    );
    assert!(
        projections
            .get(EFFECT_RECOVERY_PROJECTION, &started.stream_id)
            .expect("projection lookup")
            .is_none()
    );

    assert_eq!(
        recover_unknown_effects(&journal).expect("canonical recovery"),
        1
    );
    assert_eq!(
        journal
            .read_stream("effect:request-1")
            .expect("effect stream")
            .len(),
        2
    );
}

#[test]
fn startup_effect_recovery_rejects_more_than_the_safe_pending_bound() {
    let journal = InMemoryEventJournal::default();
    for index in 0..1_025 {
        journal
            .append(NewEvent {
                event_version: 1,
                stream_id: format!("effect:{index:04}"),
                expected_stream_version: 0,
                classification: EventClassification::Effect,
                event_type: "effect.started.v1".into(),
                actor: Actor {
                    actor_type: ActorType::System,
                    id: "test".into(),
                },
                context: ExecutionContext::default(),
                payload: json!({}),
            })
            .expect("started effect");
    }

    let error =
        recover_unknown_effects(&journal).expect_err("oversized startup recovery must fail closed");
    assert!(
        error
            .to_string()
            .contains("startup effect recovery exceeds the safe bound of 1024")
    );
    for stream_id in ["effect:0000", "effect:1024"] {
        assert_eq!(
            journal.read_stream(stream_id).expect("effect stream").len(),
            1,
            "recovery wrote a partial result before enforcing its bound"
        );
    }
}

#[tokio::test]
async fn model_skill_resource_tool_is_active_scoped_and_post_gated() {
    let directory = tempdir().expect("tempdir");
    let skill = directory.path().join("skills/demo");
    fs::create_dir_all(skill.join("references")).expect("skill directory");
    fs::write(skill.join("SKILL.md"), "Use the resource.").expect("instructions");
    fs::write(
            skill.join("manifest.json"),
            r#"{"name":"demo","version":"1.0.0","description":"Demo","triggers":[],"required_tools":[],"permissions":[],"offline_compatible":true}"#,
        )
        .expect("manifest");
    fs::write(skill.join("references/guide.md"), "bounded resource").expect("resource");
    let repository: Arc<dyn SkillRepository> = Arc::new(
        FilesystemSkillRepository::new(
            vec![SkillRoot {
                path: directory.path().join("skills"),
                label: "test".into(),
            }],
            false,
            Vec::new(),
        )
        .expect("repository"),
    );
    let skill_executor = Arc::new(SkillEffectExecutor {
        resources: Arc::new(SkillResourceService::new(repository)),
        authoring: Arc::new(
            SkillAuthoringService::new(
                directory.path().join("user-skills"),
                directory.path().canonicalize().expect("workspace"),
            )
            .expect("authoring"),
        ),
    });
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("skill.resource.read", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["skill.resource.read".into()]),
        [25_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: Some(skill_executor),
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: directory.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let call = ToolCall {
        call_id: "skill-call".into(),
        name: "skill.resource.read".into(),
        arguments: json!({"name": "demo", "path": "references/guide.md"}),
    };
    let context = ExecutionContext {
        correlation_id: "run-1".into(),
        session_id: Some("session-1".into()),
        run_id: Some("run-1".into()),
        skill_ids: vec!["demo".into()],
        ..ExecutionContext::default()
    };
    let result = executor
        .execute(call.clone(), context)
        .await
        .expect("active resource");
    assert!(result.output.contains("bounded resource"));
    let denied = executor
        .execute(
            call,
            ExecutionContext {
                correlation_id: "run-2".into(),
                session_id: Some("session-1".into()),
                run_id: Some("run-2".into()),
                ..ExecutionContext::default()
            },
        )
        .await;
    assert!(denied.is_err());
    let event_types = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"effect.release_requested.v1".into()));
}

#[tokio::test]
async fn skill_authoring_mutation_cannot_execute_without_approval_permit() {
    let directory = tempdir().expect("tempdir");
    let workspace = directory.path().canonicalize().expect("workspace");
    let repository: Arc<dyn SkillRepository> = Arc::new(
        FilesystemSkillRepository::new(Vec::new(), false, Vec::new()).expect("repository"),
    );
    let executor = SkillEffectExecutor {
        resources: Arc::new(SkillResourceService::new(repository)),
        authoring: Arc::new(
            SkillAuthoringService::new(directory.path().join("user"), workspace)
                .expect("authoring"),
        ),
    };
    let operation = SkillOperation::Scaffold {
        name: "permit-demo".into(),
        description: "Permit-bound skill".into(),
        instructions: "Data-only instructions.".into(),
        resource_dirs: Vec::new(),
    };
    let request = colossus_policy::effect_request(
        colossus_policy::system_actor("skill-test"),
        operation.action(),
        operation.resource(),
        serde_json::to_value(&operation).expect("operation"),
    );
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = Arc::new(
        colossus_policy::BuiltInPolicy::offline_default()
            .with_action("skill.scaffold", DecisionOutcome::RequireApproval),
    );
    let denied = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        policy.clone(),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["skill.scaffold".into()]),
        [26_u8; 32],
    )
    .execute(request.clone(), &executor)
    .await;
    assert!(denied.is_err());
    assert!(!directory.path().join("user/permit-demo").exists());

    let released = colossus_policy::EffectGateway::new(
        journal,
        policy,
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new(["skill.scaffold".into()]),
        [27_u8; 32],
    )
    .execute(request, &executor)
    .await
    .expect("approved scaffold");
    let result: SkillScaffoldResult = serde_json::from_slice(&released.bytes).expect("result");
    assert_eq!(result.name, "permit-demo");
    assert!(directory.path().join("user/permit-demo/SKILL.md").is_file());
}

#[test]
fn startup_marks_running_subagents_interrupted_without_retrying() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    sessions
        .create_session(
            "session-1",
            Some("parent"),
            Actor {
                actor_type: ActorType::User,
                id: "test".into(),
            },
        )
        .expect("session");
    let repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let service = colossus_work::WorkService::new(Arc::clone(&repository), sessions);
    let actor = Actor {
        actor_type: ActorType::User,
        id: "test".into(),
    };
    let job = service
        .create_subagent(
            colossus_work::CreateSubagentRequest {
                session_id: "session-1".into(),
                parent_run_id: "run-1".into(),
                parent_call_id: "call-1".into(),
                task: "unfinished".into(),
                role: "subagent_default".into(),
                allowed_tools: None,
                instruction_snapshot_id: None,
            },
            actor.clone(),
        )
        .expect("queue");
    service.start_subagent(&job.id, actor).expect("start");
    assert_eq!(
        recover_interrupted_subagents(repository.as_ref(), &service).expect("recover"),
        1
    );
    assert_eq!(
        repository
            .get_subagent(&job.id)
            .expect("job")
            .expect("record")
            .status,
        SubagentStatus::Interrupted
    );
    assert_eq!(
        recover_interrupted_subagents(repository.as_ref(), &service).expect("idempotent"),
        0
    );
}

#[tokio::test]
async fn filesystem_adapter_cannot_escape_permitted_root_and_uses_post_release() {
    let allowed = tempdir().expect("allowed root");
    let denied = tempdir().expect("denied root");
    let allowed_file = allowed.path().join("workflow.yaml");
    let denied_file = denied.path().join("secret.txt");
    fs::write(&allowed_file, "safe").expect("allowed file");
    fs::write(&denied_file, "secret").expect("denied file");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("filesystem.read", DecisionOutcome::Allow)
        .with_filesystem_read_root(allowed.path().display().to_string());
    let gateway = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["filesystem.read".into()]),
        [4_u8; 32],
    );

    let mut allowed_request = colossus_policy::effect_request(
        colossus_policy::system_actor("test"),
        "filesystem.read",
        allowed_file.display().to_string(),
        json!({"path": allowed_file}),
    );
    allowed_request.capabilities = vec!["filesystem.read".into()];
    let released = gateway
        .execute(
            allowed_request,
            &colossus_sandbox::FilesystemExecutor::new(),
        )
        .await
        .expect("allowed read");
    assert_eq!(released.bytes, b"safe");
    assert!(
        journal
            .read_global(1, 20)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "effect.release_requested.v1")
    );

    let mut denied_request = colossus_policy::effect_request(
        colossus_policy::system_actor("test"),
        "filesystem.read",
        denied_file.display().to_string(),
        json!({"path": denied_file}),
    );
    denied_request.capabilities = vec!["filesystem.read".into()];
    let error = gateway
        .execute(denied_request, &colossus_sandbox::FilesystemExecutor::new())
        .await
        .expect_err("path escape denied");
    assert!(matches!(error, colossus_policy::GatewayError::Safety(_)));
}

#[tokio::test]
async fn agent_filesystem_tool_executes_only_through_the_gateway() {
    let allowed = tempdir().expect("allowed root");
    let file = allowed.path().join("note.txt");
    fs::write(&file, "tool content").expect("fixture");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("filesystem.read", DecisionOutcome::Allow)
        .with_filesystem_read_root(allowed.path().display().to_string());
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["filesystem.read".into()]),
        [5_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: allowed.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let result = executor
        .execute(
            ToolCall {
                call_id: "call-1".into(),
                name: "filesystem.read".into(),
                arguments: json!({"path": "note.txt"}),
            },
            ExecutionContext {
                correlation_id: "run-1".into(),
                run_id: Some("run-1".into()),
                ..ExecutionContext::default()
            },
        )
        .await
        .expect("tool result");
    assert_eq!(result.output, "tool content");
    let events = journal.read_global(1, 20).expect("effect events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.started.v1")
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.completed.v1")
    );
}

#[tokio::test]
async fn agent_list_and_search_tools_return_only_workspace_relative_results() {
    let allowed = tempdir().expect("allowed root");
    fs::create_dir_all(allowed.path().join("src")).expect("src");
    fs::create_dir_all(allowed.path().join(".colossus")).expect("control");
    fs::create_dir_all(allowed.path().join(".git")).expect("git control");
    fs::write(
        allowed.path().join("src/example.rs"),
        "fn transition_to_rust() {}\n",
    )
    .expect("fixture");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("filesystem.list", DecisionOutcome::Allow)
        .with_action("filesystem.search", DecisionOutcome::Allow)
        .with_filesystem_read_root(allowed.path().display().to_string());
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["filesystem.list".into(), "filesystem.search".into()]),
        [5_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: allowed.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let context = ExecutionContext {
        correlation_id: "run-1".into(),
        run_id: Some("run-1".into()),
        ..ExecutionContext::default()
    };
    let listed = executor
        .execute(
            ToolCall {
                call_id: "call-list".into(),
                name: "filesystem.list".into(),
                arguments: json!({"path": "."}),
            },
            context.clone(),
        )
        .await
        .expect("list");
    let listed: serde_json::Value = serde_json::from_str(&listed.output).expect("list JSON");
    assert_eq!(listed["root"], ".");
    assert_eq!(listed["entries"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["entries"][0]["path"], "src");

    let searched = executor
        .execute(
            ToolCall {
                call_id: "call-search".into(),
                name: "filesystem.search".into(),
                arguments: json!({
                    "path": ".",
                    "pattern": "transition_to_rust",
                    "regex": false,
                }),
            },
            context,
        )
        .await
        .expect("search");
    let searched: serde_json::Value = serde_json::from_str(&searched.output).expect("search JSON");
    assert_eq!(
        searched["matches"][0]["path"]
            .as_str()
            .expect("match path")
            .replace('\\', "/"),
        "src/example.rs"
    );
    assert_eq!(searched["matches"][0]["line"], 1);

    let denied = executor
        .execute(
            ToolCall {
                call_id: "call-control".into(),
                name: "filesystem.list".into(),
                arguments: json!({"path": ".colossus"}),
            },
            ExecutionContext {
                offered_tools: vec![
                    "tool.search".into(),
                    "repo.map".into(),
                    "repo.symbol_search".into(),
                    "repo.references".into(),
                    "repo.file_summary".into(),
                ],
                ..ExecutionContext::default()
            },
        )
        .await
        .expect_err("control directory denied");
    assert!(matches!(denied, colossus_ports::ToolError::Denied(_)));
}

#[tokio::test]
async fn agent_mutations_require_approval_and_return_audited_diff_visibility() {
    let workspace = tempdir().expect("workspace");
    let target = workspace.path().join("note.txt");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let denied_gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("filesystem.write", DecisionOutcome::RequireApproval)
                .with_filesystem_root(workspace.path().display().to_string(), "write"),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["filesystem.write".into()]),
        [7_u8; 32],
    ));
    let denied_executor = GatewayToolExecutor {
        gateway: denied_gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let denied = denied_executor
        .execute(
            ToolCall {
                call_id: "write-denied".into(),
                name: "filesystem.write".into(),
                arguments: json!({
                    "path": "note.txt",
                    "content": "hello hello",
                    "mode": "create",
                }),
            },
            ExecutionContext::default(),
        )
        .await
        .expect_err("approval denied");
    assert!(matches!(denied, colossus_ports::ToolError::Denied(_)));
    assert!(!target.exists());

    let allowed_gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("filesystem.write", DecisionOutcome::RequireApproval)
                .with_filesystem_root(workspace.path().display().to_string(), "write"),
        ),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new(["filesystem.write".into()]),
        [8_u8; 32],
    ));
    let allowed_executor = GatewayToolExecutor {
        gateway: allowed_gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let written = allowed_executor
        .execute(
            ToolCall {
                call_id: "write-allowed".into(),
                name: "filesystem.write".into(),
                arguments: json!({
                    "path": "note.txt",
                    "content": "hello hello",
                    "mode": "create",
                }),
            },
            ExecutionContext::default(),
        )
        .await
        .expect("approved write");
    let written: serde_json::Value = serde_json::from_str(&written.output).expect("write JSON");
    assert!(
        written["diff"]
            .as_str()
            .is_some_and(|diff| diff.contains("+hello hello"))
    );

    let replaced = allowed_executor
        .execute(
            ToolCall {
                call_id: "replace-allowed".into(),
                name: "filesystem.replace".into(),
                arguments: json!({
                    "path": "note.txt",
                    "old": "hello",
                    "new": "hi",
                    "replace_all": true,
                }),
            },
            ExecutionContext::default(),
        )
        .await
        .expect("approved replace");
    let replaced: serde_json::Value = serde_json::from_str(&replaced.output).expect("replace JSON");
    assert_eq!(replaced["replacements"], 2);
    assert_eq!(fs::read_to_string(target).expect("read"), "hi hi");

    let names = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(names.contains(&"approval.denied.v1".into()));
    assert!(names.contains(&"approval.granted.v1".into()));
    assert!(names.contains(&"effect.release_requested.v1".into()));
}

#[tokio::test]
async fn model_work_tools_are_durable_attributed_and_session_confined() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    for id in ["session-a", "session-b"] {
        sessions
            .create_session(
                id,
                Some(id),
                Actor {
                    actor_type: ActorType::User,
                    id: "test-user".into(),
                },
            )
            .expect("session");
    }
    let repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let service = Arc::new(colossus_work::WorkService::new(
        Arc::clone(&repository),
        sessions,
    ));
    let work = Arc::new(WorkEffectExecutor {
        service,
        repository: Arc::clone(&repository),
        instruction_snapshots: Arc::new(InstructionSnapshotStore::new(Arc::clone(&journal))),
    });
    let actions = [
        "task.create",
        "task.update",
        "task.list",
        "decision.create",
        "decision.update",
        "decision.list",
        "decision.archive",
        "decision.supersede",
    ];
    let mut policy = colossus_policy::BuiltInPolicy::offline_default();
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(actions.map(str::to_owned)),
        [10_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: Some(work),
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let context = |session: &str| ExecutionContext {
        correlation_id: format!("run-{session}"),
        session_id: Some(session.into()),
        run_id: Some(format!("run-{session}")),
        ..ExecutionContext::default()
    };

    let created = executor
        .execute(
            ToolCall {
                call_id: "task-create".into(),
                name: "task.create".into(),
                arguments: json!({
                    "title": "Finish Rust transition",
                    "description": "Port durable model tools",
                }),
            },
            context("session-a"),
        )
        .await
        .expect("task create");
    let task: serde_json::Value = serde_json::from_str(&created.output).expect("task JSON");
    let task_id = task["id"].as_str().expect("task id").to_owned();
    assert_eq!(task["session_id"], "session-a");
    assert_eq!(task["status"], "pending");

    let denied = executor
        .execute(
            ToolCall {
                call_id: "task-cross-session".into(),
                name: "task.update".into(),
                arguments: json!({"id": task_id, "status": "completed"}),
            },
            context("session-b"),
        )
        .await
        .expect_err("cross-session task update denied");
    assert!(matches!(denied, colossus_ports::ToolError::Failed(_)));
    assert_eq!(
        repository
            .get_task(&task_id)
            .expect("task")
            .expect("record")
            .status,
        TaskStatus::Pending
    );

    let decision = executor
        .execute(
            ToolCall {
                call_id: "decision-create".into(),
                name: "decision.create".into(),
                arguments: json!({
                    "title": "Rust implementation",
                    "decision": "All new implementation work is Rust.",
                    "priority": "critical",
                    "rationale": "Complete the cutover",
                }),
            },
            context("session-a"),
        )
        .await
        .expect("decision create");
    let decision: serde_json::Value =
        serde_json::from_str(&decision.output).expect("decision JSON");
    assert_eq!(decision["source"], "agent");
    assert_eq!(decision["session_id"], "session-a");

    let listed = executor
        .execute(
            ToolCall {
                call_id: "decision-list".into(),
                name: "decision.list".into(),
                arguments: json!({"status": "active"}),
            },
            context("session-a"),
        )
        .await
        .expect("decision list");
    let listed: serde_json::Value = serde_json::from_str(&listed.output).expect("list JSON");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let task_events = journal
        .read_stream(&format!("task:{task_id}"))
        .expect("task events");
    assert_eq!(task_events[0].actor.actor_type, ActorType::Model);
    assert_eq!(task_events[0].actor.id, "tool-call:task-create");
    assert!(
        journal
            .read_global(1, 200)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "effect.release_requested.v1")
    );
}

#[tokio::test]
async fn subprocess_content_denied_post_effect_never_reaches_the_tool_caller() {
    let secret = "process-private-content";
    let workspace = tempdir().expect("workspace");
    let executable = std::env::current_exe()
        .expect("current executable")
        .canonicalize()
        .expect("canonical executable");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("shell.run", DecisionOutcome::Allow)
        .with_sandbox("native", "post-deny-process", false)
        .with_filesystem_root(executable.display().to_string(), "execute")
        .with_filesystem_read_root(workspace.path().display().to_string())
        .with_post_effect(true);
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(RuntimePostDenyPolicy(policy)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["shell.run".into()]),
        [54_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: Some(Arc::new(PrivateOutputProcess)),
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: vec![executable.clone()],
    };
    let error = executor
        .execute(
            ToolCall {
                call_id: "process-post-deny".into(),
                name: "shell.run".into(),
                arguments: json!({
                    "argv": [executable.display().to_string()],
                    "cwd": ".",
                }),
            },
            ExecutionContext {
                correlation_id: "process-post-deny".into(),
                run_id: Some("process-post-deny".into()),
                ..ExecutionContext::default()
            },
        )
        .await
        .expect_err("subprocess post-effect denial");
    assert!(matches!(error, colossus_ports::ToolError::Denied(_)));
    assert!(error.to_string().contains("post-effect release denied"));
    assert!(!error.to_string().contains(secret));

    let events = journal.read_global(1, 30).expect("effect events");
    let event_types = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"effect.started.v1"));
    assert!(event_types.contains(&"effect.release_requested.v1"));
    assert!(event_types.contains(&"effect.release_denied.v1"));
    assert!(!event_types.contains(&"effect.completed.v1"));
    assert!(
        !serde_json::to_string(&events)
            .expect("effect evidence")
            .contains(secret)
    );
}

#[tokio::test]
async fn actual_memory_content_denied_post_effect_never_reaches_the_caller() {
    let secret = "memory-private-content";
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    let repository: Arc<dyn colossus_ports::MemoryRepository> = Arc::new(
        colossus_memory::EventSourcedMemoryRepository::new(Arc::clone(&journal)),
    );
    let service = Arc::new(
        colossus_memory::MemoryService::new(
            Arc::clone(&journal),
            repository,
            external_work_queue(Arc::clone(&journal)),
            Arc::new(colossus_memory::UnavailableMemoryIndex::new(
                "post-deny fixture index",
            )),
            sessions,
        )
        .expect("memory service"),
    );
    let record = service
        .create(
            MemoryScope::Global,
            "fact",
            1.0,
            secret,
            "post-deny fixture",
            None,
            Actor {
                actor_type: ActorType::System,
                id: "memory-fixture".into(),
            },
        )
        .await
        .expect("memory fixture");
    let baseline = journal
        .read_global(1, 100)
        .expect("baseline events")
        .last()
        .map_or(0, |event| event.global_sequence);
    let executor = MemoryEffectExecutor {
        service,
        repository_id: "repo-test".into(),
    };
    let policy = BuiltInPolicy::offline_default()
        .with_action("memory.read", DecisionOutcome::Allow)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(RuntimePostDenyPolicy(policy)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["memory.read".into()]),
        [53_u8; 32],
    );
    let mut request = effect_request(
        terminal_actor(),
        "memory.read",
        record.id.clone(),
        serde_json::to_value(MemoryOperation::Read { id: record.id }).expect("memory operation"),
    );
    request.capabilities = vec!["memory.read".into()];
    let error = gateway
        .execute(request, &executor)
        .await
        .expect_err("memory post-effect denial");
    assert!(error.to_string().contains("post-effect release denied"));
    assert!(!error.to_string().contains(secret));

    let events = journal
        .read_global(baseline.saturating_add(1), 30)
        .expect("effect events");
    let event_types = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"effect.started.v1"));
    assert!(event_types.contains(&"effect.release_requested.v1"));
    assert!(event_types.contains(&"effect.release_denied.v1"));
    assert!(!event_types.contains(&"effect.completed.v1"));
    assert!(
        !serde_json::to_string(&events)
            .expect("effect evidence")
            .contains(secret)
    );
}

#[tokio::test]
async fn model_memory_tools_are_durable_scoped_and_post_gated() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    for id in ["session-a", "session-b"] {
        sessions
            .create_session(
                id,
                Some(id),
                Actor {
                    actor_type: ActorType::User,
                    id: "test-user".into(),
                },
            )
            .expect("session");
    }
    let repository: Arc<dyn colossus_ports::MemoryRepository> = Arc::new(
        colossus_memory::EventSourcedMemoryRepository::new(Arc::clone(&journal)),
    );
    let queue = external_work_queue(Arc::clone(&journal));
    let service = Arc::new(
        colossus_memory::MemoryService::new(
            Arc::clone(&journal),
            Arc::clone(&repository),
            queue,
            Arc::new(colossus_memory::UnavailableMemoryIndex::new(
                "test fallback index",
            )),
            sessions,
        )
        .expect("memory service"),
    );
    let memory = Arc::new(MemoryEffectExecutor {
        service,
        repository_id: "repo-test".into(),
    });
    let actions = [
        "memory.create",
        "memory.update",
        "memory.list",
        "memory.search",
        "memory.archive",
        "memory.supersede",
    ];
    let mut policy = colossus_policy::BuiltInPolicy::offline_default();
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(actions.map(str::to_owned)),
        [12_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: Some(memory),
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let context = |session: &str| ExecutionContext {
        correlation_id: format!("run-{session}"),
        session_id: Some(session.into()),
        run_id: Some(format!("run-{session}")),
        ..ExecutionContext::default()
    };
    let create = |call_id: &str, scope: &str, text: &str| ToolCall {
        call_id: call_id.into(),
        name: "memory.create".into(),
        arguments: json!({
            "scope": scope,
            "kind": "preference",
            "text": text,
            "confidence": 0.9,
        }),
    };

    let global = executor
        .execute(
            create("memory-global", "global", "Use auditable changes"),
            context("session-a"),
        )
        .await
        .expect("global create");
    let global: serde_json::Value = serde_json::from_str(&global.output).expect("global JSON");
    assert_eq!(global["scope"]["kind"], "global");
    let repository_memory = executor
        .execute(
            create("memory-repository", "repository", "Run workspace tests"),
            context("session-a"),
        )
        .await
        .expect("repository create");
    let repository_memory: serde_json::Value =
        serde_json::from_str(&repository_memory.output).expect("repository JSON");
    assert_eq!(repository_memory["scope"]["kind"], "repository");
    assert_eq!(repository_memory["scope"]["id"], "repo-test");
    let session_memory = executor
        .execute(
            create("memory-session", "session", "Private session preference"),
            context("session-a"),
        )
        .await
        .expect("session create");
    let session_memory: serde_json::Value =
        serde_json::from_str(&session_memory.output).expect("session JSON");
    let session_memory_id = session_memory["id"]
        .as_str()
        .expect("session memory id")
        .to_owned();
    assert_eq!(session_memory["scope"]["kind"], "session");
    assert_eq!(session_memory["scope"]["id"], "session-a");
    assert_eq!(session_memory["source"], "agent");

    let listed = executor
        .execute(
            ToolCall {
                call_id: "memory-list-b".into(),
                name: "memory.list".into(),
                arguments: json!({"status": "active", "limit": 2}),
            },
            context("session-b"),
        )
        .await
        .expect("scoped list");
    let listed: Vec<serde_json::Value> = serde_json::from_str(&listed.output).expect("list JSON");
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|record| record["id"] != session_memory_id)
    );

    let denied = executor
        .execute(
            ToolCall {
                call_id: "memory-cross-session".into(),
                name: "memory.update".into(),
                arguments: json!({"id": session_memory_id, "text": "not allowed"}),
            },
            context("session-b"),
        )
        .await
        .expect_err("cross-session update denied");
    assert!(matches!(denied, colossus_ports::ToolError::Failed(_)));

    let updated = executor
        .execute(
            ToolCall {
                call_id: "memory-update".into(),
                name: "memory.update".into(),
                arguments: json!({
                    "id": session_memory_id,
                    "text": "Private Rust session preference",
                    "confidence": 1.0,
                }),
            },
            context("session-a"),
        )
        .await
        .expect("memory update");
    let updated: serde_json::Value = serde_json::from_str(&updated.output).expect("updated JSON");
    assert_eq!(updated["source"], "agent");
    assert_eq!(updated["scope"]["kind"], "session");
    assert_eq!(updated["scope"]["id"], "session-a");

    let searched = executor
        .execute(
            ToolCall {
                call_id: "memory-search".into(),
                name: "memory.search".into(),
                arguments: json!({"query": "Private Rust", "limit": 5}),
            },
            context("session-a"),
        )
        .await
        .expect("memory search");
    let searched: Vec<serde_json::Value> =
        serde_json::from_str(&searched.output).expect("search JSON");
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0]["id"], session_memory_id);

    assert_eq!(
        repository
            .get_memory(&session_memory_id)
            .expect("memory")
            .expect("record")
            .status,
        MemoryStatus::Active
    );
    let events = journal
        .read_stream(&format!("memory:{session_memory_id}"))
        .expect("memory events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].actor.actor_type, ActorType::Model);
    assert_eq!(events[0].actor.id, "tool-call:memory-session");
    assert_eq!(events[1].event_type, "memory.updated.v1");
    assert_eq!(events[1].actor.id, "tool-call:memory-update");
    let global_scope: MemoryScope =
        serde_json::from_value(global["scope"].clone()).expect("global scope");
    assert_eq!(global_scope, MemoryScope::Global);
    assert!(
        journal
            .read_global(1, 500)
            .expect("events")
            .iter()
            .filter(|event| event.event_type == "effect.release_requested.v1")
            .count()
            >= 6
    );
}

#[tokio::test]
async fn model_plans_are_session_confined_and_approval_obligated() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    for id in ["session-a", "session-b"] {
        sessions
            .create_session(
                id,
                Some(id),
                Actor {
                    actor_type: ActorType::User,
                    id: "test-user".into(),
                },
            )
            .expect("session");
    }
    let repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let work = Arc::new(WorkEffectExecutor {
        service: Arc::new(colossus_work::WorkService::new(
            Arc::clone(&repository),
            sessions,
        )),
        repository: Arc::clone(&repository),
        instruction_snapshots: Arc::new(InstructionSnapshotStore::new(Arc::clone(&journal))),
    });
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("plan.create", DecisionOutcome::Allow)
        .with_action("plan.show", DecisionOutcome::Allow)
        .with_action("plan.approve_request", DecisionOutcome::RequireApproval);
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new([
            "plan.create".into(),
            "plan.show".into(),
            "plan.approve_request".into(),
        ]),
        [14_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: Some(work),
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let context = |session: &str| ExecutionContext {
        correlation_id: format!("run-{session}"),
        session_id: Some(session.into()),
        run_id: Some(format!("run-{session}")),
        ..ExecutionContext::default()
    };
    let created = executor
        .execute(
            ToolCall {
                call_id: "plan-create".into(),
                name: "plan.create".into(),
                arguments: json!({
                    "prompt": "Finish the Rust transition",
                    "content": "# Durable plan",
                    "steps": [
                        {"title": "Inspect", "detail": "Read the contracts"},
                        {"title": "Implement", "requires_mutation": true}
                    ],
                }),
            },
            context("session-a"),
        )
        .await
        .expect("plan create");
    let created: serde_json::Value = serde_json::from_str(&created.output).expect("plan JSON");
    let plan_id = created["id"].as_str().expect("plan id").to_owned();
    assert_eq!(created["session_id"], "session-a");
    assert_eq!(created["status"], "draft");
    assert_eq!(created["steps"][1]["index"], 2);

    let denied = executor
        .execute(
            ToolCall {
                call_id: "plan-show-cross-session".into(),
                name: "plan.show".into(),
                arguments: json!({"id": plan_id}),
            },
            context("session-b"),
        )
        .await
        .expect_err("cross-session plan read denied");
    assert!(matches!(denied, colossus_ports::ToolError::Failed(_)));

    let approved = executor
        .execute(
            ToolCall {
                call_id: "plan-approve".into(),
                name: "plan.approve_request".into(),
                arguments: json!({"id": plan_id}),
            },
            context("session-a"),
        )
        .await
        .expect("plan approved");
    let approved: serde_json::Value =
        serde_json::from_str(&approved.output).expect("approved JSON");
    assert_eq!(approved["status"], "approved");
    assert!(approved["approved_at"].as_str().is_some());
    assert_eq!(
        repository
            .get_plan(&plan_id)
            .expect("get")
            .expect("plan")
            .status,
        PlanStatus::Approved
    );
    let event_types = journal
        .read_global(1, 300)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"approval.granted.v1".into()));
    assert!(event_types.contains(&"plan.approved.v1".into()));
    assert!(event_types.contains(&"effect.release_requested.v1".into()));
}

#[tokio::test]
async fn model_subagent_tools_inject_lineage_scope_results_and_deny_recursion() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    for id in ["session-a", "session-b"] {
        sessions
            .create_session(
                id,
                Some(id),
                Actor {
                    actor_type: ActorType::User,
                    id: "test-user".into(),
                },
            )
            .expect("session");
    }
    let repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let instruction_snapshots = Arc::new(InstructionSnapshotStore::new(Arc::clone(&journal)));
    let work = Arc::new(WorkEffectExecutor {
        service: Arc::new(colossus_work::WorkService::new(
            Arc::clone(&repository),
            Arc::clone(&sessions),
        )),
        repository: Arc::clone(&repository),
        instruction_snapshots: Arc::clone(&instruction_snapshots),
    });
    let actions = ["subagent.create", "subagent.read", "subagent.list"];
    let mut policy = colossus_policy::BuiltInPolicy::offline_default();
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    let executor = GatewayToolExecutor {
        gateway: Arc::new(colossus_policy::EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(colossus_policy::DenyApproval),
            colossus_policy::SafetyKernel::new(actions.map(str::to_owned)),
            [16_u8; 32],
        )),
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: Some(work),
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let context = |session: &str| ExecutionContext {
        correlation_id: "run-parent".into(),
        session_id: Some(session.into()),
        run_id: Some("run-parent".into()),
        offered_tools: vec!["agent.delegate".into(), "echo".into()],
        ..ExecutionContext::default()
    };
    let instruction_workspace = tempdir().expect("instruction workspace");
    fs::write(
        instruction_workspace.path().join("AGENTS.md"),
        "rules captured before delegation",
    )
    .expect("initial AGENTS.md");
    let snapshot = Arc::new(
        super::InstructionSnapshot::capture(
            None,
            instruction_workspace.path(),
            "parent invocation rules",
            "immutable Plan Mode rules",
        )
        .expect("instruction snapshot"),
    );
    let snapshot_id = snapshot.id().to_owned();
    let created = super::scope_instruction_snapshot(
        Some(snapshot),
        executor.execute(
            ToolCall {
                call_id: "delegate-1".into(),
                name: "agent.delegate".into(),
                arguments: json!({"task": "Review the Rust tests"}),
            },
            context("session-a"),
        ),
    )
    .await
    .expect("delegate");
    let created: serde_json::Value = serde_json::from_str(&created.output).expect("job JSON");
    let id = created["id"].as_str().expect("id").to_owned();
    assert_eq!(created["parent_run_id"], "run-parent");
    assert_eq!(created["parent_call_id"], "delegate-1");
    assert_eq!(created["status"], "queued");
    assert!(
        created.get("instruction_snapshot_id").is_none(),
        "private instruction provenance must not enter the public child-job DTO"
    );
    assert_eq!(
        repository
            .subagent_instruction_snapshot_id(&id)
            .expect("snapshot reference")
            .as_deref(),
        Some(snapshot_id.as_str())
    );
    fs::write(
        instruction_workspace.path().join("AGENTS.md"),
        "rules changed after delegation",
    )
    .expect("changed AGENTS.md");
    let recovered_repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let recovered_store = InstructionSnapshotStore::new(Arc::clone(&journal));
    let recovered_snapshot_id = recovered_repository
        .subagent_instruction_snapshot_id(&id)
        .expect("recovered snapshot reference")
        .expect("durable snapshot reference");
    let recovered_instructions = recovered_store
        .load(&recovered_snapshot_id)
        .expect("snapshot after repository reopen")
        .compose();
    assert!(recovered_instructions.contains("rules captured before delegation"));
    assert!(!recovered_instructions.contains("rules changed after delegation"));
    assert!(recovered_instructions.contains("immutable Plan Mode rules"));
    assert!(
        instruction_snapshots
            .load(&snapshot_id)
            .expect("durable snapshot")
            .compose()
            .contains("parent invocation rules")
    );
    assert!(
        instruction_snapshots
            .load(&snapshot_id)
            .expect("durable snapshot")
            .compose()
            .contains("immutable Plan Mode rules"),
        "delegated recovery must retain the parent's immutable runtime mode"
    );
    assert_eq!(
        created["allowed_tools"],
        json!(["agent.delegate", "echo"]),
        "the durable child job must preserve the parent's exact offered-tool ceiling"
    );
    assert!(
        sessions
            .get_session(created["child_session_id"].as_str().expect("child"))
            .expect("child session")
            .is_some()
    );

    let denied = executor
        .execute(
            ToolCall {
                call_id: "result-cross".into(),
                name: "agent.result".into(),
                arguments: json!({"id": id}),
            },
            context("session-b"),
        )
        .await
        .expect_err("cross-session result denied");
    assert!(matches!(denied, colossus_ports::ToolError::Failed(_)));

    let mut child_context = context("session-a");
    child_context.subagent_id = Some(id.clone());
    let nested = executor
        .execute(
            ToolCall {
                call_id: "nested".into(),
                name: "agent.delegate".into(),
                arguments: json!({"task": "Delegate again"}),
            },
            child_context,
        )
        .await
        .expect_err("nested delegation denied");
    assert!(matches!(nested, colossus_ports::ToolError::Denied(_)));
    let events = journal
        .read_stream(&format!("subagent:{id}"))
        .expect("events");
    assert_eq!(events[0].actor.actor_type, ActorType::Model);
    assert_eq!(events[0].actor.id, "tool-call:delegate-1");
}

struct WorkScriptedProvider {
    turns: Mutex<VecDeque<ProviderTurn>>,
    requests: Mutex<Vec<ModelRequest>>,
}

#[async_trait::async_trait]
impl ModelProvider for WorkScriptedProvider {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        Ok(ProviderRoute {
            role: role.into(),
            profile: "scripted".into(),
            model_profile: "scripted".into(),
            provider_profile: "scripted-provider".into(),
            provider: "test".into(),
            model: "test-model".into(),
            limits: ModelLimits {
                context_window_tokens: 32_768,
                max_output_tokens: 4_096,
                safety_margin_tokens: 3_276,
                input_budget_tokens: 25_396,
            },
            capabilities: ModelCapabilities {
                tool_calls: true,
                streaming: true,
                image_inputs: false,
            },
            reasoning_effort: None,
        })
    }

    async fn turn(
        &self,
        _role: &str,
        request: ModelRequest,
        _context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.requests.lock().expect("requests").push(request);
        self.turns
            .lock()
            .expect("turns")
            .pop_front()
            .ok_or_else(|| ModelProviderError::Failed("script exhausted".into()))
    }
}

#[tokio::test]
async fn risk_evaluator_uses_strict_json_tools_disabled_and_redacted_metadata() {
    let valid = ProviderTurn {
        profile: "scripted".into(),
        model_profile: "scripted".into(),
        provider_profile: "scripted-provider".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_id: Some("risk-valid".into()),
        events: vec![ProviderEvent::FinalOutput {
            text: serde_json::json!({
                "risk_level": "low",
                "recommended_decision": "allow",
                "reason": "bounded read-only inspection"
            })
            .to_string(),
        }],
    };
    let invalid = ProviderTurn {
        profile: "scripted".into(),
        model_profile: "scripted".into(),
        provider_profile: "scripted-provider".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_id: Some("risk-invalid".into()),
        events: vec![ProviderEvent::FinalOutput {
            text: serde_json::json!({
                "risk_level": "low",
                "recommended_decision": "allow",
                "reason": "looks safe",
                "confidence": 1.0
            })
            .to_string(),
        }],
    };
    let fenced = ProviderTurn {
        profile: "scripted".into(),
        model_profile: "scripted".into(),
        provider_profile: "scripted-provider".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_id: Some("risk-fenced".into()),
        events: vec![ProviderEvent::FinalOutput {
            text: concat!(
                "```json\n",
                "{\"risk_level\":\"low\",\"recommended_decision\":\"allow\",",
                "\"reason\":\"bounded local read-only search\"}\n",
                "```"
            )
            .into(),
        }],
    };
    let fenced_with_prose = ProviderTurn {
        profile: "scripted".into(),
        model_profile: "scripted".into(),
        provider_profile: "scripted-provider".into(),
        provider: "test".into(),
        model: "test-model".into(),
        response_id: Some("risk-fenced-prose".into()),
        events: vec![ProviderEvent::FinalOutput {
            text: concat!(
                "Assessment:\n```json\n",
                "{\"risk_level\":\"low\",\"recommended_decision\":\"allow\",",
                "\"reason\":\"looks safe\"}\n",
                "```"
            )
            .into(),
        }],
    };
    let provider = Arc::new(WorkScriptedProvider {
        turns: Mutex::new(VecDeque::from([valid, fenced, invalid, fenced_with_prose])),
        requests: Mutex::new(Vec::new()),
    });
    let evaluator = GatewayRiskEvaluator {
        provider: Arc::clone(&provider) as Arc<dyn ModelProvider>,
    };
    let mut request = effect_request(
        terminal_actor(),
        "shell.run",
        "/usr/bin/example",
        json!({
            "cwd": "/workspace",
            "args": ["inspect", "--token", "argument-secret"],
            "environment": {"SERVICE_TOKEN": "environment-secret"},
            "stdin_base64": "c3RkaW4tc2VjcmV0",
            "timeout_ms": 1000,
            "max_output_bytes": 4096,
        }),
    );
    request.capabilities = vec!["shell.run".into()];
    let decision = BuiltInPolicy::offline_default()
        .decide(&effect_request(
            terminal_actor(),
            "provider.echo",
            "provider:echo",
            json!({"message": "decision"}),
        ))
        .await
        .expect("decision");

    let assessment = evaluator
        .evaluate(&request, &decision)
        .await
        .expect("assessment");
    assert_eq!(assessment.risk_level, RiskLevel::Low);
    assert_eq!(assessment.recommended_decision, RiskRecommendation::Allow);
    {
        let recorded = provider.requests.lock().expect("requests");
        let model_request = recorded.first().expect("model request");
        assert!(model_request.tools.is_empty());
        assert!(
            !model_request.instructions.contains("[Colossus "),
            "the internal risk evaluator bypasses user-facing instruction composition"
        );
        let disclosed = &model_request.messages[0].content;
        assert!(!disclosed.contains("argument-secret"));
        assert!(!disclosed.contains("environment-secret"));
        assert!(!disclosed.contains("c3RkaW4tc2VjcmV0"));
        assert!(disclosed.contains("SERVICE_TOKEN"));
        assert!(disclosed.contains("[REDACTED]"));
    }

    let fenced_assessment = evaluator
        .evaluate(&request, &decision)
        .await
        .expect("single JSON fence");
    assert_eq!(fenced_assessment.risk_level, RiskLevel::Low);
    assert_eq!(
        fenced_assessment.recommended_decision,
        RiskRecommendation::Allow
    );

    assert!(matches!(
        evaluator.evaluate(&request, &decision).await,
        Err(RiskEvaluationError::InvalidAssessment(_))
    ));
    assert!(matches!(
        evaluator.evaluate(&request, &decision).await,
        Err(RiskEvaluationError::InvalidAssessment(_))
    ));
}

#[tokio::test]
async fn mcp_risk_metadata_is_exact_bounded_and_credential_free() {
    let request = {
        let mut request = effect_request(
            terminal_actor(),
            "mcp.call",
            "http://127.0.0.1:3001/mcp",
            json!({
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
                    "arguments": {
                        "message": "MCP tool test",
                        "password": "resolved-argument-secret",
                        "token": "resolved-token-secret",
                        "github_token": "resolved-compound-secret",
                        "dbPassword": "resolved-camel-secret",
                        "clientSecret": "resolved-client-secret",
                        "apiKey": "resolved-api-key-secret",
                        "service-api-key": "resolved-separated-api-key-secret",
                        "max_output_tokens": 512,
                        "nested": {
                            "access_token": "nested-resolved-secret",
                            "refreshTokenValue": "nested-camel-token-secret",
                            "credentialBundle": {
                                "value": "nested-compound-credential-secret"
                            },
                        },
                        "monkey": "ordinary-non-secret-value",
                    },
                    "input_schema": {
                        "type": "object",
                        "description": "schema-only-marker",
                    },
                    "schema_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                },
                "transport": "streamable_http",
                "cwd": null,
                "args": [],
                "environment": {"CHILD_TOKEN": "env:HOST_MCP_TOKEN"},
                "url": "http://127.0.0.1:3001/mcp",
                "headers": {"X-Client": "colossus"},
                "credential_headers": {
                    "Authorization": {"scheme": "Bearer", "reference": "env:HOST_MCP_TOKEN"}
                },
                "allow_stateless": true,
                "oauth": {
                    "clientId": "sensitive-client-context",
                    "clientSecretReference": "env:HOST_MCP_CLIENT_SECRET",
                    "callbackPort": 8787,
                    "scopes": ["openid"],
                },
                "timeout_ms": 30_000,
                "max_output_bytes": 1_048_576,
                "provenance": null,
            }),
        );
        request.credential_references = vec![CredentialReference {
            reference: "env:HOST_MCP_TOKEN".into(),
            value_hash: None,
        }];
        request
    };
    let decision = BuiltInPolicy::offline_default()
        .with_action("mcp.call", DecisionOutcome::RequireApproval)
        .decide(&request)
        .await
        .expect("decision");

    let metadata = redacted_risk_metadata(&request, &decision);
    assert_eq!(
        metadata["proposed_effect"]["endpoint"]["allow_stateless"],
        true
    );
    let disclosed = serde_json::to_string(&metadata).expect("metadata");
    assert!(disclosed.contains("http://127.0.0.1:3001/mcp"));
    assert!(disclosed.contains("everything"));
    assert!(disclosed.contains("Echo one bounded message"));
    assert!(disclosed.contains("readOnlyHint"));
    assert!(disclosed.contains("MCP tool test"));
    assert!(disclosed.contains(&"a".repeat(64)));
    assert!(disclosed.contains("[REDACTED]"));
    assert!(!disclosed.contains("resolved-argument-secret"));
    assert!(!disclosed.contains("resolved-token-secret"));
    assert!(!disclosed.contains("resolved-compound-secret"));
    assert!(!disclosed.contains("resolved-camel-secret"));
    assert!(!disclosed.contains("resolved-client-secret"));
    assert!(!disclosed.contains("resolved-api-key-secret"));
    assert!(!disclosed.contains("nested-resolved-secret"));
    assert!(!disclosed.contains("resolved-separated-api-key-secret"));
    assert!(!disclosed.contains("nested-camel-token-secret"));
    assert!(!disclosed.contains("nested-compound-credential-secret"));
    assert!(disclosed.contains("ordinary-non-secret-value"));
    assert!(
        disclosed.contains("\"max_output_tokens\":512"),
        "word-based redaction must keep non-secret names such as token counts"
    );
    assert!(!disclosed.contains("schema-only-marker"));
    assert!(!disclosed.contains("HOST_MCP_TOKEN"));
    assert!(!disclosed.contains("HOST_MCP_CLIENT_SECRET"));
    assert!(!disclosed.contains("sensitive-client-context"));
    assert!(!disclosed.contains("Authorization"));

    let mut stdio_request = request.clone();
    stdio_request.resource = "/usr/local/bin/everything-mcp".into();
    stdio_request.content["transport"] = json!("stdio");
    stdio_request.content["url"] = Value::Null;
    stdio_request.content["cwd"] = json!("/workspace");
    stdio_request.content["args"] = json!(["--token", "resolved-stdio-secret"]);
    let stdio_disclosed = serde_json::to_string(&redacted_risk_metadata(&stdio_request, &decision))
        .expect("stdio metadata");
    assert!(stdio_disclosed.contains("/usr/local/bin/everything-mcp"));
    assert!(stdio_disclosed.contains("CHILD_TOKEN"));
    assert!(stdio_disclosed.contains("[REDACTED]"));
    assert!(!stdio_disclosed.contains("resolved-stdio-secret"));
    assert!(!stdio_disclosed.contains("HOST_MCP_TOKEN"));
}

#[tokio::test]
async fn decision_created_by_one_model_turn_binds_the_next_turn_context() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    let repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let work_service = Arc::new(colossus_work::WorkService::new(
        Arc::clone(&repository),
        Arc::clone(&sessions),
    ));
    let work = Arc::new(WorkEffectExecutor {
        service: work_service,
        repository: Arc::clone(&repository),
        instruction_snapshots: Arc::new(InstructionSnapshotStore::new(Arc::clone(&journal))),
    });
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("decision.create", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["decision.create".into()]),
        [11_u8; 32],
    ));
    let executor: Arc<dyn ToolExecutor> = Arc::new(GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: Some(work),
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    });
    let provider = Arc::new(WorkScriptedProvider {
        turns: Mutex::new(VecDeque::from([
            ProviderTurn {
                profile: "scripted".into(),
                model_profile: "scripted".into(),
                provider_profile: "scripted-provider".into(),
                provider: "test".into(),
                model: "test-model".into(),
                response_id: None,
                events: vec![ProviderEvent::ToolCallRequested {
                    call_id: "decision-call".into(),
                    name: "decision.create".into(),
                    arguments: json!({
                        "title": "Rust-only implementation",
                        "decision": "All new implementation work must be written in Rust.",
                        "priority": "critical",
                    }),
                }],
            },
            ProviderTurn {
                profile: "scripted".into(),
                model_profile: "scripted".into(),
                provider_profile: "scripted-provider".into(),
                provider: "test".into(),
                model: "test-model".into(),
                response_id: None,
                events: vec![ProviderEvent::FinalOutput {
                    text: "decision retained".into(),
                }],
            },
        ])),
        requests: Mutex::new(Vec::new()),
    });
    let context = colossus_context::ContextService::new(
        colossus_context::ContextConfig {
            model_assisted: false,
            ..colossus_context::ContextConfig::default()
        },
        Arc::clone(&sessions),
        Arc::new(colossus_context::EventSourcedContextRepository::new(
            Arc::clone(&journal),
        )),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
    )
    .expect("context")
    .with_work_repository(Arc::clone(&repository));
    let agent = colossus_agent::AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            colossus_tools::StaticToolRegistry::builtins(&["decision.create".into()])
                .expect("tools"),
        ),
        executor,
        sessions,
    )
    .with_context_preparer(Arc::new(context));

    let result = agent
        .run(
            "primary",
            "You are Colossus.",
            "Remember our implementation rule.",
            3,
        )
        .await
        .expect("agent run");
    assert_eq!(result.output, "decision retained");
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].messages[0].role,
        colossus_contracts::ModelMessageRole::System
    );
    assert!(
        requests[1].messages[0]
            .content
            .starts_with("[Binding active key decisions]")
    );
    assert!(
        requests[1].messages[0]
            .content
            .contains("All new implementation work must be written in Rust.")
    );
    let decisions = repository
        .list_decisions(
            result.session_id.as_deref(),
            Some(colossus_contracts::DecisionStatus::Active),
            10,
        )
        .expect("decisions");
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        decisions[0].source,
        colossus_contracts::DecisionSource::Agent
    );
}

#[tokio::test]
async fn memory_created_by_one_model_turn_is_retrieved_for_the_next_turn() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    let repository: Arc<dyn colossus_ports::MemoryRepository> = Arc::new(
        colossus_memory::EventSourcedMemoryRepository::new(Arc::clone(&journal)),
    );
    let queue = external_work_queue(Arc::clone(&journal));
    let memory_service = Arc::new(
        colossus_memory::MemoryService::new(
            Arc::clone(&journal),
            Arc::clone(&repository),
            queue,
            Arc::new(colossus_memory::UnavailableMemoryIndex::new(
                "test fallback index",
            )),
            Arc::clone(&sessions),
        )
        .expect("memory service"),
    );
    let memory = Arc::new(MemoryEffectExecutor {
        service: memory_service,
        repository_id: "repo-test".into(),
    });
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("memory.create", DecisionOutcome::Allow)
                .with_action("memory.search", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["memory.create".into(), "memory.search".into()]),
        [13_u8; 32],
    ));
    let executor: Arc<dyn ToolExecutor> = Arc::new(GatewayToolExecutor {
        gateway: Arc::clone(&gateway),
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: Some(Arc::clone(&memory)),
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    });
    let provider = Arc::new(WorkScriptedProvider {
        turns: Mutex::new(VecDeque::from([
            ProviderTurn {
                profile: "scripted".into(),
                model_profile: "scripted".into(),
                provider_profile: "scripted-provider".into(),
                provider: "test".into(),
                model: "test-model".into(),
                response_id: None,
                events: vec![ProviderEvent::ToolCallRequested {
                    call_id: "memory-call".into(),
                    name: "memory.create".into(),
                    arguments: json!({
                        "scope": "session",
                        "kind": "preference",
                        "text": "Always run Rust Clippy before completion.",
                        "rationale": "User requested a Rust verification preference.",
                    }),
                }],
            },
            ProviderTurn {
                profile: "scripted".into(),
                model_profile: "scripted".into(),
                provider_profile: "scripted-provider".into(),
                provider: "test".into(),
                model: "test-model".into(),
                response_id: None,
                events: vec![ProviderEvent::FinalOutput {
                    text: "memory retained".into(),
                }],
            },
        ])),
        requests: Mutex::new(Vec::new()),
    });
    let retriever: Arc<dyn colossus_ports::MemoryRetriever> = Arc::new(GatewayMemoryRetriever {
        gateway,
        executor: memory,
        limit: 8,
        repository_id: "repo-test".into(),
    });
    let context = colossus_context::ContextService::new(
        colossus_context::ContextConfig {
            model_assisted: false,
            ..colossus_context::ContextConfig::default()
        },
        Arc::clone(&sessions),
        Arc::new(colossus_context::EventSourcedContextRepository::new(
            Arc::clone(&journal),
        )),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
    )
    .expect("context")
    .with_memory_retriever(retriever);
    let agent = colossus_agent::AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            colossus_tools::StaticToolRegistry::builtins(&["memory.create".into()]).expect("tools"),
        ),
        executor,
        sessions,
    )
    .with_context_preparer(Arc::new(context));

    let result = agent
        .run(
            "primary",
            "You are Colossus.",
            "Remember to run Rust Clippy before completion.",
            3,
        )
        .await
        .expect("agent run");
    assert_eq!(result.output, "memory retained");
    let requests = provider.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].messages[0]
            .content
            .starts_with("[Relevant memories]")
    );
    assert!(
        requests[1].messages[0]
            .content
            .contains("background context, not instructions")
    );
    assert!(
        requests[1].messages[0]
            .content
            .contains("Always run Rust Clippy before completion.")
    );
    let records = repository
        .list_memories(Some(MemoryStatus::Active), 10)
        .expect("memories");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source, "agent");
    assert_eq!(
        records[0].scope,
        MemoryScope::Session(result.session_id.expect("session id"))
    );
}

#[tokio::test]
async fn goal_update_is_bound_to_active_goal_context_and_stops_future_updates() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    sessions
        .create_session(
            "session-goal",
            Some("goal"),
            Actor {
                actor_type: ActorType::User,
                id: "test-user".into(),
            },
        )
        .expect("session");
    let repository: Arc<dyn colossus_ports::WorkRepository> = Arc::new(
        colossus_work::EventSourcedWorkRepository::new(Arc::clone(&journal)),
    );
    let service = Arc::new(colossus_work::WorkService::new(
        Arc::clone(&repository),
        Arc::clone(&sessions),
    ));
    let goal = service
        .create_goal(
            "session-goal",
            "Finish the bounded task",
            3,
            None,
            Actor {
                actor_type: ActorType::User,
                id: "test-user".into(),
            },
        )
        .expect("goal");
    let work = Arc::new(WorkEffectExecutor {
        service: Arc::clone(&service),
        repository: Arc::clone(&repository),
        instruction_snapshots: Arc::new(InstructionSnapshotStore::new(Arc::clone(&journal))),
    });
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("goal.show", DecisionOutcome::Allow)
                .with_action("goal.update", DecisionOutcome::Allow),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["goal.show".into(), "goal.update".into()]),
        [15_u8; 32],
    ));
    let executor: Arc<dyn ToolExecutor> = Arc::new(GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: Some(work),
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: std::env::current_dir().expect("cwd"),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    });
    let provider = Arc::new(WorkScriptedProvider {
        turns: Mutex::new(VecDeque::from([
            ProviderTurn {
                profile: "scripted".into(),
                model_profile: "scripted".into(),
                provider_profile: "scripted-provider".into(),
                provider: "test".into(),
                model: "test-model".into(),
                response_id: None,
                events: vec![ProviderEvent::ToolCallRequested {
                    call_id: "goal-complete".into(),
                    name: "goal.update".into(),
                    arguments: json!({
                        "status": "complete",
                        "summary": "Bounded task verified.",
                    }),
                }],
            },
            ProviderTurn {
                profile: "scripted".into(),
                model_profile: "scripted".into(),
                provider_profile: "scripted-provider".into(),
                provider: "test".into(),
                model: "test-model".into(),
                response_id: None,
                events: vec![ProviderEvent::FinalOutput {
                    text: "done".into(),
                }],
            },
        ])),
        requests: Mutex::new(Vec::new()),
    });
    let agent = colossus_agent::AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(
            colossus_tools::StaticToolRegistry::builtins(&[
                "goal.show".into(),
                "goal.update".into(),
            ])
            .expect("tools"),
        ),
        executor,
        sessions,
    );
    let result = agent
        .run_goal_iteration(
            "primary",
            "Use goal.update only when done.",
            "Finish now.",
            3,
            "session-goal",
            &goal.id,
            None,
        )
        .await
        .expect("goal iteration");
    assert_eq!(result.output, "done");
    let completed = repository
        .get_goal(&goal.id)
        .expect("goal")
        .expect("record");
    assert_eq!(completed.status, GoalStatus::Complete);
    assert_eq!(completed.summary, "Bounded task verified.");
    let run_events = journal
        .read_stream(&format!("run:{}", result.run_id))
        .expect("run events");
    assert!(
        run_events
            .iter()
            .all(|event| { event.context.goal_id.as_deref() == Some(goal.id.as_str()) })
    );
    assert!(
        service
            .update_goal_status(
                &goal.id,
                GoalStatus::Blocked,
                "",
                "too late",
                Actor {
                    actor_type: ActorType::User,
                    id: "test-user".into(),
                },
            )
            .is_err()
    );
}

#[tokio::test]
async fn trace_tools_expose_metadata_only_and_export_through_the_gateway() {
    let workspace = tempdir().expect("workspace");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "run:trace-run".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "model.request.prepared.v1".into(),
            actor: Actor {
                actor_type: ActorType::Model,
                id: "trace-model".into(),
            },
            context: ExecutionContext {
                correlation_id: "trace-run".into(),
                run_id: Some("trace-run".into()),
                ..ExecutionContext::default()
            },
            payload: json!({"secret": "must-not-export"}),
        })
        .expect("trace event");
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_post_effect(true)
        .with_action("trace.export", DecisionOutcome::RequireApproval)
        .with_filesystem_root(workspace.path().display().to_string(), "write");
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new(["trace.export".into()]),
        [47_u8; 32],
    ));
    let executor = TraceToolExecutor {
        journal: Arc::clone(&journal),
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        workspace: workspace.path().to_path_buf(),
        inner: Arc::new(UnusedToolExecutor),
    };
    let context = ExecutionContext {
        correlation_id: "trace-run".into(),
        run_id: Some("trace-run".into()),
        ..ExecutionContext::default()
    };
    let shown = executor
        .execute(
            ToolCall {
                call_id: "trace-show".into(),
                name: "trace.show".into(),
                arguments: json!({}),
            },
            context.clone(),
        )
        .await
        .expect("trace show");
    let shown: Value = serde_json::from_str(&shown.output).expect("trace JSON");
    assert_eq!(shown["available"], true);
    assert_eq!(
        shown["events"][0]["event_type"],
        "model.request.prepared.v1"
    );
    assert!(!shown.to_string().contains("must-not-export"));
    assert!(!shown.to_string().contains("ciphertext"));

    let exported = executor
        .execute(
            ToolCall {
                call_id: "trace-export".into(),
                name: "trace.export".into(),
                arguments: json!({"path": "trace.json"}),
            },
            context,
        )
        .await
        .expect("trace export");
    let exported: Value = serde_json::from_str(&exported.output).expect("export JSON");
    assert_eq!(exported["path"], "trace.json");
    let content = fs::read_to_string(workspace.path().join("trace.json")).expect("export");
    assert!(content.contains("model.request.prepared.v1"));
    assert!(!content.contains("must-not-export"));
    assert!(!content.contains("ciphertext"));
    let event_types = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"approval.granted.v1".into()));
}

#[tokio::test]
async fn model_patch_tools_preview_apply_and_reverse_exact_text_under_policy() {
    let workspace = tempdir().expect("workspace");
    let target = workspace.path().join("note.txt");
    fs::write(&target, "alpha\nbeta\n").expect("fixture");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_post_effect(true)
        .with_action("patch.preview", DecisionOutcome::Allow)
        .with_action("patch.apply", DecisionOutcome::RequireApproval)
        .with_action("patch.reverse", DecisionOutcome::RequireApproval)
        .with_filesystem_root(workspace.path().display().to_string(), "write");
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new([
            "patch.preview".into(),
            "patch.apply".into(),
            "patch.reverse".into(),
        ]),
        [46_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let arguments = || json!({"path": "note.txt", "old": "beta", "new": "gamma"});
    let invoke = |name: &str| ToolCall {
        call_id: format!("call-{name}"),
        name: name.into(),
        arguments: arguments(),
    };

    let preview = executor
        .execute(invoke("patch.preview"), ExecutionContext::default())
        .await
        .expect("preview");
    assert!(preview.output.contains("+gamma"));
    assert_eq!(fs::read_to_string(&target).expect("read"), "alpha\nbeta\n");
    let applied = executor
        .execute(invoke("patch.apply"), ExecutionContext::default())
        .await
        .expect("apply");
    let applied: Value = serde_json::from_str(&applied.output).expect("apply JSON");
    assert_eq!(applied["changed_line_ranges"][0]["start"], 2);
    assert_eq!(fs::read_to_string(&target).expect("read"), "alpha\ngamma\n");
    executor
        .execute(invoke("patch.reverse"), ExecutionContext::default())
        .await
        .expect("reverse");
    assert_eq!(fs::read_to_string(&target).expect("read"), "alpha\nbeta\n");
    let event_types = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"approval.granted.v1".into()));
}

#[tokio::test]
async fn context_tools_authorize_reads_and_mutations_with_session_bound_provenance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn colossus_ports::SessionRepository> = Arc::new(
        colossus_session::EventSourcedSessionRepository::new(Arc::clone(&journal)),
    );
    sessions
        .create_session(
            "session-context",
            Some("Context tools"),
            Actor {
                actor_type: ActorType::User,
                id: "test-user".into(),
            },
        )
        .expect("session");
    for (index, (role, content)) in [
        (ModelMessageRole::User, "Remember the Rust boundary."),
        (ModelMessageRole::Assistant, "The boundary is retained."),
    ]
    .into_iter()
    .enumerate()
    {
        sessions
            .append_message(
                "session-context",
                "run-context",
                ModelMessage {
                    role,
                    content: content.into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                Actor {
                    actor_type: ActorType::User,
                    id: format!("message-{index}"),
                },
            )
            .expect("message");
    }
    let provider = Arc::new(WorkScriptedProvider {
        turns: Mutex::new(VecDeque::new()),
        requests: Mutex::new(Vec::new()),
    });
    let context_service = Arc::new(
        colossus_context::ContextService::new(
            colossus_context::ContextConfig {
                model_assisted: false,
                ..colossus_context::ContextConfig::default()
            },
            Arc::clone(&sessions),
            Arc::new(colossus_context::EventSourcedContextRepository::new(
                Arc::clone(&journal),
            )),
            provider as Arc<dyn ModelProvider>,
        )
        .expect("context service"),
    );
    let registry: Arc<dyn colossus_ports::ToolRegistry> = Arc::new(
        colossus_tools::StaticToolRegistry::builtins(&[
            "context.show".into(),
            "context.compact".into(),
            "context.snapshots".into(),
            "context.restore".into(),
        ])
        .expect("tools"),
    );
    let context_executor = Arc::new(ContextEffectExecutor {
        service: context_service,
        tool_definitions: colossus_tools::model_definitions(registry.as_ref()),
    });
    let actions = [
        "context.show",
        "context.compact",
        "context.snapshots",
        "context.restore",
    ];
    let mut policy = colossus_policy::BuiltInPolicy::offline_default().with_post_effect(true);
    for action in &actions {
        policy = policy.with_action(
            *action,
            if matches!(*action, "context.compact" | "context.restore") {
                DecisionOutcome::RequireApproval
            } else {
                DecisionOutcome::Allow
            },
        );
    }
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new(actions.map(str::to_owned)),
        [45_u8; 32],
    ));
    let executor = ContextToolExecutor {
        gateway,
        context: context_executor,
        inner: Arc::new(UnusedToolExecutor),
    };
    let execution_context = ExecutionContext {
        correlation_id: "run-context".into(),
        session_id: Some("session-context".into()),
        run_id: Some("run-context".into()),
        ..ExecutionContext::default()
    };
    let call = |name: &str, arguments: Value| ToolCall {
        call_id: format!("call-{name}"),
        name: name.into(),
        arguments,
    };

    let shown = executor
        .execute(call("context.show", json!({})), execution_context.clone())
        .await
        .expect("context show");
    let shown: Value = serde_json::from_str(&shown.output).expect("show JSON");
    assert_eq!(shown["session_id"], "session-context");

    let compacted = executor
        .execute(
            call("context.compact", json!({})),
            execution_context.clone(),
        )
        .await
        .expect("context compact");
    let compacted: Value = serde_json::from_str(&compacted.output).expect("compact JSON");
    let snapshot_id = compacted["snapshot_id"]
        .as_str()
        .expect("snapshot id")
        .to_owned();
    assert_eq!(compacted["snapshot_created"], true);

    let snapshots = executor
        .execute(
            call("context.snapshots", json!({})),
            execution_context.clone(),
        )
        .await
        .expect("context snapshots");
    let snapshots: Value = serde_json::from_str(&snapshots.output).expect("snapshots JSON");
    assert_eq!(snapshots.as_array().map(Vec::len), Some(1));

    executor
        .execute(
            call("context.restore", json!({"snapshot_id": snapshot_id})),
            execution_context,
        )
        .await
        .expect("context restore");
    let session_events = journal
        .read_stream("session:session-context")
        .expect("session events");
    let created = session_events
        .iter()
        .find(|event| event.event_type == "context.snapshot.created.v1")
        .expect("snapshot created event");
    assert_eq!(created.actor.actor_type, ActorType::Model);
    assert_eq!(created.actor.id, "run:run-context");
    let event_types = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"approval.granted.v1".into()));
    assert!(event_types.contains(&"effect.release_requested.v1".into()));
}

#[tokio::test]
async fn user_ask_uses_only_an_explicit_interactive_interface_port() {
    let executor = InteractiveToolExecutor {
        prompts: Arc::new(FixedUserPrompt),
        inner: Arc::new(UnusedToolExecutor),
    };
    let result = executor
        .execute(
            ToolCall {
                call_id: "ask".into(),
                name: "user.ask".into(),
                arguments: json!({
                    "question": "Choose a runtime",
                    "choices": ["Rust", "Python"],
                    "allow_free_form": false,
                }),
            },
            ExecutionContext::default(),
        )
        .await
        .expect("user answer");
    let answer: Value = serde_json::from_str(&result.output).expect("answer JSON");
    assert_eq!(answer["answer"], "Rust");
    assert_eq!(answer["selected_index"], 0);
}

#[tokio::test]
async fn tool_search_returns_only_ranked_active_catalog_entries() {
    let registry: Arc<dyn colossus_ports::ToolRegistry> = Arc::new(
        colossus_tools::StaticToolRegistry::builtins(&[
            "tool.search".into(),
            "repo.map".into(),
            "repo.symbol_search".into(),
            "repo.references".into(),
            "repo.file_summary".into(),
            "echo".into(),
        ])
        .expect("catalog"),
    );
    let executor = DiscoverableToolExecutor {
        registry,
        inner: Arc::new(UnusedToolExecutor),
    };
    let result = executor
        .execute(
            ToolCall {
                call_id: "search".into(),
                name: "tool.search".into(),
                arguments: json!({"query": "repository", "max_results": 2}),
            },
            ExecutionContext {
                offered_tools: vec![
                    "tool.search".into(),
                    "repo.map".into(),
                    "repo.symbol_search".into(),
                    "repo.references".into(),
                    "repo.file_summary".into(),
                    "echo".into(),
                ],
                ..ExecutionContext::default()
            },
        )
        .await
        .expect("tool search");
    let output: Value = serde_json::from_str(&result.output).expect("search JSON");
    assert_eq!(output["tools"].as_array().map(Vec::len), Some(2));
    assert_eq!(output["truncated"], true);
    assert!(output["tools"].as_array().is_some_and(|tools| {
        tools.iter().all(|tool| {
            tool["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("repo."))
        })
    }));
}

#[tokio::test]
async fn tool_search_never_discovers_tools_outside_the_model_visible_ceiling() {
    let registry: Arc<dyn colossus_ports::ToolRegistry> = Arc::new(
        colossus_tools::StaticToolRegistry::builtins(&[
            "tool.search".into(),
            "echo".into(),
            "agent.delegate".into(),
        ])
        .expect("catalog"),
    );
    let executor = DiscoverableToolExecutor {
        registry,
        inner: Arc::new(UnusedToolExecutor),
    };
    let result = executor
        .execute(
            ToolCall {
                call_id: "search".into(),
                name: "tool.search".into(),
                arguments: json!({"query": "agent", "max_results": 10}),
            },
            ExecutionContext {
                offered_tools: vec!["tool.search".into(), "echo".into()],
                ..ExecutionContext::default()
            },
        )
        .await
        .expect("tool search");
    let output: Value = serde_json::from_str(&result.output).expect("search JSON");
    assert_eq!(output["tools"], json!([]));
    assert_eq!(output["truncated"], false);
}

#[tokio::test]
async fn repository_context_tools_are_permit_bound_bounded_and_workspace_confined() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("src")).expect("src");
    fs::create_dir_all(workspace.path().join(".colossus")).expect("control state");
    fs::write(
        workspace.path().join("src/lib.rs"),
        "pub struct Widget {}\nfn use_widget(value: Widget) {}\nstruct WidgetFactory {}\n",
    )
    .expect("source");
    fs::write(workspace.path().join("README.md"), "# Example\n").expect("readme");
    fs::write(
        workspace.path().join(".colossus/secret"),
        "must stay hidden",
    )
    .expect("control state");
    fs::write(workspace.path().join("binary.bin"), b"a\0b").expect("binary");
    let workspace_path = fs::canonicalize(workspace.path()).expect("canonical workspace");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let actions = [
        "repo.map",
        "repo.symbol_search",
        "repo.references",
        "repo.file_summary",
    ];
    let mut policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_post_effect(true)
        .with_filesystem_read_root(workspace_path.display().to_string());
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(actions.map(str::to_owned)),
        [44_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace_path,
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };
    let invoke = |name: &str, arguments: Value| ToolCall {
        call_id: format!("call-{name}"),
        name: name.into(),
        arguments,
    };

    let mapped = executor
        .execute(
            invoke("repo.map", json!({"path": ".", "max_files": 10})),
            ExecutionContext::default(),
        )
        .await
        .expect("repository map");
    let mapped: Value = serde_json::from_str(&mapped.output).expect("map JSON");
    let mapped_paths = mapped["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|file| file["path"].as_str())
        .map(|path| path.replace('\\', "/"))
        .collect::<Vec<_>>();
    assert!(mapped_paths.iter().any(|path| path == "src/lib.rs"));
    assert!(!mapped_paths.iter().any(|path| path.contains(".colossus")));

    let symbols = executor
        .execute(
            invoke(
                "repo.symbol_search",
                json!({"pattern": "Widget", "max_results": 10}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("symbol search");
    let symbols: Value = serde_json::from_str(&symbols.output).expect("symbols JSON");
    assert_eq!(symbols["match_count"], 3);

    let references = executor
        .execute(
            invoke(
                "repo.references",
                json!({"symbol": "Widget", "max_results": 10}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("references");
    let references: Value = serde_json::from_str(&references.output).expect("references JSON");
    assert_eq!(references["match_count"], 2);

    let summary = executor
        .execute(
            invoke(
                "repo.file_summary",
                json!({"path": "src/lib.rs", "max_lines": 2}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("file summary");
    let summary: Value = serde_json::from_str(&summary.output).expect("summary JSON");
    assert_eq!(summary["line_count"], 3);
    assert_eq!(summary["preview_truncated"], true);
    assert!(
        summary["symbols"]
            .as_array()
            .is_some_and(|items| items.len() == 3)
    );

    fs::write(
        workspace.path().join("src/generated.rs"),
        format!(
            "const GENERATED: &str = \"{}\";\n",
            "\\\"".repeat(256 * 1024)
        ),
    )
    .expect("generated source");
    let bounded_summary = executor
        .execute(
            invoke(
                "repo.file_summary",
                json!({"path": "src/generated.rs", "max_lines": 500}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("bounded file summary");
    assert!(
        bounded_summary.output.len() <= MAX_FILE_SUMMARY_OUTPUT_BYTES,
        "file summary output was {} bytes",
        bounded_summary.output.len()
    );
    let bounded_summary: Value =
        serde_json::from_str(&bounded_summary.output).expect("bounded summary JSON");
    assert_eq!(bounded_summary["line_count"], 1);
    assert_eq!(bounded_summary["preview_truncated"], true);
    assert_eq!(bounded_summary["path_truncated"], false);

    let absolute_summary = executor
        .execute(
            invoke(
                "repo.file_summary",
                json!({
                    "path": executor.workspace.join("src/lib.rs").to_string_lossy(),
                    "max_lines": 2,
                }),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("absolute in-workspace file summary");
    let absolute_summary: Value =
        serde_json::from_str(&absolute_summary.output).expect("absolute summary JSON");
    assert_eq!(
        absolute_summary["path"]
            .as_str()
            .expect("summary path")
            .replace('\\', "/"),
        "src/lib.rs"
    );
    assert_eq!(absolute_summary["line_count"], 3);

    let absolute_map = executor
        .execute(
            invoke(
                "repo.map",
                json!({"path": executor.workspace.to_string_lossy(), "max_files": 10}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("absolute in-workspace map");
    let absolute_map: Value =
        serde_json::from_str(&absolute_map.output).expect("absolute map JSON");
    assert!(
        absolute_map["files"]
            .as_array()
            .expect("files")
            .iter()
            .filter_map(|file| file["path"].as_str())
            .map(|path| path.replace('\\', "/"))
            .any(|path| path == "src/lib.rs")
    );

    assert!(
        executor
            .execute(
                invoke("repo.file_summary", json!({"path": "../outside"})),
                ExecutionContext::default(),
            )
            .await
            .is_err()
    );
    let event_types = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"effect.requested.v1".into()));
    assert!(event_types.contains(&"effect.release_requested.v1".into()));
}

#[tokio::test]
async fn ambient_structured_filesystem_and_repository_tools_accept_external_control_paths() {
    let workspace = tempdir().expect("workspace");
    let outside = tempdir().expect("outside");
    let control = outside.path().join(".colossus");
    fs::create_dir_all(&control).expect("control directory");
    let source = control.join("ambient.txt");
    fs::write(&source, "ambient access\n").expect("source");
    let destination = outside.path().join("written.txt");
    let actions = [
        "filesystem.read",
        "filesystem.write",
        "repo.map",
        "repo.file_summary",
    ];
    let mut policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_sandbox("danger_full_access", "test", false)
        .with_resource_authority(ResourceAuthority::Ambient);
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(actions.map(str::to_owned)).with_sandbox_boundary_gate(
            Arc::new(colossus_policy::SandboxBoundaryGate::new(
                Some(SandboxBoundaryMode::DangerFullAccess),
                true,
            )),
        ),
        [45_u8; 32],
    ));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: None,
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: fs::canonicalize(workspace.path()).expect("canonical workspace"),
        repository_id: "ambient-repo-test".into(),
        executables: Vec::new(),
    };
    let invoke = |name: &str, arguments: Value| ToolCall {
        call_id: format!("call-{name}"),
        name: name.into(),
        arguments,
    };

    let read = executor
        .execute(
            invoke("filesystem.read", json!({"path": source})),
            ExecutionContext::default(),
        )
        .await
        .expect("external control-state read");
    assert_eq!(read.output, "ambient access\n");

    executor
        .execute(
            invoke(
                "filesystem.write",
                json!({"path": destination, "content": "written", "mode": "create"}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("external write");
    assert_eq!(
        fs::read_to_string(&destination).expect("written file"),
        "written"
    );

    let summary = executor
        .execute(
            invoke(
                "repo.file_summary",
                json!({"path": source, "max_lines": 10}),
            ),
            ExecutionContext::default(),
        )
        .await
        .expect("external .colossus repository summary");
    let summary: Value = serde_json::from_str(&summary.output).expect("summary JSON");
    assert_eq!(
        summary["path"],
        fs::canonicalize(&source)
            .expect("canonical source")
            .display()
            .to_string()
    );

    let mapped = executor
        .execute(
            invoke("repo.map", json!({"path": outside.path(), "max_files": 10})),
            ExecutionContext::default(),
        )
        .await
        .expect("ambient repository map");
    let mapped: Value = serde_json::from_str(&mapped.output).expect("map JSON");
    assert!(
        mapped["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|file| file["path"]
                == fs::canonicalize(&source)
                    .expect("canonical mapped source")
                    .display()
                    .to_string()))
    );
}

struct FakeProcessExecutor {
    actions: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl colossus_policy::EffectExecutor for FakeProcessExecutor {
    async fn execute(
        &self,
        request: &colossus_contracts::EffectRequest,
        _permit: colossus_policy::ExecutionPermit,
    ) -> Result<colossus_contracts::QuarantinedEffectResult, colossus_policy::ExecutionError> {
        self.actions
            .lock()
            .expect("actions")
            .push(request.action.clone());
        let (exit_code, stdout, stderr) = if request.action == "shell.run" {
            (7, "", "command failed")
        } else {
            (0, " M note.txt\n", "")
        };
        Ok(colossus_contracts::QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({
                "backend": "test",
                "exit_code": exit_code,
                "success": exit_code == 0,
                "timed_out": false,
                "resource_limit_exceeded": null,
                "output_truncated": false,
                "stdout_base64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    stdout,
                ),
                "stderr_base64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    stderr,
                ),
            }))
            .expect("result JSON"),
            effect_succeeded: true,
        })
    }
}

struct RecordingProcessExecutor {
    request: Arc<Mutex<Option<colossus_contracts::EffectRequest>>>,
}

#[async_trait::async_trait]
impl colossus_policy::EffectExecutor for RecordingProcessExecutor {
    async fn execute(
        &self,
        request: &colossus_contracts::EffectRequest,
        _permit: colossus_policy::ExecutionPermit,
    ) -> Result<colossus_contracts::QuarantinedEffectResult, colossus_policy::ExecutionError> {
        *self.request.lock().expect("request") = Some(request.clone());
        Ok(colossus_contracts::QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({
                "backend": "danger_full_access",
                "exit_code": 0,
                "success": true,
                "timed_out": false,
                "resource_limit_exceeded": null,
                "output_truncated": false,
                "stdout_base64": "",
                "stderr_base64": "",
            }))
            .expect("result JSON"),
            effect_succeeded: true,
        })
    }
}

#[tokio::test]
async fn danger_full_access_shell_needs_no_process_resource_configuration() {
    let workspace = tempdir().expect("workspace");
    let outside_cwd = tempdir().expect("outside cwd");
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("shell.run", DecisionOutcome::Allow)
        .with_sandbox("danger_full_access", "test", false)
        .with_resource_authority(ResourceAuthority::Ambient);
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["shell.run".into()]).with_sandbox_boundary_gate(
            Arc::new(colossus_policy::SandboxBoundaryGate::new(
                Some(SandboxBoundaryMode::DangerFullAccess),
                true,
            )),
        ),
        [9_u8; 32],
    ));
    let recorded = Arc::new(Mutex::new(None));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: Some(Arc::new(RecordingProcessExecutor {
            request: Arc::clone(&recorded),
        })),
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };

    let result = executor
        .execute(
            ToolCall {
                call_id: "danger-shell".into(),
                name: "shell.run".into(),
                arguments: json!({
                    "command": "echo unrestricted",
                    "cwd": outside_cwd.path(),
                    "env": {"PATH": "/operator/path", "UNDECLARED_ENVIRONMENT": "available"},
                }),
            },
            ExecutionContext::default(),
        )
        .await
        .expect("danger full access shell");
    let output: Value = serde_json::from_str(&result.output).expect("tool output");
    assert_eq!(
        output["cwd"],
        outside_cwd
            .path()
            .canonicalize()
            .expect("canonical cwd")
            .display()
            .to_string()
    );
    let request = recorded
        .lock()
        .expect("request")
        .clone()
        .expect("recorded request");
    assert_eq!(request.content["environment"]["PATH"], "/operator/path");
    assert_eq!(
        request.content["environment"]["UNDECLARED_ENVIRONMENT"],
        "available"
    );
    assert!(Path::new(&request.resource).is_absolute());
}

#[tokio::test]
async fn danger_full_access_withholds_host_resolution_until_acknowledgement() {
    let workspace = tempdir().expect("workspace");
    let outside_cwd = tempdir().expect("outside cwd");
    let candidate = outside_cwd.path().join("candidate");
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("shell.run", DecisionOutcome::Allow)
        .with_sandbox("danger_full_access", "test", false)
        .with_resource_authority(ResourceAuthority::Ambient);
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["shell.run".into()]).with_sandbox_boundary_gate(
            Arc::new(colossus_policy::SandboxBoundaryGate::new(
                Some(SandboxBoundaryMode::DangerFullAccess),
                false,
            )),
        ),
        [9_u8; 32],
    ));
    let recorded = Arc::new(Mutex::new(None));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: Some(Arc::new(RecordingProcessExecutor {
            request: Arc::clone(&recorded),
        })),
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: Vec::new(),
    };

    let probe = async || -> String {
        executor
            .execute(
                ToolCall {
                    call_id: "unacknowledged-shell".into(),
                    name: "shell.run".into(),
                    arguments: json!({
                        "argv": [candidate.display().to_string(), "--version"],
                        "cwd": outside_cwd.path(),
                    }),
                },
                ExecutionContext {
                    session_id: Some("session-unacknowledged".into()),
                    ..ExecutionContext::default()
                },
            )
            .await
            .expect_err("unacknowledged danger full access shell")
            .to_string()
    };

    let absent = probe().await;
    fs::write(&candidate, "test executable identity").expect("executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))
            .expect("executable permissions");
    }
    let present = probe().await;
    assert_eq!(
        absent, present,
        "unacknowledged resolution must not disclose host existence"
    );
    assert!(absent.contains("not explicitly configured"), "{absent}");
    assert!(
        recorded.lock().expect("request").is_none(),
        "unacknowledged shell must not reach the process executor"
    );
}

#[tokio::test]
async fn git_and_shell_tools_keep_distinct_policy_and_nonzero_exit_semantics() {
    let workspace = tempdir().expect("workspace");
    let executable = workspace.path().join("git");
    fs::write(&executable, "test executable identity").expect("executable");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = colossus_policy::BuiltInPolicy::offline_default()
        .with_action("git.status", DecisionOutcome::Allow)
        .with_action("shell.run", DecisionOutcome::RequireApproval)
        .with_sandbox("native", "test", false)
        .with_filesystem_root(workspace.path().display().to_string(), "read")
        .with_filesystem_root(executable.display().to_string(), "execute");
    let gateway = Arc::new(colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        colossus_policy::SafetyKernel::new(["git.status".into(), "shell.run".into()]),
        [9_u8; 32],
    ));
    let actions = Arc::new(Mutex::new(Vec::new()));
    let executor = GatewayToolExecutor {
        gateway,
        filesystem: Arc::new(colossus_sandbox::FilesystemExecutor::new()),
        process: Some(Arc::new(FakeProcessExecutor {
            actions: Arc::clone(&actions),
        })),
        http: Arc::new(colossus_sandbox::HttpExecutor::new()),
        work: None,
        memory: None,
        skills: None,
        pack_processes: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.path().to_path_buf(),
        repository_id: "repo-test".into(),
        executables: vec![executable],
    };
    let status = executor
        .execute(
            ToolCall {
                call_id: "git-status".into(),
                name: "git.status".into(),
                arguments: json!({}),
            },
            ExecutionContext::default(),
        )
        .await
        .expect("git status");
    let status: serde_json::Value = serde_json::from_str(&status.output).expect("status JSON");
    assert_eq!(status["entries"][0]["status"], " M");
    assert_eq!(status["entries"][0]["path"], "note.txt");

    let shell = executor
        .execute(
            ToolCall {
                call_id: "shell".into(),
                name: "shell.run".into(),
                arguments: json!({"argv": ["git", "bad-command"]}),
            },
            ExecutionContext::default(),
        )
        .await
        .expect("known nonzero outcome");
    assert_eq!(shell.exit_code, 7);
    let shell: serde_json::Value = serde_json::from_str(&shell.output).expect("shell JSON");
    assert_eq!(shell["exit_code"], 7);
    assert_eq!(shell["stderr"], "command failed");
    assert_eq!(
        actions.lock().expect("actions").as_slice(),
        ["git.status", "shell.run"]
    );

    for (name, arguments) in [
        ("git.diff", json!({"paths": ["../outside"]})),
        ("git.show", json!({"rev": "--exec-path=/tmp"})),
        ("shell.run", json!({"argv": ["sh", "-c", "id"]})),
    ] {
        assert!(
            executor
                .execute(
                    ToolCall {
                        call_id: format!("denied-{name}"),
                        name: name.into(),
                        arguments,
                    },
                    ExecutionContext::default(),
                )
                .await
                .is_err()
        );
    }
    assert_eq!(actions.lock().expect("actions").len(), 2);
    let names = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(names.contains(&"approval.granted.v1".into()));
    assert!(names.contains(&"effect.release_requested.v1".into()));
}
