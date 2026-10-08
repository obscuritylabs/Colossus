//! Complete domain fixtures for persistence conformance.
use super::*;
use colossus_sdk::{CreateRunRequest, IdempotencyKey, InputContentPart, RunMode};

pub(super) fn value(kind: EntityKind, id: &str, overrides: serde_json::Value) -> EntityValue {
    let project = "acceptance-project";
    let time = "2026-01-01T00:00:00Z";
    let mut value = match kind {
        EntityKind::Project => json!({
            "id":id,"name":overrides.get("label").and_then(serde_json::Value::as_str).unwrap_or(id),
            "description":"","parent_project_id":null,"archived":false,"revision":1,
            "created_at":time,"updated_at":time
        }),
        EntityKind::Node => json!({
            "node_id":id,"project_id":project,"instance_id":"local-instance","label":id,
            "certificate_sha256":"a".repeat(64),"roles":["primary"],"revoked":false,"revision":1
        }),
        EntityKind::Host => json!({
            "host_id":id,"project_id":project,"label":id,"platform":"linux",
            "deployment_kind":"standalone","last_seen_at":0,"revision":1
        }),
        EntityKind::Workspace => json!({
            "workspace_id":id,"project_id":project,"host_id":"host","node_id":"runtime",
            "label":id,"sharing":"private","revision":1
        }),
        EntityKind::Thread => json!({
            "thread_id":id,"project_id":project,"node_id":"runtime","host_id":null,"workspace_id":null,
            "title":id,"created_at":time,"updated_at":time,"revision":1,"archived":false,"session_id":null,
            "sync_status":"current","source":"cloud","can_continue":true,"active_task_id":null,
            "queued_task_ids":[]
        }),
        EntityKind::ThreadMessage => json!({
            "message_id":id,"thread_id":"thread","project_id":project,"role":"assistant",
            "text":id,"created_at":time,"task_id":"message-task","revision":1
        }),
        EntityKind::Task => {
            let request = CreateRunRequest {
                plugin_skill_ids: Vec::new(),
                input: vec![InputContentPart::Text("fixture".into())],
                session_id: None,
                end_user_id: None,
                role: "primary".into(),
                mode: RunMode::Execute,
                research_depth: None,
                research_sources: Vec::new(),
                plan_action: None,
                branch: None,
                max_turns: 1,
                idempotency_key: IdempotencyKey::new(id).unwrap(),
            };
            json!({
                "task_id":id,"project_id":project,"node_id":"runtime","subject":"human",
                "created_at":time,"updated_at":time,"request":request,"thread_id":null,
                "source_read_only":false,"history_complete":true,"history_bounded":false,
                "run_id":null,"snapshot":null,"dispatch_error":null,"last_sequence":0,
                "released_bytes":0,"output_limited":false,"revision":1
            })
        }
        EntityKind::AuthFlow => return EntityValue::AuthFlow(overrides),
        _ => panic!("fixture requires an explicit typed value for this entity"),
    };
    value
        .as_object_mut()
        .unwrap()
        .extend(overrides.as_object().unwrap().clone());
    crate::normalized::decode(kind, id, value).unwrap()
}
