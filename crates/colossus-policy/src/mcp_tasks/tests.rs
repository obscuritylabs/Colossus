use super::*;
use colossus_testkit::InMemoryEventJournal;
use std::sync::{Mutex, atomic::AtomicUsize};

const ORIGIN: &str = "https://mcp.example.test";
const ENDPOINT: &str = "https://mcp.example.test/api/mcp";
const BINDING: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn task_request(action: &str) -> EffectRequest {
    let step = match action {
        "mcp.call" => json!({"kind": "start"}),
        "mcp.tasks.get" => json!({"kind": "get", "binding": BINDING}),
        "mcp.tasks.cancel" => json!({"kind": "cancel", "binding": BINDING}),
        _ => panic!("unknown test MCP action"),
    };
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::Model,
            id: "tool-call:fixture".into(),
        },
        action,
        ENDPOINT,
        json!({
            "operation": {
                "kind": "call_tool",
                "server": "fixture",
                "tool": "lookup",
                "arguments": {},
                "input_schema": {"type": "object"}
            },
            "transport": "streamable_http",
            "url": ENDPOINT,
            "protocol_version": "2026-07-28",
            "provenance": null,
            "task_step": step
        }),
    );
    request.capabilities.push("mcp.invoke".into());
    request.context.run_id = Some("run-fixture".into());
    request.context.session_id = Some("session-fixture".into());
    request
}

fn kernel() -> SafetyKernel {
    SafetyKernel::new(["mcp.invoke".into()])
}

#[test]
fn task_envelope_rejects_legacy_foreign_and_unscoped_operations() {
    for action in ["mcp.call", "mcp.tasks.get", "mcp.tasks.cancel"] {
        let request = task_request(action);
        kernel().prepare(&request).expect("valid task effect");
        for (path, value) in [
            ("/task_step", Value::Null),
            ("/task_step/kind", json!("unrecognized")),
            ("/transport", json!("stdio")),
            ("/protocol_version", json!("2025-11-25")),
            ("/url", json!("https://mcp.example.test/different-path")),
            ("/provenance", json!({"owner": "plugin"})),
            ("/operation/kind", json!("list_tools")),
        ] {
            let mut invalid = request.clone();
            *invalid.content.pointer_mut(path).expect("fixture field") = value;
            assert!(
                matches!(kernel().prepare(&invalid), Err(GatewayError::Safety(_))),
                "{action} accepted invalid {path}"
            );
        }
        let mut missing_run = request.clone();
        missing_run.context.run_id = None;
        let mut missing_session = request.clone();
        missing_session.context.session_id = None;
        let mut user = request.clone();
        user.actor.actor_type = ActorType::User;
        let mut missing_capability = request.clone();
        missing_capability.capabilities.clear();
        for invalid in [missing_run, missing_session, user, missing_capability] {
            assert!(matches!(
                kernel().prepare(&invalid),
                Err(GatewayError::Safety(_))
            ));
        }
        let mut wire_handle = request;
        wire_handle.content["task_step"]["taskId"] = json!("remote-task-secret");
        assert!(matches!(
            kernel().prepare(&wire_handle),
            Err(GatewayError::Safety(_))
        ));
    }
}

#[test]
fn follow_up_accepts_only_opaque_lowercase_hex_bindings() {
    let request = task_request("mcp.tasks.get");
    for invalid_binding in [
        Value::Null,
        json!("a".repeat(63)),
        json!("a".repeat(65)),
        json!("A".repeat(64)),
        json!("g".repeat(64)),
        json!({"taskId": "raw-wire-task"}),
    ] {
        let mut invalid = request.clone();
        invalid.content["task_step"]["binding"] = invalid_binding;
        assert!(matches!(
            kernel().prepare(&invalid),
            Err(GatewayError::Safety(_))
        ));
    }
}

