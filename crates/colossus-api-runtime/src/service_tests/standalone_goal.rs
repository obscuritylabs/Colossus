//! Standalone Goals use the durable public run lifecycle and original caller ceiling.
use super::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn fixture() -> RuntimeFixture {
    let directory = runtime_tempdir();
    let root = fs::canonicalize(directory.path()).unwrap();
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("workflows");
    fs::create_dir_all(&config.workflows.repository).unwrap();
    RuntimeFixture {
        runtime: Arc::new(
            Runtime::open_with_options(
                &config,
                Arc::new(DenyApproval),
                None,
                RuntimeOpenOptions::for_workspace(&root).unwrap(),
            )
            .unwrap(),
        ),
        _directory: directory,
    }
}

#[tokio::test]
async fn standalone_goal_is_plan_free_budgeted_and_idempotent() {
    let fixture = fixture();
    let runtime = fixture.runtime;
    let api = service(runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_scopes_and_tools(
        "app:goal-owner",
        "goal-start",
        &[],
        &["echo", "goal.show", "goal.update"],
    );
    let mut input = request("standalone-goal", "Report the selected workspace status");
    input.mode = RunMode::Goal;
    input.goal_max_iterations = 3;
    input.max_turns = 3;
    let created = api.create_run(&owner, input.clone()).await.unwrap();
    let terminal = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(terminal.mode, RunMode::Goal);
    assert_eq!(terminal.status, RunStatus::Completed, "{terminal:?}");
    let result = terminal.result.expect("Goal result");
    assert!(result.plan_id.is_none());
    assert!(result.plan_revision.is_none());
    assert!(result.plan_status.is_none());
    let id = result.goal_id.unwrap();
    assert!(result.output.contains("iteration limit was reached"));
    let events = runtime
        .journal()
        .read_stream(&format!("goal:{id}"))
        .unwrap();
    let goal = runtime
        .journal()
        .decrypt_payload(events.last().unwrap())
        .unwrap();
    assert_eq!(goal["record"]["iterations_completed"], 3);
    assert!(goal["record"]["source_plan_id"].is_null());
    let replay = api.create_run(&owner, input).await.unwrap();
    assert_eq!(replay.run.id, created.run.id);
    assert_eq!(
        runtime
            .journal()
            .list_stream_ids("goal:", None, 10)
            .unwrap()
            .len(),
        1
    );
    assert!(
        runtime
            .journal()
            .list_stream_ids("plan:", None, 10)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn standalone_goal_denies_missing_tools_and_foreign_sessions_before_creation() {
    let fixture = fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let denied = caller("app:limited", "goal-denied");
    let mut input = request("goal-denied", "An explicit objective");
    input.mode = RunMode::Goal;
    input.goal_max_iterations = 2;
    assert_eq!(
        api.create_run(&denied, input.clone())
            .await
            .unwrap_err()
            .code,
        colossus_api::ApiErrorCode::PermissionDenied
    );
    let owner = caller_with_scopes_and_tools(
        "app:goal-owner",
        "goal-source",
        &[],
        &["echo", "goal.show", "goal.update"],
    );
    let source = api
        .create_run(&owner, request("source", "Owned conversation"))
        .await
        .unwrap();
    wait_terminal(&api, &owner, &source.run.id).await;
    let foreign = caller_with_scopes_and_tools(
        "app:other",
        "goal-foreign",
        &[],
        &["echo", "goal.show", "goal.update"],
    );
    input.session_id = Some(source.run.session_id);
    assert_eq!(
        api.create_run(&foreign, input).await.unwrap_err().code,
        colossus_api::ApiErrorCode::NotFound
    );
    assert!(
        fixture
            .runtime
            .journal()
            .list_stream_ids("goal:", None, 10)
            .unwrap()
            .is_empty()
    );
}

fn response(content: Value) -> String {
    format!("data: {content}\n\ndata: [DONE]\n\n")
}

fn text_response(text: &str) -> String {
    response(json!({"id": "goal-fixture", "choices": [{"index": 0,
        "delta": {"content": text}, "finish_reason": "stop"}]}))
}

fn tool_response(name: &str, arguments: Value) -> String {
    response(json!({"id": "goal-fixture", "choices": [{"index": 0,
        "delta": {"tool_calls": [{"index": 0, "id": format!("call-{name}"),
            "type": "function", "function": {"name": name, "arguments": arguments.to_string()}}]},
        "finish_reason": "tool_calls"}]}))
}

async fn serve(listener: TcpListener, responses: Vec<String>) -> Vec<Value> {
    let mut requests = Vec::new();
    for response in responses {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let (start, length) = loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() <= 1_048_576);
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break (end + 4, length);
                }
            }
        };
        requests.push(serde_json::from_slice(&bytes[start..start + length]).unwrap());
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
    }
    requests
}

