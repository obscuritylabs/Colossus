//! Exercise real Hosted dispatch and caller-bound approvals through the HTTP edge.
use super::fixture::App;
use axum::http::{HeaderMap, Method};
use colossus_cloud::CloudTask;
use colossus_sdk::{
    InteractionContent, InteractionStatus, RunStatus, RunTerminal, RunUpdate, RunUpdateKind,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

pub(super) async fn run(
    app: &App,
    headers: &HeaderMap,
    project: &str,
    marker: &str,
    key: &str,
) -> CloudTask {
    Box::pin(run_inner(app, headers, project, marker, key)).await
}

async fn run_inner(
    app: &App,
    headers: &HeaderMap,
    project: &str,
    marker: &str,
    key: &str,
) -> CloudTask {
    let key = key.replace(['[', ']'], "");
    let created = app
        .ok(
            headers,
            Method::POST,
            &format!("/api/projects/{project}/chat/threads"),
            json!({"request":{
                "model_profile":null,"reasoning_effort":null,"plugin_skill_ids":[],
                "input":[{"text":marker}],"session_id":null,"end_user_id":null,
                "role":"primary","mode":"execute","goal_max_iterations":0,
                "research_depth":null,"research_sources":[],"plan_action":null,"branch":null,
                "max_turns":8,"idempotency_key":key
            }}),
        )
        .await;
    let mut task: CloudTask = serde_json::from_value(created["task"].clone()).unwrap();
    let path = format!("/api/projects/{project}/tasks/{}", task.task_id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut answered = BTreeSet::new();
    loop {
        if task.dispatch_error.is_some() {
            return task;
        }
        if let Some(snapshot) = &task.snapshot {
            if matches!(
                snapshot.run.status,
                RunStatus::Completed
                    | RunStatus::Failed
                    | RunStatus::Cancelled
                    | RunStatus::Interrupted
                    | RunStatus::OutcomeUnknown
            ) && (task.output_limited || task.last_sequence >= snapshot.run.last_sequence)
            {
                return task;
            }
            for interaction in &snapshot.pending_interactions {
                if interaction.status != InteractionStatus::Pending
                    || !answered.insert(interaction.interaction_id.clone())
                {
                    continue;
                }
                let InteractionContent::Approval(approval) = &interaction.content else {
                    panic!("matrix fixture received an unexpected ordinary prompt");
                };
                assert!(marker.contains("[matrix:mcp]"));
                // The public API deliberately groups mcp.* under integration.invoke
                // and releases only the network origin. Bind that display to this
                // fixture's configured MCP destination and exact model tool call.
                assert_eq!(approval.action, "integration.invoke");
                let configuration = app
                    .state
                    .hosted
                    .as_ref()
                    .unwrap()
                    .project_config(project)
                    .await
                    .unwrap()
                    .unwrap();
                let endpoint = configuration.runtime.mcp.servers["matrix"]
                    .url
                    .as_deref()
                    .unwrap();
                let origin = url::Url::parse(endpoint)
                    .unwrap()
                    .origin()
                    .ascii_serialization();
                assert_eq!(approval.resource, origin);
                assert_eq!(interaction.run_id, snapshot.run.run_id);
                assert_mcp_call(app, headers, &path).await;
                assert!(interaction.respondable_by_caller);
                // Cloud mutation identities are path-safe and scoped to this task;
                // the server also prefixes its own bounded SDK idempotency key.
                let mutation = format!("matrix-approval-{}", answered.len());
                app.ok(
                    headers,
                    Method::POST,
                    &format!("{path}/respond"),
                    json!({"mutation_id":mutation,"request":{
                        "run_id":interaction.run_id,"interaction_id":interaction.interaction_id,
                        "etag":interaction.etag,"idempotency_key":mutation,
                        "response":{"approval":{"approved":true,"request_hash":approval.request_hash}}
                    }}),
                )
                .await;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Hosted settings matrix did not settle {key}: {task:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
        let current: Value = app.read(headers, &path).await;
        task = serde_json::from_value(current["task"].clone()).unwrap();
    }
}

async fn assert_mcp_call(app: &App, headers: &HeaderMap, path: &str) {
    let mut after = 0;
    for _ in 0..8 {
        let batch = app
            .read(headers, &format!("{path}/updates?after={after}"))
            .await;
        let updates: Vec<RunUpdate> = serde_json::from_value(batch["updates"].clone()).unwrap();
        if updates.iter().any(|update| {
            matches!(&update.update, RunUpdateKind::ToolActivity(activity)
                if activity.tool_name == "mcp.call" && activity.call_id == "matrix-mcp_call")
        }) {
            return;
        }
        let Some(next) = batch["next_after"].as_u64() else {
            break;
        };
        assert!(next > after);
        after = next;
    }
    panic!("MCP approval was not paired with the fixture's released mcp.call activity");
}

pub(super) fn successful(task: &CloudTask) -> bool {
    task.snapshot.as_ref().is_some_and(|snapshot| {
        snapshot.run.status == RunStatus::Completed
            && matches!(&snapshot.run.terminal, Some(RunTerminal::Result(result))
                if result.output.contains("matrix-complete"))
    })
}
