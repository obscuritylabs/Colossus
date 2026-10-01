//! Real helper and agent-loop acceptance for managed shell ownership.
#![cfg(unix)]
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, CommandIntent, DecisionOutcome, EffectPhase, EffectRequest, PolicyDecision,
    ProcessLifetime, ProcessSessionStatus,
};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, ExecutionError, ReleasedEffectObserver,
    ReleasedEffectResult, SafetyKernel, effect_request,
};
use colossus_ports::{EventJournal, PolicyDecisionPoint, PolicyError};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, SandboxConfig};
use colossus_sandbox::{
    ProcessControl, ProcessSpec, SandboxExecutorConfig, SandboxProcessExecutor,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "terminal-user".into(),
    }
}
fn helper() -> SandboxProcessExecutor {
    SandboxProcessExecutor::new(
        SandboxExecutorConfig {
            helper_executable: env!("CARGO_BIN_EXE_colossus").into(),
            oci_runtime: None,
            oci_image: None,
            oci_proxy_image: None,
        },
        [7; 32],
    )
}
fn policy(directory: &Path, timeout: u64, output: u64) -> BuiltInPolicy {
    BuiltInPolicy::offline_default()
        .with_action("shell.run", DecisionOutcome::Allow)
        .with_sandbox("external", "managed-test", false)
        .with_filesystem_root(directory.to_string_lossy(), "write")
        .with_filesystem_root(
            std::fs::canonicalize("/bin/sh")
                .expect("shell")
                .to_string_lossy(),
            "execute",
        )
        .with_filesystem_root(
            std::fs::canonicalize("/bin/sleep")
                .expect("sleep")
                .to_string_lossy(),
            "execute",
        )
        .with_limits(timeout, output, 16, 512 * 1024 * 1024, 2)
}
fn request(directory: &Path, command: &str, timeout: u64, output: u64) -> EffectRequest {
    let spec = ProcessSpec {
        lifetime: Some(ProcessLifetime::Workspace),
        cwd: directory.into(),
        args: vec!["-c".into(), command.into()],
        environment: BTreeMap::new(),
        stdin_base64: None,
        stdin_completion: None,
        timeout_ms: Some(timeout),
        max_output_bytes: Some(output),
    };
    let mut request = effect_request(
        actor(),
        "shell.run",
        "/bin/sh",
        serde_json::to_value(spec).expect("spec"),
    );
    request.capabilities = vec!["shell.run".into()];
    request.command_intent = Some(CommandIntent {
        justification: "Verify managed process lifecycle".into(),
    });
    request
}
#[derive(Default)]
struct Output {
    frames: Vec<Value>,
    stop_on_output: Option<ProcessControl>,
}
#[async_trait]
impl ReleasedEffectObserver for Output {
    async fn observe(&mut self, result: ReleasedEffectResult) -> Result<(), ExecutionError> {
        let value: Value = serde_json::from_slice(&result.bytes).expect("frame");
        if value["kind"] == "output"
            && let Some(control) = &self.stop_on_output
        {
            control.cancel();
        }
        self.frames.push(value);
        Ok(())
    }
}
fn journal(directory: &Path) -> Arc<dyn EventJournal> {
    let mut config = RuntimeConfig::offline_template(directory.join("test.redb"));
    config.use_ephemeral_storage();
    Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(directory).expect("workspace"),
    )
    .expect("journal runtime")
    .journal()
}
#[tokio::test]
async fn real_helper_streams_stops_descendants_times_out_and_bounds_floods() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let journal = journal(&root);
    let gateway = EffectGateway::new(
        journal,
        Arc::new(policy(&root, 5000, 65536)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["shell.run".into()]).with_sandbox_boundary_gate(Arc::new(
            colossus_policy::SandboxBoundaryGate::new(
                Some(colossus_contracts::SandboxBoundaryMode::External),
                true,
            ),
        )),
        [9; 32],
    );
    let control = ProcessControl::default();
    let executor = helper().controlled(control.clone());
    let mut output = Output {
        stop_on_output: Some(control),
        ..Output::default()
    };
    let result = gateway
        .execute_stream(
            request(
                &root,
                "printf ready; (sleep 1; printf escaped > escaped) & wait",
                5000,
                65536,
            ),
            &executor,
            &mut output,
        )
        .await
        .expect("stopped stream");
    let terminal: Value = serde_json::from_slice(&result.bytes).expect("terminal");
    assert_eq!(terminal["result"]["stopped"], true);
    assert_eq!(output.frames.first().expect("started")["kind"], "started");
    assert!(output.frames.iter().any(|frame| frame["kind"] == "output"));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(!root.join("escaped").exists(), "stop must reap descendants");
    let started = Instant::now();
    let terminal = gateway
        .execute_stream(
            request(&root, "sleep 5", 500, 65536),
            &helper(),
            &mut Output::default(),
        )
        .await
        .expect("deadline stream");
    let terminal: Value = serde_json::from_slice(&terminal.bytes).expect("terminal");
    assert_eq!(terminal["result"]["timed_out"], true);
    assert!(started.elapsed() < Duration::from_secs(2));
    let terminal = gateway
        .execute_stream(
            request(
                &root,
                "i=0; while [ $i -lt 5000 ]; do printf 01234567890123456789; i=$((i+1)); done",
                5000,
                1024,
            ),
            &helper(),
            &mut Output::default(),
        )
        .await
        .expect("bounded flood");
    let terminal: Value = serde_json::from_slice(&terminal.bytes).expect("terminal");
    assert_eq!(terminal["result"]["output_truncated"], true);
}