fn provider_fixture(origin: String) -> (RuntimeFixture, Arc<PublicInteractionRouter>) {
    let directory = runtime_tempdir();
    let root = fs::canonicalize(directory.path()).unwrap();
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("workflows");
    fs::create_dir_all(&config.workflows.repository).unwrap();
    config.access = serde_json::from_value(json!({
        "profile": "pinned", "tools": {"include": ["goal.show", "goal.update", "user.ask", "echo"]},
        "actions": {"allow": ["provider.openai.chat", "goal.show", "goal.update", "goal.iteration.record"]}
    }))
    .unwrap();
    config.providers = serde_json::from_value(json!({"profiles": {"fixture": {
        "kind": "open_ai_compatible", "baseUrl": format!("{origin}/v1"), "timeoutMs": 5000
    }}}))
    .unwrap();
    config.models = serde_json::from_value(json!({
        "profiles": {"fixture": {"providerProfile": "fixture", "model": "goal-fixture",
            "contextWindowTokens": 32768, "maxOutputTokens": 4096,
            "capabilities": {"toolCalls": true, "streaming": true}}}, "roles": {"primary": "fixture"}
    })).unwrap();
    config.sandbox.network_destinations = vec![origin];
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    (
        RuntimeFixture {
            runtime: Arc::new(
                Runtime::open_with_options(
                    &config,
                    interactions.clone(),
                    Some(interactions.clone()),
                    RuntimeOpenOptions::for_workspace(&root).unwrap(),
                )
                .unwrap(),
            ),
            _directory: directory,
        },
        interactions,
    )
}

#[tokio::test]
async fn standalone_goal_completes_or_blocks_early_under_each_iterations_tool_ceiling() {
    for status in ["complete", "blocked"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let provider = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(15), serve(listener, vec![
                text_response("The first bounded step made progress."),
                tool_response("goal_update", json!({"status": status, "summary": "Verified progress.",
                    "blocked_reason": if status == "blocked" { "Need the next input." } else { "" }})),
                text_response("Verified progress."),
            ])).await.unwrap()
        });
        let (fixture, interactions) = provider_fixture(origin);
        let runtime = fixture.runtime.clone();
        let repository = Arc::new(EventSourcedRunRepository::new(runtime.journal()));
        let api = RuntimeAgentRunApi::with_repository(
            runtime.clone(),
            repository,
            interactions,
            "primary",
            "Work toward the objective.",
        );
        let owner = caller_with_scopes_and_tools(
            "app:bounded-goal",
            status,
            &[],
            &["goal.show", "goal.update"],
        );
        let mut input = request(status, "Verify the workspace outcome.");
        input.mode = RunMode::Goal;
        input.goal_max_iterations = 5;
        input.max_turns = 2;
        let created = api.create_run(&owner, input).await.unwrap();
        let terminal = wait_terminal(&api, &owner, &created.run.id).await;
        assert_eq!(terminal.status, RunStatus::Completed, "{terminal:?}");
        let result = terminal.result.unwrap();
        assert!(!result.output.contains("iteration limit was reached"));
        assert_eq!(
            result.output.contains("Goal blocked: Need the next input."),
            status == "blocked"
        );
        let events = runtime
            .journal()
            .read_stream(&format!("goal:{}", result.goal_id.unwrap()))
            .unwrap();
        let goal = runtime
            .journal()
            .decrypt_payload(events.last().unwrap())
            .unwrap();
        assert_eq!(goal["record"]["status"], status);
        assert_eq!(goal["record"]["iterations_completed"], 2);
        assert_eq!(events[0].actor, owner.actor());
        let requests = provider.await.unwrap();
        assert_eq!(requests.len(), 3);
        for request in requests {
            let mut tools = request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["function"]["name"].as_str().unwrap())
                .collect::<Vec<_>>();
            tools.sort_unstable();
            assert_eq!(
                tools,
                ["goal_show", "goal_update"],
                "no runtime-wide tool grant leaks into later iterations"
            );
        }
    }
}

