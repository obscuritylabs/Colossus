use super::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn tool_response(name: &str, arguments: Value) -> String {
    let response = json!({
        "id": "plan-fixture",
        "choices": [{"index": 0, "delta": {"tool_calls": [{
            "index": 0, "id": format!("call-{name}"), "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()}
        }]}, "finish_reason": "tool_calls"}]
    });
    format!("data: {response}\n\ndata: [DONE]\n\n")
}

async fn serve_plan(listener: TcpListener) -> Vec<Value> {
    let responses = [
        tool_response(
            "user_ask",
            json!({
                "question": "Which tone?", "choices": ["Formal", "Casual"], "allow_free_form": false
            }),
        ),
        tool_response(
            "plan_create",
            json!({
                "prompt": "Plan a welcome message", "content": "Use the selected casual tone.",
                "steps": [{"title": "Write a casual welcome in chat", "requires_mutation": false}]
            }),
        ),
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({
                "id": "plan-finished", "choices": [{"index": 0,
                    "delta": {"content": "Casual welcome plan ready."}, "finish_reason": "stop"}]
            })
        ),
    ];
    let mut requests = Vec::new();
    for response in responses {
        let (mut stream, _) = listener.accept().await.expect("provider connection");
        let mut bytes = Vec::new();
        let (body_start, content_length) = loop {
            let mut chunk = [0_u8; 4096];
            let count = stream.read(&mut chunk).await.expect("request bytes");
            assert!(count > 0, "complete provider request");
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() <= 1_048_576, "bounded provider request");
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .expect("content length")
                    .trim()
                    .parse::<usize>()
                    .expect("numeric length");
                if bytes.len() >= end + 4 + length {
                    break (end + 4, length);
                }
            }
        };
        requests.push(
            serde_json::from_slice(&bytes[body_start..body_start + content_length])
                .expect("request JSON"),
        );
        stream.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()
        ).as_bytes()).await.expect("provider response");
    }
    requests
}

#[tokio::test]
async fn public_plan_question_resumes_after_answer_and_persists_one_draft() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback provider");
    let origin = format!(
        "http://{}",
        listener.local_addr().expect("provider address")
    );
    let provider = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(15), serve_plan(listener))
            .await
            .expect("provider deadline")
    });
    let directory = runtime_tempdir();
    let root = fs::canonicalize(directory.path()).expect("canonical workspace");
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("workflows");
    fs::create_dir_all(&config.workflows.repository).expect("workflows");
    config.access = serde_json::from_value(json!({
        "profile": "pinned", "tools": {"include": ["user.ask", "plan.create"]},
        "actions": {"allow": ["provider.openai.chat", "plan.create"]}
    }))
    .expect("access");
    config.providers = serde_json::from_value(json!({"profiles": {"fixture": {
        "kind": "open_ai_compatible", "baseUrl": format!("{origin}/v1"), "timeoutMs": 5000
    }}}))
    .expect("provider config");
    config.models = serde_json::from_value(json!({
        "profiles": {"fixture": {"providerProfile": "fixture", "model": "plan-fixture",
            "contextWindowTokens": 32768, "maxOutputTokens": 4096,
            "capabilities": {"toolCalls": true, "streaming": true}}},
        "roles": {"primary": "fixture"}
    }))
    .expect("model config");
    config.sandbox.network_destinations = vec![origin];
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let runtime = Arc::new(
        Runtime::open_with_options(
            &config,
            interactions.clone(),
            Some(interactions.clone()),
            RuntimeOpenOptions::for_workspace(&root).expect("workspace"),
        )
        .expect("runtime"),
    );
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(runtime.journal()));
    let service = Arc::new(RuntimeAgentRunApi::with_repository(
        Arc::clone(&runtime),
        Arc::clone(&repository),
        interactions,
        "primary",
        "Plan the request.",
    ));
    let owner = caller_with_scopes_and_tools(
        "app:plan-question",
        "plan-question",
        &[],
        &["user.ask", "plan.create"],
    );
    let mut create = request(
        "plan-question-create",
        "Plan a welcome message; ask which tone first.",
    );
    create.mode = RunMode::Plan;
    create.max_turns = 3;
    let created = service
        .create_run(&owner, create)
        .await
        .expect("create Plan run");
    let pending = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = service
                .get_run(
                    &owner,
                    GetRunRequest {
                        run_id: created.run.id.clone(),
                    },
                )
                .await
                .expect("run");
            assert!(
                !run.status.is_terminal(),
                "planning failed before its question: {run:?}"
            );
            if run.pending_interaction.is_some() {
                break run;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("question appeared");
    let updates = repository
        .updates_after(&owner, &pending.id, 0, 100)
        .expect("pending updates");
    let started = updates.iter().position(|update| matches!(&update.kind,
        RunUpdateKind::ToolActivity { activity } if activity.tool_name == "user.ask" && activity.state == colossus_api::ToolActivityState::Started
    )).expect("tool start released before question");
    let question = updates
        .iter()
        .position(|update| matches!(&update.kind, RunUpdateKind::Interaction { .. }))
        .expect("question update");
    assert!(started < question);
    let interaction = pending.pending_interaction.expect("pending question");
    service
        .respond_interaction(
            &owner,
            RespondInteractionRequest {
                run_id: pending.id,
                interaction_id: interaction.id,
                etag: pending.etag,
                idempotency_key: IdempotencyKey::new("answer-tone").expect("answer key"),
                response: InteractionResponse::Prompt {
                    answer: "Casual".into(),
                    selected_index: Some(1),
                },
            },
        )
        .await
        .expect("answer question");
    let completed = wait_terminal(&service, &owner, &created.run.id).await;
    assert_eq!(completed.status, RunStatus::Completed, "{completed:?}");
    let plans = runtime
        .list_plans(Some(&completed.session_id), None, 10)
        .expect("plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].revision, 1);
    assert_eq!(plans[0].status, colossus_contracts::PlanStatus::Draft);
    let requests = provider.await.expect("provider task");
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1]["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|message| message["role"] == "tool"
                && message["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("Casual")))
    );
}
