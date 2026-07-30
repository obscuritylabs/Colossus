use super::{
    Actor, ActorType, AgentRunMode, ExecutionContext, PlanDraftTarget, PlanExecutionStrategy,
    PlanRecord, PolicyDecision, RunEvent, ThemeName, ThemeSpinner,
};

#[test]
fn application_actor_has_stable_journal_provenance() {
    let actor = Actor {
        actor_type: ActorType::Application,
        id: "app:019f7d38-649a-7580-a30f-01157b719c2a".into(),
    };
    assert_eq!(
        serde_json::to_string(&actor).expect("application actor"),
        r#"{"actor_type":"application","id":"app:019f7d38-649a-7580-a30f-01157b719c2a"}"#
    );
}

#[test]
fn policy_decision_rejects_unknown_fields() {
    let document = r#"{
            "decision_id":"d1","policy_revision":"r1","outcome":"deny",
            "reason":"no","obligations":{"sandbox_backend":"none",
            "sandbox_profile":"none","filesystem":[],"network_destinations":[],
            "allowed_environment":[],"allow_sandbox_downgrade":false,
            "timeout_ms":1,"max_output_bytes":1,"max_processes":0,
            "max_memory_bytes":1,"max_concurrency":1,"required_redactions":[],
            "require_post_effect":false,"audit_labels":{},"retention":"standard"},
            "surprise":true
        }"#;
    assert!(serde_json::from_str::<PolicyDecision>(document).is_err());
}

#[test]
fn run_event_rejects_unknown_fields() {
    let document = r#"{
            "type":"phase","phase":"preparing","turn":1,"action":null,
            "elapsed_seconds":0.1,"surprise":true
        }"#;
    assert!(serde_json::from_str::<RunEvent>(document).is_err());
}

#[test]
fn run_error_http_status_is_optional_and_structured() {
    let legacy = r#"{
        "type":"error","code":"provider.failed","message":"failed",
        "recoverable":false,"turn":1,"elapsed_seconds":0.1
    }"#;
    assert!(matches!(
        serde_json::from_str::<RunEvent>(legacy).expect("legacy error event"),
        RunEvent::Error {
            http_status: None,
            ..
        }
    ));

    let event = RunEvent::Error {
        code: "provider.temporarily_unavailable".into(),
        message: "not ready".into(),
        recoverable: true,
        http_status: Some(503),
        retry_after_ms: Some(7_000),
        turn: Some(1),
        elapsed_seconds: 0.1,
    };
    let value = serde_json::to_value(event).expect("structured error event");
    assert_eq!(value["http_status"], 503);
    assert_eq!(value["retry_after_ms"], 7_000);
}

#[test]
fn legacy_plan_records_default_to_revision_zero() {
    let document = serde_json::json!({
        "id": "plan-legacy",
        "session_id": "session-1",
        "prompt": "Preserve compatibility",
        "status": "draft",
        "content": "# Plan",
        "steps": [{
            "index": 1,
            "title": "Verify",
            "detail": "",
            "requires_mutation": false
        }],
        "created_at": "2026-07-29T00:00:00Z",
        "updated_at": "2026-07-29T00:00:00Z",
        "approved_at": null,
        "executed_run_id": null
    });
    let plan: PlanRecord = serde_json::from_value(document).expect("legacy plan");
    assert_eq!(plan.revision, 0);
}

#[test]
fn plan_mode_and_execution_strategy_have_stable_tagged_shapes() {
    let mode = AgentRunMode::Plan(PlanDraftTarget::Update {
        plan_id: "plan-1".into(),
        revision: 7,
    });
    let encoded_mode = serde_json::json!({
        "mode": "plan",
        "target": {
            "operation": "update",
            "plan_id": "plan-1",
            "revision": 7
        }
    });
    assert_eq!(serde_json::to_value(&mode).expect("mode"), encoded_mode);
    assert_eq!(
        serde_json::from_value::<AgentRunMode>(encoded_mode).expect("mode round trip"),
        mode
    );
    assert_eq!(
        serde_json::to_value(PlanExecutionStrategy::Goal { max_iterations: 5 }).expect("strategy"),
        serde_json::json!({"strategy": "goal", "max_iterations": 5})
    );
    assert_eq!(AgentRunMode::default(), AgentRunMode::Execute);
}

#[test]
fn draft_plan_dispatch_target_is_never_serialized() {
    let context = ExecutionContext {
        correlation_id: "run-1".into(),
        draft_plan_id: Some("plan-1".into()),
        draft_plan_revision: Some(7),
        ..ExecutionContext::default()
    };
    let value = serde_json::to_value(&context).expect("context");
    assert!(value.get("draft_plan_id").is_none());
    assert!(value.get("draft_plan_revision").is_none());
    let restored: ExecutionContext = serde_json::from_value(value).expect("restore");
    assert_eq!(restored.draft_plan_id, None);
    assert_eq!(restored.draft_plan_revision, None);
}

#[test]
fn theme_names_are_stable_and_plain_migrates_to_mono() {
    for (theme, name) in [
        (ThemeName::Default, "default"),
        (ThemeName::Mono, "mono"),
        (ThemeName::HighContrast, "high_contrast"),
        (ThemeName::Carrot, "carrot"),
        (ThemeName::Hacker, "hacker"),
    ] {
        assert_eq!(theme.as_str(), name);
        assert_eq!(
            serde_json::to_string(&theme).expect("theme"),
            format!("\"{name}\"")
        );
    }
    assert_eq!(
        serde_json::from_str::<ThemeName>("\"plain\"").expect("legacy plain theme"),
        ThemeName::Mono
    );
    assert_eq!(
        serde_json::to_string(&ThemeSpinner::BouncingBar).expect("spinner"),
        "\"bouncingBar\""
    );
    assert_eq!(
        serde_json::from_str::<ThemeSpinner>("\"bouncing_bar\"").expect("legacy spinner spelling"),
        ThemeSpinner::BouncingBar
    );
}