#[tokio::test]
async fn native_streaming_and_dropped_call_reap_the_process_tree() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path().canonicalize().expect("workspace");
    let gateway = Arc::new(EffectGateway::new(
        journal(&root),
        Arc::new(policy(&root, 5000, 65536).with_sandbox("native", "managed-native-test", false)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["shell.run".into()]),
        [9; 32],
    ));
    let control = ProcessControl::default();
    let mut output = Output {
        stop_on_output: Some(control.clone()),
        ..Output::default()
    };
    let result = gateway
        .execute_stream(
            request(&root, "printf ready; /bin/sleep 5", 5000, 65536),
            &helper().controlled(control),
            &mut output,
        )
        .await
        .expect("native stream");
    let result: Value = serde_json::from_slice(&result.bytes).expect("terminal");
    assert_eq!(
        result["result"]["stopped"], true,
        "native result: {result}; frames: {:?}",
        output.frames
    );
    assert!(output.frames.iter().any(|frame| frame["kind"] == "output"));

    let task_root = root.clone();
    let task = tokio::spawn(async move {
        gateway
            .execute_stream(
                request(
                    &task_root,
                    "printf ready > started; (/bin/sleep 2 2>sleep-error; printf escaped > escaped) & wait",
                    5000,
                    65536,
                ),
                &helper(),
                &mut Output::default(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(4), async {
        while !root.join("started").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("child started");
    assert!(
        !root.join("escaped").exists(),
        "delay must still be active before abort: {:?}",
        std::fs::read_to_string(root.join("sleep-error"))
    );
    task.abort();
    assert!(task.await.expect_err("cancelled call").is_cancelled());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(
        !root.join("escaped").exists(),
        "dropping the caller must stop descendants; sleep error: {:?}",
        std::fs::read_to_string(root.join("sleep-error"))
    );
}

// Only the spawned integration-test process enters this fixture server.
#[test]
fn managed_webserver_fixture() {
    let Some(port_file) = std::env::var_os("COLOSSUS_MANAGED_TEST_SERVER_FILE") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("test webserver");
    std::fs::write(
        port_file,
        listener.local_addr().expect("address").to_string(),
    )
    .expect("port file");
    println!("ready");
    std::io::stdout().flush().expect("flush");
    for stream in listener.incoming() {
        let mut stream = stream.expect("connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let mut buffer = [0; 1024];
        let _ = stream.read(&mut buffer);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready")
            .expect("response");
    }
}

fn server_response(address: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("background server remains reachable");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .expect("request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("response");
    response
}

struct DenyRelease(BuiltInPolicy);
#[async_trait]
impl PolicyDecisionPoint for DenyRelease {
    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.0.doctor().await
    }
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
        }
        Ok(decision)
    }
}
#[tokio::test]
async fn denied_release_never_reaches_logs_and_stops_the_child() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let gateway = EffectGateway::new(
        journal(&root),
        Arc::new(DenyRelease(policy(&root, 5000, 65536))),
        Arc::new(DenyApproval),
        SafetyKernel::new(["shell.run".into()]).with_sandbox_boundary_gate(Arc::new(
            colossus_policy::SandboxBoundaryGate::new(
                Some(colossus_contracts::SandboxBoundaryMode::External),
                true,
            ),
        )),
        [9; 32],
    );
    let mut output = Output::default();
    assert!(
        gateway
            .execute_stream(
                request(
                    &root,
                    "printf secret; sleep 1; printf escaped > escaped",
                    5000,
                    65536
                ),
                &helper(),
                &mut output
            )
            .await
            .is_err()
    );
    assert!(output.frames.is_empty());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(!root.join("escaped").exists());
}

fn tool(name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"id":"tool", "choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":uuid::Uuid::now_v7().to_string(),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]})
    )
}
fn final_answer() -> String {
    "data: {\"id\":\"final\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into()
}
fn read_request(stream: &mut TcpStream) -> Value {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).expect("request");
        assert_ne!(count, 0);
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let length = String::from_utf8_lossy(&request[..end])
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("length"))
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                return serde_json::from_slice(&request[end + 4..end + 4 + length])
                    .expect("request JSON");
            }
        }
        assert!(request.len() < 4 * 1024 * 1024);
    }
}
#[tokio::test]
async fn background_survives_final_reply_and_later_turn_while_run_jobs_are_reaped() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path().canonicalize().expect("workspace");
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    let port_file = root.join("server-address");
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
    let command = format!(
        "COLOSSUS_MANAGED_TEST_SERVER_FILE={} {} --exact managed_webserver_fixture --nocapture",
        quote(&port_file.to_string_lossy()),
        quote(
            &std::env::current_exe()
                .expect("test executable")
                .to_string_lossy()
        )
    );
    let responses = vec![
        tool(
            "shell_run",
            json!({"command":command,"justification":"Run the requested background job","lifetime":"workspace","yield_time_ms":10000,"timeout_ms":30000}),
        ),
        final_answer(),
        tool("shell_list", json!({})),
        final_answer(),
        tool(
            "shell_run",
            json!({"command":"printf ready; while :; do sleep 1; done","justification":"Verify ordinary run cleanup","lifetime":"run","yield_time_ms":10000,"timeout_ms":30000}),
        ),
        final_answer(),
    ];
    let provider = thread::spawn(move || {
        let mut requests = Vec::new();
        listener.set_nonblocking(true).expect("nonblocking");
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "provider request timed out");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            stream.set_nonblocking(false).expect("blocking");
            requests.push(read_request(&mut stream));
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.len(), response).expect("response");
        }
        requests
    });
    let runtime = managed_runtime(&root, &origin);
    let first = runtime
        .run_model("primary", "Execute the requested test tool.", "start")
        .await
        .expect("first run");
    let page = runtime
        .list_process_sessions(actor(), None)
        .await
        .expect("sessions");
    assert_eq!(page.sessions.len(), 1);
    let background = &page.sessions[0];
    assert_eq!(
        background.status,
        ProcessSessionStatus::Running,
        "background must survive final reply: {background:?}"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !port_file.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("server starts");
    let address = std::fs::read_to_string(&port_file).expect("server address");
    assert!(server_response(&address).ends_with("ready"));
    let deadline = background.deadline_ms;
    runtime
        .run_model_in_session(
            "primary",
            "Inspect the background job",
            "inspect",
            None,
            first.session_id.as_deref().expect("chat"),
        )
        .await
        .expect("later turn");
    runtime
        .run_model_in_session(
            "primary",
            "Start an ordinary job",
            "ordinary",
            None,
            first.session_id.as_deref().expect("chat"),
        )
        .await
        .expect("ordinary turn");
    let sessions = runtime
        .list_process_sessions(actor(), None)
        .await
        .expect("sessions")
        .sessions;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.lifetime == ProcessLifetime::Run)
            .expect("ordinary job")
            .status,
        ProcessSessionStatus::Stopped
    );
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == background.id)
            .expect("background")
            .deadline_ms,
        deadline
    );
    assert!(server_response(&address).ends_with("ready"));
    let alien = Actor {
        actor_type: ActorType::Application,
        id: "other-application".into(),
    };
    assert!(
        runtime
            .list_process_sessions(alien.clone(), None)
            .await
            .expect("isolated list")
            .sessions
            .is_empty()
    );
    assert!(
        runtime
            .read_process_session(alien, background.id.clone(), 0, 0, 65536)
            .await
            .is_err()
    );
    runtime
        .stop_process_session(actor(), background.id.clone())
        .await
        .expect("stop");
    runtime.drain_process_sessions().await;
    let final_state = runtime
        .read_process_session(actor(), background.id.clone(), 0, 0, 65536)
        .await
        .expect("final state");
    assert_eq!(final_state.session.status, ProcessSessionStatus::Stopped);
    assert!(
        TcpStream::connect(&address).is_err(),
        "stopped server no longer accepts connections"
    );
    assert!(
        final_state
            .chunks
            .iter()
            .any(|chunk| chunk.stdout.contains("ready"))
    );
    let requests = provider.join().expect("provider thread");
    assert!(
        requests[3].to_string().contains(&background.id),
        "later turn must observe the same process identity"
    );
}

