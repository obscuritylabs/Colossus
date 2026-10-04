use super::*;
use colossus_ports::AgentRunLifecycle;

fn executor(runtime: &Runtime, workspace: &std::path::Path) -> GatewayToolExecutor {
    let binding = Arc::new(std::sync::OnceLock::new());
    binding.set(Arc::downgrade(&runtime.workflows)).unwrap();
    GatewayToolExecutor {
        gateway: runtime.gateway.clone(),
        filesystem: runtime.filesystem_executor.clone(),
        process: None,
        process_sessions: Some(runtime.process_sessions.clone()),
        workflows: Some(binding),
        http: runtime.http_executor.clone(),
        work: Some(runtime.work_executor.clone()),
        memory: None,
        plugins: None,
        integrations: None,
        mcp: None,
        bound_effects: None,
        search: None,
        workspace: workspace.into(),
        repository_id: "workflow-test".into(),
        executables: Vec::new(),
        plugin_skill_roots: BTreeMap::new(),
    }
}

#[tokio::test]
async fn schedule_tools_require_live_trusted_ownership_and_exact_public_scopes() {
    let root = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::offline_template(root.path().join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(root.path()).unwrap(),
    )
    .unwrap();
    let executor = executor(&runtime, root.path());
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "app:tools".into(),
    };
    let context = ExecutionContext {
        session_id: Some("session-tools".into()),
        run_id: Some("run-tools".into()),
        ..ExecutionContext::default()
    };
    let control = RunControl::default();
    runtime
        .process_sessions
        .begin_run(&context, &owner, control.clone())
        .unwrap();
    runtime.journal.append(NewEvent { stream_id: "api-run:run-tools".into(), expected_stream_version: 0, event_type: "api.run.created.v1".into(), actor: owner.clone(), event_version: 1, classification: colossus_contracts::EventClassification::System, context: context.clone(), payload: json!({"execution": {"application_id": "app:tools", "scopes": ["schedules:read"]}}) }).unwrap();
    let call = ToolCall {
        call_id: "schedule-read".into(),
        name: "workflow.schedule.list".into(),
        arguments: json!({}),
    };
    assert!(
        executor
            .execute_workflow_tool(&call, context.clone())
            .await
            .unwrap()
            .contains("items")
    );
    let task_call = ToolCall {
        call_id: "schedule-task".into(),
        name: "workflow.task.schedule".into(),
        arguments: json!({"schedule_id":"briefing","task":{"name":"Briefing","instructions":"Review procurement."},"calendar":{"timezone":"America/New_York","time":"09:00","weekdays":[1]},"starts_at":"2026-10-05T13:00:00Z","misfire_policy":"fire_once","enabled":true,"idempotency_key":"task-v1"}),
    };
    assert!(matches!(
        executor
            .execute_workflow_tool(&task_call, context.clone())
            .await,
        Err(ToolError::Denied(_))
    ));
    let mut oversized = task_call.clone();
    oversized.arguments["task"]["instructions"] = json!("\n".repeat(32 * 1024));
    assert!(matches!(
        executor
            .execute_workflow_tool(&oversized, context.clone())
            .await,
        Err(ToolError::InvalidArguments { .. })
    ));
    for field in ["origin", "workflow_id", "inputs", "operation"] {
        let mut forged = task_call.clone();
        forged.arguments[field] = json!("forged");
        assert!(
            executor
                .execute_workflow_tool(&forged, context.clone())
                .await
                .is_err()
        );
    }
    let origin = runtime
        .process_sessions
        .workflow_origin(&context, "run-tools", &["schedules:read"])
        .unwrap();
    assert_eq!(origin.owner, owner);
    assert_eq!(origin.session_id, context.session_id);
    assert!(
        runtime
            .process_sessions
            .workflow_origin(&context, "run-tools", &["schedules:create"])
            .is_err()
    );
    assert!(
        runtime
            .process_sessions
            .workflow_origin(&context, "forged-parent", &["schedules:read"])
            .is_err()
    );
    let mut other = context.clone();
    other.session_id = Some("other-workspace-session".into());
    assert!(executor.execute_workflow_tool(&call, other).await.is_err());
    let forged = ToolCall {
        arguments: json!({"origin": {"owner": "other"}}),
        ..call.clone()
    };
    assert!(
        executor
            .execute_workflow_tool(&forged, context.clone())
            .await
            .is_err()
    );
    control.cancel();
    assert!(
        executor
            .execute_workflow_tool(&call, context)
            .await
            .is_err()
    );
}