#[tokio::test]
async fn standalone_goal_cancels_a_pending_question_without_another_iteration() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(15),
            serve(
                listener,
                vec![tool_response(
                    "user_ask",
                    json!({"question": "Which outcome?", "choices": [], "allow_free_form": true}),
                )],
            ),
        )
        .await
        .unwrap()
    });
    let (fixture, interactions) = provider_fixture(origin);
    let runtime = fixture.runtime.clone();
    let repository = Arc::new(EventSourcedRunRepository::new(runtime.journal()));
    let api = RuntimeAgentRunApi::with_repository(
        runtime.clone(),
        repository.clone(),
        interactions,
        "primary",
        "Ask for necessary input.",
    );
    let owner = caller_with_scopes_and_tools(
        "app:cancel-goal",
        "cancel-goal",
        &[],
        &["goal.show", "goal.update", "user.ask"],
    );
    let mut input = request("cancel-goal", "Ask which outcome to produce.");
    input.mode = RunMode::Goal;
    input.goal_max_iterations = 5;
    let created = api.create_run(&owner, input.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = api
                .get_run(
                    &owner,
                    GetRunRequest {
                        run_id: created.run.id.clone(),
                    },
                )
                .await
                .unwrap();
            assert!(!run.status.is_terminal(), "{run:?}");
            if run.pending_interaction.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    api.cancel_run(
        &owner,
        CancelRunRequest {
            run_id: created.run.id.clone(),
            idempotency_key: IdempotencyKey::new("cancel-goal-stop").unwrap(),
        },
    )
    .await
    .unwrap();
    let terminal = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(terminal.status, RunStatus::Cancelled, "{terminal:?}");
    let cancellation = terminal.cancellation.unwrap();
    assert!(cancellation.plan_id.is_none());
    let id = cancellation.goal_id.unwrap();
    let events = runtime
        .journal()
        .read_stream(&format!("goal:{id}"))
        .unwrap();
    let goal = runtime
        .journal()
        .decrypt_payload(events.last().unwrap())
        .unwrap();
    assert_eq!(goal["record"]["iterations_completed"], 1);
    assert_eq!(goal["record"]["status"], "active");
    let replay = api.create_run(&owner, input).await.unwrap();
    assert_eq!(replay.run.id, created.run.id);
    assert_eq!(replay.run.status, RunStatus::Cancelled);
    assert_eq!(provider.await.unwrap().len(), 1);
    assert!(repository.updates_after(&owner, &created.run.id, 0, 100).unwrap().iter()
        .any(|update| matches!(&update.kind, RunUpdateKind::ToolActivity { activity } if activity.tool_name == "user.ask")));
}

#[tokio::test]
async fn standalone_goal_unknown_terminal_outcome_is_not_replayed() {
    let fixture = fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    api.inject_next_execution_fault(ExecutionTestFault::FailTerminalAppend);
    let owner = caller_with_scopes_and_tools(
        "app:goal-unknown",
        "goal-unknown",
        &[],
        &["echo", "goal.show", "goal.update"],
    );
    let mut input = request("goal-unknown", "Make bounded progress.");
    input.mode = RunMode::Goal;
    input.goal_max_iterations = 2;
    let created = api.create_run(&owner, input.clone()).await.unwrap();
    let terminal = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(terminal.status, RunStatus::OutcomeUnknown, "{terminal:?}");
    assert_eq!(terminal.failure.unwrap().outcome, OutcomeCertainty::Unknown);
    let before = fixture
        .runtime
        .journal()
        .list_stream_ids("goal:", None, 10)
        .unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(
        api.create_run(&owner, input).await.unwrap().run.id,
        created.run.id
    );
    assert_eq!(
        fixture
            .runtime
            .journal()
            .list_stream_ids("goal:", None, 10)
            .unwrap(),
        before
    );
    wait_inactive(&api).await;
}

#[tokio::test]
async fn standalone_goal_rejects_stale_skill_selection_before_goal_creation() {
    let fixture = fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_scopes_and_tools(
        "app:goal-skills",
        "goal-skills",
        &[],
        &["echo", "goal.show", "goal.update"],
    );
    let mut input = request("goal-skills", "Work with the selected skill.");
    input.mode = RunMode::Goal;
    input.goal_max_iterations = 2;
    input.skill_ids = vec!["missing/stale".into()];
    let created = api.create_run(&owner, input).await.unwrap();
    let terminal = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(terminal.status, RunStatus::Failed, "{terminal:?}");
    assert!(
        fixture
            .runtime
            .journal()
            .list_stream_ids("goal:", None, 10)
            .unwrap()
            .is_empty()
    );
}