fn managed_runtime(root: &Path, origin: &str) -> Runtime {
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.use_ephemeral_storage();
    config.sandbox = SandboxConfig {
        helper_path: Some(env!("CARGO_BIN_EXE_colossus").into()),
        max_concurrency: 4,
        timeout_ms: 30000,
        ..SandboxConfig::default()
    };
    config.memory.index_enabled = false;
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("user-workflows");
    let mut value = serde_json::to_value(config).expect("config");
    value["providers"] = json!({"profiles":{"test":{"kind":"open_ai_compatible","baseUrl":format!("{origin}/v1"),"credentialReference":null,"timeoutMs":10000}}});
    value["models"] = json!({"profiles":{"test":{"providerProfile":"test","model":"test-model","contextWindowTokens":65536,"maxOutputTokens":4096,"capabilities":{"toolCalls":true,"streaming":true}}},"roles":{"primary":"test","subagent_default":"test"}});
    let config = RuntimeConfig::from_yaml(&value.to_string()).expect("valid config");
    Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(root).expect("workspace"),
    )
    .expect("runtime")
}

struct IgnoreRunEvents;
#[async_trait]
impl colossus_ports::RunEventObserver for IgnoreRunEvents {
    async fn observe(
        &mut self,
        _: colossus_contracts::RunEventEnvelope,
    ) -> Result<(), colossus_ports::ModelProviderError> {
        Ok(())
    }
}