#[derive(Default)]
struct PolicyFixture {
    deny: Option<(&'static str, EffectPhase)>,
    approval_action: Option<&'static str>,
    obligation_override: Option<(&'static str, EffectPhase, PolicyObligations)>,
    require_post_effect: bool,
    seen: Mutex<Vec<EffectRequest>>,
}

fn task_obligations(require_post_effect: bool) -> PolicyObligations {
    let mut obligations = default_obligations();
    obligations.require_post_effect = require_post_effect;
    obligations.network_destinations.push(ORIGIN.into());
    obligations
}

#[async_trait]
impl PolicyDecisionPoint for PolicyFixture {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        self.seen.lock().unwrap().push(request.clone());
        let obligations = self
            .obligation_override
            .as_ref()
            .filter(|(action, phase, _)| {
                *action == request.action.as_str() && *phase == request.phase
            })
            .map_or_else(
                || task_obligations(self.require_post_effect),
                |(_, _, obligations)| obligations.clone(),
            );
        let outcome = if self.deny == Some((request.action.as_str(), request.phase)) {
            DecisionOutcome::Deny
        } else if self.approval_action == Some(request.action.as_str())
            && request.phase == EffectPhase::PreEffect
            && request.approval.is_none()
        {
            DecisionOutcome::RequireApproval
        } else {
            DecisionOutcome::Allow
        };
        Ok(PolicyDecision {
            decision_id: Uuid::now_v7().to_string(),
            policy_revision: "mcp-tasks-fixture".into(),
            outcome,
            reason: "fixture decision".into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        Ok(json!({"ready": true}))
    }
}

#[derive(Default)]
struct Executor(AtomicUsize);

#[async_trait]
impl EffectExecutor for Executor {
    async fn execute(
        &self,
        _: &EffectRequest,
        _: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Ok(QuarantinedEffectResult {
            effect_succeeded: true,
            media_type: "application/json".into(),
            bytes: br#"{"status":"working"}"#.to_vec(),
        })
    }
}

fn gateway(policy: Arc<dyn PolicyDecisionPoint>) -> (EffectGateway, Arc<InMemoryEventJournal>) {
    let journal = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        journal.clone(),
        policy,
        Arc::new(AllowApproval {
            approved_by: "fixture".into(),
        }),
        kernel(),
        [2; 32],
    );
    (gateway, journal)
}

#[tokio::test]
async fn external_policy_cannot_skip_release_for_any_task_step() {
    let (gateway, _) = gateway(Arc::new(PolicyFixture::default()));
    let executor = Executor::default();
    for action in ["mcp.call", "mcp.tasks.get", "mcp.tasks.cancel"] {
        assert!(matches!(
            gateway.execute(task_request(action), &executor).await,
            Err(GatewayError::Safety(message)) if message.contains("mandatory post-effect")
        ));
    }
    assert_eq!(executor.0.load(Ordering::Acquire), 0);

    let mut ordinary_call = task_request("mcp.call");
    ordinary_call
        .content
        .as_object_mut()
        .unwrap()
        .remove("task_step");
    gateway
        .execute(ordinary_call, &executor)
        .await
        .expect("ordinary MCP keeps its existing policy contract");
    assert_eq!(executor.0.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn follow_up_requires_the_current_network_origin() {
    let (gateway, _) = gateway(Arc::new(PolicyFixture {
        require_post_effect: true,
        ..PolicyFixture::default()
    }));
    let executor = Executor::default();
    for action in ["mcp.tasks.get", "mcp.tasks.cancel"] {
        let mut request = task_request(action);
        request.resource = "https://outside.example.test/api/mcp".into();
        request.content["url"] = json!(request.resource);
        assert!(matches!(
            gateway.execute(request, &executor).await,
            Err(GatewayError::Safety(message)) if message.contains("network destination")
        ));
    }
    assert_eq!(executor.0.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn built_in_mcp_invoke_denial_vetoes_both_follow_ups() {
    let policy = BuiltInPolicy::offline_default()
        .with_network_destination(ORIGIN)
        .with_action("mcp.call", DecisionOutcome::Allow)
        .with_action("mcp.tasks.get", DecisionOutcome::Allow)
        .with_action("mcp.tasks.cancel", DecisionOutcome::Allow)
        .with_action("mcp.invoke", DecisionOutcome::Deny);
    let (gateway, _) = gateway(Arc::new(policy));
    let executor = Executor::default();
    for action in ["mcp.tasks.get", "mcp.tasks.cancel"] {
        assert!(matches!(
            gateway.execute(task_request(action), &executor).await,
            Err(GatewayError::Denied(_))
        ));
    }
    assert_eq!(executor.0.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn current_original_call_denial_vetoes_follow_up_before_permit() {
    let policy = Arc::new(PolicyFixture {
        deny: Some(("mcp.call", EffectPhase::PreEffect)),
        approval_action: Some("mcp.tasks.cancel"),
        require_post_effect: true,
        ..PolicyFixture::default()
    });
    let (gateway, journal) = gateway(policy.clone());
    let executor = Executor::default();
    let request = task_request("mcp.tasks.cancel");
    let stream = format!("effect:{}", request.request_id);
    assert!(matches!(
        gateway.execute(request.clone(), &executor).await,
        Err(GatewayError::Denied(message)) if message.contains("originating MCP call")
    ));
    assert_eq!(executor.0.load(Ordering::Acquire), 0);
    let seen = policy.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].action, "mcp.tasks.cancel");
    assert_eq!(seen[0].content["task_step"]["binding"], BINDING);
    assert!(seen[0].approval.is_none());
    assert_eq!(seen[1].action, "mcp.tasks.cancel");
    assert!(seen[1].approval.is_some());
    assert_eq!(seen[2].action, "mcp.call");
    assert_eq!(seen[2].content["task_step"], json!({"kind": "start"}));
    assert_eq!(seen[2].actor, request.actor);
    assert_eq!(seen[2].context, request.context);
    assert_ne!(seen[2].request_id, request.request_id);
    assert!(seen[2].approval.is_none());
    let events = journal.read_stream(&stream).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.denied.v1")
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "effect.started.v1")
    );
}

#[tokio::test]
async fn both_release_decisions_must_allow_the_follow_up_bytes() {
    for deny in [Some("mcp.call"), Some("mcp.tasks.get"), None] {
        let policy = Arc::new(PolicyFixture {
            deny: deny.map(|action| (action, EffectPhase::PostEffect)),
            require_post_effect: true,
            ..PolicyFixture::default()
        });
        let (gateway, journal) = gateway(policy.clone());
        let executor = Executor::default();
        let request = task_request("mcp.tasks.get");
        let post_stream = format!("effect:{}:post", request.request_id);
        let result = gateway.execute(request, &executor).await;
        assert_eq!(executor.0.load(Ordering::Acquire), 1);
        let seen = policy.seen.lock().unwrap();
        assert_eq!(seen[2].phase, EffectPhase::PostEffect);
        assert_eq!(seen[2].action, "mcp.tasks.get");
        assert_eq!(seen[2].content["task_step"]["binding"], BINDING);
        if deny != Some("mcp.tasks.get") {
            assert_eq!(seen.len(), 4);
            assert_eq!(seen[3].phase, EffectPhase::PostEffect);
            assert_eq!(seen[3].action, "mcp.call");
            assert_eq!(seen[3].content["task_step"], json!({"kind": "start"}));
            assert_eq!(
                seen[3].content["content_base64"],
                seen[2].content["content_base64"]
            );
            assert!(seen[3].approval.is_none());
        } else {
            assert_eq!(seen.len(), 3);
        }
        let events = journal.read_stream(&post_stream).unwrap();
        if deny.is_some() {
            assert!(matches!(result, Err(GatewayError::Denied(_))));
            assert!(
                events
                    .iter()
                    .any(|event| event.event_type == "effect.release_denied.v1")
            );
        } else {
            assert_eq!(
                result.expect("both policies permitted release").bytes,
                br#"{"status":"working"}"#
            );
            assert!(
                !events
                    .iter()
                    .any(|event| event.event_type == "effect.release_denied.v1")
            );
        }
    }
}

#[tokio::test]
async fn live_parent_ceiling_vetoes_a_broader_follow_up_permit() {
    let default = task_obligations(true);
    for (action, obligations) in [
        (
            "mcp.call",
            PolicyObligations {
                max_output_bytes: 128,
                ..default.clone()
            },
        ),
        (
            "mcp.call",
            PolicyObligations {
                timeout_ms: 1,
                ..default.clone()
            },
        ),
        (
            "mcp.call",
            PolicyObligations {
                required_redactions: vec!["private".into()],
                ..default.clone()
            },
        ),
        (
            "mcp.tasks.get",
            PolicyObligations {
                allowed_environment: vec!["EXTRA".into()],
                ..default.clone()
            },
        ),
        (
            "mcp.tasks.get",
            PolicyObligations {
                network_destinations: vec![ORIGIN.into(), "https://additional.example.test".into()],
                ..default.clone()
            },
        ),
    ] {
        let (gateway, _) = gateway(Arc::new(PolicyFixture {
            require_post_effect: true,
            obligation_override: Some((action, EffectPhase::PreEffect, obligations)),
            ..PolicyFixture::default()
        }));
        let executor = Executor::default();
        assert!(matches!(
            gateway.execute(task_request("mcp.tasks.get"), &executor).await,
            Err(GatewayError::Denied(message)) if message.contains("exceeds current originating call obligations")
        ));
        assert_eq!(executor.0.load(Ordering::Acquire), 0);
    }
}

#[tokio::test]
async fn live_parent_ceiling_accepts_a_narrower_follow_up_permit() {
    let (gateway, _) = gateway(Arc::new(PolicyFixture {
        require_post_effect: true,
        obligation_override: Some((
            "mcp.tasks.get",
            EffectPhase::PreEffect,
            PolicyObligations {
                timeout_ms: 1_000,
                max_output_bytes: 128,
                ..task_obligations(true)
            },
        )),
        ..PolicyFixture::default()
    }));
    let executor = Executor::default();
    gateway
        .execute(task_request("mcp.tasks.get"), &executor)
        .await
        .expect("narrower follow-up permit");
    assert_eq!(executor.0.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn original_task_release_does_not_require_a_process_or_egress_grant() {
    let (gateway, _) = gateway(Arc::new(PolicyFixture {
        require_post_effect: true,
        obligation_override: Some((
            "mcp.call",
            EffectPhase::PostEffect,
            PolicyObligations {
                network_destinations: Vec::new(),
                ..task_obligations(true)
            },
        )),
        ..PolicyFixture::default()
    }));
    let executor = Executor::default();
    for action in ["mcp.call", "mcp.tasks.get"] {
        gateway
            .execute(task_request(action), &executor)
            .await
            .expect("valid release needs no execution grants");
    }
    assert_eq!(executor.0.load(Ordering::Acquire), 2);
}

#[tokio::test]
async fn both_current_release_policies_enforce_size_and_redactions() {
    for action in ["mcp.call", "mcp.tasks.get"] {
        for (obligations, expected_reason) in [
            (
                PolicyObligations {
                    max_output_bytes: 1,
                    ..task_obligations(true)
                },
                "current release policy limit",
            ),
            (
                PolicyObligations {
                    required_redactions: vec!["private".into()],
                    ..task_obligations(true)
                },
                "cannot satisfy policy-specified output redactions",
            ),
        ] {
            let (gateway, journal) = gateway(Arc::new(PolicyFixture {
                require_post_effect: true,
                obligation_override: Some((action, EffectPhase::PostEffect, obligations)),
                ..PolicyFixture::default()
            }));
            let executor = Executor::default();
            let request = task_request("mcp.tasks.get");
            let post_stream = format!("effect:{}:post", request.request_id);
            assert!(matches!(
                gateway.execute(request, &executor).await,
                Err(GatewayError::Denied(message)) if message.contains(expected_reason)
            ));
            assert_eq!(executor.0.load(Ordering::Acquire), 1);
            let events = journal.read_stream(&post_stream).unwrap();
            let event = events
                .iter()
                .find(|event| event.event_type == "effect.release_denied.v1")
                .unwrap();
            assert!(
                !journal
                    .decrypt_payload(event)
                    .unwrap()
                    .to_string()
                    .contains("private")
            );
        }
    }
}
