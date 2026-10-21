//! Real SSE -> sidecar -> pinned gRPC projection across two executed tool turns.
use super::*;
use colossus_sdk::{RunUpdateKind, ToolActivityState, WatchRunRequest};
use colossus_sidecar_protocol::ManagedProviderKind;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};

#[tokio::test]
#[ignore = "requires Windows native credential storage and loopback; uses only generated homes and a local Anthropic SSE fixture"]
async fn native_anthropic_tool_turns_keep_public_paragraphs_and_private_provider_logs() {
    let instance = PrivateDirectory::in_user_profile();
    let workspace = PrivateDirectory::in_user_profile();
    std::fs::write(
        workspace.path.join("fixture.txt"),
        "disposable file evidence",
    )
    .expect("owned tool input");
    let instance_id = Uuid::now_v7();
    let _keys = KeyringCleanup { instance_id };
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local provider");
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let mut server = tokio::spawn(serve(listener));
    let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::AllowAll)
        .with_execution_boundary(ManagedExecutionBoundary::WorkspaceIsolated);
    runtime.providers[0].kind = ManagedProviderKind::AnthropicMessages;
    runtime.providers[0].base_url = Some(base_url);
    runtime.providers[0].timeout_ms = 10_000;
    runtime.models[0].model = "spacing-fixture".into();
    let grant = SidecarApplicationGrant::new(
        "app:anthropic-spacing-acceptance",
        [scopes::RUNS_EXECUTE, scopes::RUNS_READ]
            .into_iter()
            .map(|scope| ApiScope::new(scope).unwrap()),
        ["primary".into()],
        ["filesystem.read".into()],
    )
    .unwrap();
    let bootstrap = SidecarBootstrapConfig::new(&workspace.path, runtime, grant)
        .unwrap()
        .with_expected_workspace_identity(workspace_identity(&workspace.path))
        .unwrap()
        .with_colossus_home(instance.path.join("home"))
        .unwrap()
        .without_automatic_agent_instructions_for_diagnostics();
    let executable = std::fs::canonicalize(env!("CARGO_BIN_EXE_colossus-sidecar")).unwrap();
    let options = SidecarOptions::new(
        InstanceId::from_uuid(instance_id),
        AppPrivateInstanceDir::new(&instance.path).unwrap(),
        VerifiedExecutable::new(
            &executable,
            Sha256Digest::from_bytes(executable_digest(&executable)),
        )
        .unwrap(),
        ApiMajor::new(1).unwrap(),
    )
    .unwrap();
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let client = Colossus::start_sidecar(&lifecycle, options).await.unwrap();
    let checked = tokio::time::timeout(Duration::from_secs(60), check_stream(&client)).await;
    let closed = client.close().await;
    drop(client);
    let served = tokio::time::timeout(Duration::from_secs(12), &mut server).await;
    if served.is_err() {
        server.abort();
        let _ = server.await;
    }
    closed.expect("acknowledged actual sidecar shutdown");
    let run_id = checked
        .expect("bounded public SSE run")
        .expect("public stream completed");
    served
        .expect("bounded fixture shutdown")
        .expect("fixture task")
        .expect("three real requests");
    let logs = (0..4)
        .filter_map(|slot| {
            std::fs::read_to_string(instance.path.join(format!("logs/sidecar-{slot}.jsonl"))).ok()
        })
        .collect::<String>();
    for expected in [
        "provider.started",
        "provider.completed",
        "anthropic_messages",
        &run_id,
    ] {
        assert!(
            logs.contains(expected),
            "missing safe provider lifecycle field"
        );
    }
    for private in [
        "Doing one.",
        "Doing two.",
        "Finished.",
        "disposable file evidence",
        "fixture.txt",
    ] {
        assert!(
            !logs.contains(private),
            "provider operational logs contain fixture content"
        );
    }
}