#[tokio::test]
async fn delegated_background_shells_keep_application_ownership_after_parent_completion() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path().canonicalize().expect("workspace");
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    let provider = thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let mut parent_requests = 0;
        let mut child_requests = 0;
        for _ in 0..4 {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "provider request timed out");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            stream.set_nonblocking(false).expect("blocking");
            let request = read_request(&mut stream);
            let parent = request["tools"]
                .as_array()
                .expect("tools")
                .iter()
                .any(|tool| tool["function"]["name"] == "agent_delegate");
            let response = if parent {
                parent_requests += 1;
                if parent_requests == 1 {
                    tool(
                        "agent_delegate",
                        json!({"task":"start a delegated background shell"}),
                    )
                } else {
                    final_answer()
                }
            } else {
                child_requests += 1;
                if child_requests % 2 == 1 {
                    tool(
                        "shell_run",
                        json!({"command":"printf delegated-output; /bin/sleep 30","justification":"Verify delegated background ownership","lifetime":"workspace","yield_time_ms":10000,"timeout_ms":30000}),
                    )
                } else {
                    final_answer()
                }
            };
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.len(), response).expect("response");
        }
        assert_eq!((parent_requests, child_requests), (2, 2));
    });
    let runtime = managed_runtime(&root, &origin);
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "app:delegated-shell-owner".into(),
    };
    runtime
        .run_model_with_skills_stream_controlled_as(
            "primary",
            "Execute the requested test tool.",
            "delegate",
            Some(4),
            None,
            &[],
            &[],
            owner.clone(),
            &mut IgnoreRunEvents,
            &colossus_ports::RunControl::default(),
        )
        .await
        .expect("application parent and delegated child");
    let first = runtime
        .list_process_sessions(owner.clone(), None)
        .await
        .expect("owned shells");
    assert_eq!(
        first.sessions.len(),
        1,
        "application must retain its child's shell"
    );
    let background = &first.sessions[0];
    assert_eq!(background.owner, owner);
    assert_eq!(background.status, ProcessSessionStatus::Running);
    let child = background.subagent_id.as_deref().expect("child lineage");
    assert!(
        runtime
            .list_process_sessions(actor(), None)
            .await
            .expect("terminal shells")
            .sessions
            .is_empty()
    );
    let foreign = Actor {
        id: "app:other-shell-owner".into(),
        ..owner.clone()
    };
    assert!(
        runtime
            .list_process_sessions(foreign.clone(), None)
            .await
            .expect("foreign shells")
            .sessions
            .is_empty()
    );
    assert!(
        runtime
            .stop_process_session(foreign, background.id.clone())
            .await
            .is_err()
    );
    let job = runtime.get_subagent(child).expect("job").expect("child");
    assert_eq!(job.status, colossus_contracts::SubagentStatus::Completed);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = runtime
                .read_process_session(owner.clone(), background.id.clone(), 0, 100, 65536)
                .await
                .expect("owner can read live child output");
            if snapshot
                .chunks
                .iter()
                .any(|chunk| chunk.stdout.contains("delegated-output"))
            {
                break;
            }
        }
    })
    .await
    .expect("child emits output before stop");
    let sessions = first.sessions;
    for session in &sessions {
        assert_eq!(session.owner, owner);
        runtime
            .stop_process_session(owner.clone(), session.id.clone())
            .await
            .expect("owner can stop");
    }
    runtime.drain_process_sessions().await;
    for session in sessions {
        let snapshot = runtime
            .read_process_session(owner.clone(), session.id, 0, 0, 65536)
            .await
            .expect("owner can read");
        assert_eq!(snapshot.session.status, ProcessSessionStatus::Stopped);
        assert!(
            snapshot
                .chunks
                .iter()
                .any(|chunk| chunk.stdout.contains("delegated-output"))
        );
    }
    provider.join().expect("provider thread");
}
