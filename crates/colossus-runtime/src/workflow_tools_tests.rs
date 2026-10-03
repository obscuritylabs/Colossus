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