async fn check_stream(client: &Colossus) -> Result<String, String> {
    let created = client
        .create_run(CreateRunRequest {
            model_profile: None,
            reasoning_effort: None,
            plugin_skill_ids: Vec::new(),
            input: vec![InputContentPart::Text(
                "Exercise the disposable two-tool fixture.".into(),
            )],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            goal_max_iterations: 0,
            research_depth: None,
            research_sources: Vec::new(),
            plan_action: None,
            branch: None,
            max_turns: 4,
            idempotency_key: IdempotencyKey::new(Uuid::now_v7().to_string()).unwrap(),
        })
        .await
        .map_err(|error| format!("create: {error:?}"))?;
    let run_id = created.run.run_id;
    let mut updates = client
        .watch_run(WatchRunRequest {
            run_id: run_id.clone(),
            after_sequence: 0,
        })
        .await
        .map_err(|error| format!("watch: {error:?}"))?;
    let mut output = String::new();
    let mut completed = false;
    let mut completed_tools = std::collections::BTreeSet::new();
    while let Some(update) = updates.next_update().await {
        match update.map_err(|error| format!("update: {error:?}"))?.update {
            RunUpdateKind::OutputDelta(text) => output.push_str(&text),
            RunUpdateKind::Result(_) => completed = true,
            RunUpdateKind::Failure { failure, .. } => {
                return Err(format!("run: {}", failure.reason));
            }
            RunUpdateKind::ToolActivity(activity) => {
                if activity.tool_name != "filesystem.read"
                    || !matches!(activity.call_id.as_str(), "read-0" | "read-1")
                {
                    return Err("unexpected public tool activity".into());
                }
                match activity.state {
                    ToolActivityState::Completed => {
                        completed_tools.insert(activity.call_id);
                    }
                    ToolActivityState::Failed
                    | ToolActivityState::Cancelled
                    | ToolActivityState::OutcomeUnknown => {
                        return Err("filesystem fixture tool did not complete successfully".into());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if !completed || output != "Doing one.\n\nDoing two.\n\nFinished." {
        return Err(format!(
            "public output did not preserve tool-turn paragraphs: {output:?}; completed={completed}"
        ));
    }
    if completed_tools != std::collections::BTreeSet::from(["read-0".into(), "read-1".into()]) {
        return Err("both filesystem calls require a successful public completion".into());
    }
    Ok(run_id)
}

async fn serve(listener: TcpListener) -> Result<(), String> {
    for step in 0..3 {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(60), listener.accept())
            .await
            .map_err(|_| "provider accept deadline")?
            .map_err(|_| "provider accept")?;
        let request = tokio::time::timeout(Duration::from_secs(10), read_request(&mut socket))
            .await
            .map_err(|_| "request deadline")??;
        if request["stream"] != true || !request["messages"].is_array() {
            return Err("real Anthropic streaming request required".into());
        }
        if step > 0 {
            check_tool_result(&request, &format!("read-{}", step - 1))?;
        }
        let body = response(step);
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket
            .write_all(headers.as_bytes())
            .await
            .map_err(|_| "response headers")?;
        socket
            .write_all(body.as_bytes())
            .await
            .map_err(|_| "SSE response")?;
    }
    Ok(())
}

fn check_tool_result(request: &Value, expected_call: &str) -> Result<(), String> {
    let results = request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|content| {
            content["type"] == "tool_result" && content["tool_use_id"] == expected_call
        })
        .collect::<Vec<_>>();
    if results.len() != 1 || results[0]["is_error"] == true {
        return Err("exact successful filesystem tool result required".into());
    }
    let output: Value = results[0]["content"]
        .as_str()
        .ok_or("text filesystem result required")
        .and_then(|text| serde_json::from_str(text).map_err(|_| "bounded tool receipt required"))?;
    let metadata = &output["_colossusToolObservation"];
    if metadata["toolName"] != "filesystem.read"
        || metadata["callId"] != expected_call
        || metadata["exitCode"] != 0
        || metadata["truncated"] != false
        || metadata["format"] != "text"
        || output["preview"] != "disposable file evidence"
    {
        return Err("matching completed filesystem call and exact released output required".into());
    }
    Ok(())
}

async fn read_request(socket: &mut TcpStream) -> Result<Value, String> {
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        if headers.len() >= 16 * 1024 {
            return Err("bounded headers exceeded".into());
        }
        headers.push(socket.read_u8().await.map_err(|_| "request headers")?);
    }
    let headers = String::from_utf8(headers).map_err(|_| "HTTP encoding")?;
    if !headers.starts_with("POST /v1/messages HTTP/1.1\r\n") {
        return Err("unexpected provider path".into());
    }
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .filter(|length| *length <= 2 * 1024 * 1024)
        .ok_or("bounded request length")?;
    let mut bytes = vec![0; length];
    socket
        .read_exact(&mut bytes)
        .await
        .map_err(|_| "request body")?;
    serde_json::from_slice(&bytes).map_err(|_| "request JSON".into())
}

fn response(step: usize) -> String {
    let text = ["Doing one.", "Doing two.", "Finished."][step];
    let mut events = vec![
        (
            "message_start",
            json!({"type":"message_start","message":{"id":format!("msg-{step}"),"type":"message","role":"assistant","content":[],"usage":{"input_tokens":10,"output_tokens":0}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
    ];
    if step < 2 {
        events.push(("content_block_start", json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":format!("read-{step}"),"name":"filesystem_read","input":{}}})));
        events.push(("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"fixture.txt\"}"}})));
        events.push((
            "content_block_stop",
            json!({"type":"content_block_stop","index":1}),
        ));
    }
    events.push(("message_delta", json!({"type":"message_delta","delta":{"stop_reason":if step<2 {"tool_use"} else {"end_turn"}},"usage":{"output_tokens":10}})));
    events.push(("message_stop", json!({"type":"message_stop"})));
    events
        .into_iter()
        .map(|(name, value)| format!("event: {name}\ndata: {value}\n\n"))
        .collect()
}
