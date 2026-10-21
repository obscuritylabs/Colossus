//! Background Windows acceptance of Desktop grants, native availability and discovery.
//! Uses a fresh private sidecar and credential-free loopback provider. No computer
//! operation, capture, native input, UI activation or real profile is used.
//! Operator: set `COLOSSUS_ACCEPTANCE_SIDECAR` to the matching staged sidecar, then
//! `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --no-default-features --lib native_desktop_grant_discovers_computer_tools_without_computer_effects -- --ignored`.
use super::*;
use sha2::Digest as _;
use std::{
    io::Write as _,
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

const DISCOVERY_CHILD: &str = "managed_runtime::credential_acceptance::computer_discovery_acceptance::native_computer_discovery_child";
const TOOLS: [&str; 13] = [
    "computer.windows",
    "computer.open",
    "computer.status",
    "computer.snapshot",
    "computer.screenshot",
    "computer.focus",
    "computer.move",
    "computer.click",
    "computer.drag",
    "computer.scroll",
    "computer.type",
    "computer.press",
    "computer.close",
];

#[test]
#[ignore = "requires ordinary unlocked Windows Default desktop, native credential storage, loopback and COLOSSUS_ACCEPTANCE_SIDECAR; no UI/input/capture"]
fn native_desktop_grant_discovers_computer_tools_without_computer_effects() {
    let executable = sidecar_executable();
    let root = PrivateAcceptanceRoot::new();
    let workspace = PrivateAcceptanceRoot::new();
    let _keys = RuntimeKeys(instance_id(&root.0));
    let server = Server::start();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", DISCOVERY_CHILD, "--ignored", "--nocapture"])
        .env("COLOSSUS_DISCOVERY_ROOT", &root.0)
        .env("COLOSSUS_DISCOVERY_WORKSPACE", &workspace.0)
        .env("COLOSSUS_DISCOVERY_ORIGIN", &server.origin)
        .env("COLOSSUS_ACCEPTANCE_SIDECAR", executable)
        .stdin(Stdio::null());
    // Closing this exact owned job also closes the child's SDK-owned sidecar jobs.
    let image =
        colossus_windows_native::BoundPath::open_immutable_file(&std::env::current_exe().unwrap())
            .unwrap();
    colossus_windows_native::configure_suspended_process(&mut command);
    let mut child = command.spawn().expect("spawn isolated discovery test");
    let job = colossus_windows_native::KillOnCloseJob::assign_verify_and_resume(
        &child,
        child.id(),
        image.identity(),
    )
    .unwrap_or_else(|_| {
        child.kill().expect("stop unenrolled owned test child");
        child.wait().expect("reap unenrolled owned test child");
        panic!("verify and supervise test child");
    });
    let deadline = Instant::now() + Duration::from_secs(150);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            job.terminate().expect("terminate owned timed-out test job");
            child.wait().expect("reap owned test child");
            panic!("background discovery acceptance exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(status.success(), "background discovery test child failed");
    server.finish();
    drop(job);
}

#[tokio::test]
#[ignore = "owned subprocess for native_desktop_grant_discovers_computer_tools_without_computer_effects"]
async fn native_computer_discovery_child() {
    let Some(root) = std::env::var_os("COLOSSUS_DISCOVERY_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let workspace = PathBuf::from(std::env::var_os("COLOSSUS_DISCOVERY_WORKSPACE").unwrap());
    validate_fixture_root(&root);
    validate_fixture_root(&workspace);
    let confined = ConfinedRoot::bind(root.clone()).unwrap();
    let instance = confined.prepare_directory(Path::new("instance")).unwrap();
    let home = confined
        .prepare_directory(Path::new("runtime-home"))
        .unwrap();
    let workspace = crate::desktop_settings::validate_workspace(&workspace).unwrap();
    let instance_id = instance_id(&root);
    let origin = std::env::var("COLOSSUS_DISCOVERY_ORIGIN").unwrap();
    let worker_authentication = TerminalWorkerAuthentication::random().unwrap();
    let worker_bootstrap_secret = worker_authentication.copy_secret();
    let bootstrap = managed_bootstrap(
        &workspace.path,
        workspace.identity.unwrap(),
        &configuration(&origin),
        Vec::new(),
        approval_broker_grant().unwrap(),
        worker_bootstrap_secret.as_ref(),
        &ManagedBootstrapPaths {
            ca_bundle: None,
            codex_auth: None,
            colossus_home: &home,
        },
        None,
    )
    .unwrap();
    let executable = sidecar_executable();
    let digest: [u8; 32] = Sha256::digest(std::fs::read(&executable).unwrap()).into();
    let options = SidecarOptions::new(
        InstanceId::from_uuid(instance_id),
        AppPrivateInstanceDir::new(instance).unwrap(),
        VerifiedExecutable::new(executable, Sha256Digest::from_bytes(digest)).unwrap(),
        ApiMajor::new(1).unwrap(),
    )
    .unwrap();
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .expect("verified Desktop sidecar");
    let result = std::panic::AssertUnwindSafe(async {
        let created = client
            .create_run(CreateRunRequest {
                model_profile: None,
                reasoning_effort: None,
                plugin_skill_ids: Vec::new(),
                input: vec![InputContentPart::Text(
                    "Discover +computer tools only. Do not control the computer.".into(),
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
                max_turns: 2,
                idempotency_key: IdempotencyKey::new(Uuid::now_v7().to_string()).unwrap(),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(90), async {
            loop {
                let run = client
                    .get_run(GetRunRequest {
                        run_id: created.run.run_id.clone(),
                    })
                    .await
                    .unwrap()
                    .run;
                if !matches!(run.status, RunStatus::Queued | RunStatus::Running) {
                    assert_eq!(
                        run.status,
                        RunStatus::Completed,
                        "discovery run must complete"
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("bounded discovery run");
    })
    .catch_unwind()
    .await;
    client.close().await.expect("acknowledged sidecar cleanup");
    drop(client);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

struct RuntimeKeys(Uuid);
fn instance_id(root: &Path) -> Uuid {
    let name = root.file_name().unwrap().to_string_lossy();
    Uuid::parse_str(name.strip_prefix(ROOT_PREFIX).unwrap()).unwrap()
}
impl Drop for RuntimeKeys {
    fn drop(&mut self) {
        let cleanup = std::panic::catch_unwind(|| {
            for account in [
                format!("journal-key:journal-{}", self.0),
                format!("signing-key:checkpoint-{}", self.0),
                format!("journal-anchor:journal-{}", self.0),
            ] {
                let entry = colossus_credentials::SystemCredentialEntry::new(
                    "com.obscuritylabs.colossus.managed-runtime",
                    &account,
                )
                .unwrap();
                entry.delete().expect("remove only owned runtime key");
            }
        });
        if let Err(panic) = cleanup {
            if std::thread::panicking() {
                eprintln!("generated discovery runtime key cleanup incomplete");
            } else {
                std::panic::resume_unwind(panic);
            }
        }
    }
}

fn configuration(origin: &str) -> ResolvedSpaceConfiguration {
    // Reuse only the parent's synthetic nonstreaming config constructor. Never
    // open its vault/MCP fixture; remove both credential references and MCP now.
    let mut config = resolved_configuration(origin, 0);
    config.access_profile = AccessProfileSetting::AllowAll;
    config.providers[0].base_url = format!("{origin}/v1");
    config.providers[0].credential_id = None;
    config.providers[0].timeout_ms = Some(15_000);
    config.models[0].context_window_tokens = 131_072;
    config.mcp_servers.clear();
    config.field_overrides = [("computer.enabled", true), ("browser.enabled", false)]
        .into_iter()
        .map(
            |(field_id, value)| crate::managed_configuration::FieldOverrideSetting {
                field_id: field_id.into(),
                value: json!(value),
            },
        )
        .collect();
    config
}

struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&stop);
        let task = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_mins(2);
            let mut calls = 0;
            while calls < 2 && !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("loopback accept failed: {}", error.kind()),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let body = request(&mut stream);
                let reply = completion(&body, calls);
                let body = serde_json::to_vec(&reply).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
                calls += 1;
            }
            assert_eq!(
                calls, 2,
                "both real provider requests must reach the fixture"
            );
        });
        Self {
            origin,
            stop,
            task: Some(task),
        }
    }
    fn finish(mut self) {
        self.task
            .take()
            .unwrap()
            .join()
            .expect("provider discovery evidence");
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn request(stream: &mut TcpStream) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        assert!(header.len() < 16 * 1024, "bounded HTTP headers");
        let mut byte = [0];
        read_bounded(stream, &mut byte, deadline);
        header.push(byte[0]);
    }
    let header = String::from_utf8(header).unwrap();
    assert!(header.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert!(
        !header
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "fixture must not receive provider credentials"
    );
    let length = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .unwrap()
        .1
        .trim()
        .parse::<usize>()
        .unwrap();
    assert!(
        length <= 2 * 1024 * 1024,
        "bounded synthetic provider request"
    );
    let mut body = vec![0; length];
    read_bounded(stream, &mut body, deadline);
    serde_json::from_slice(&body).unwrap()
}

fn read_bounded(stream: &mut TcpStream, mut bytes: &mut [u8], deadline: Instant) {
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "synthetic request deadline exceeded");
        stream.set_read_timeout(Some(remaining)).unwrap();
        let count = stream.read(bytes).expect("bounded synthetic request read");
        assert!(count > 0, "synthetic request ended early");
        bytes = &mut bytes[count..];
    }
}

fn completion(body: &Value, turn: usize) -> Value {
    assert_ne!(body["stream"], true);
    let definitions = body["tools"].as_array().unwrap();
    for tool in TOOLS {
        assert!(
            definitions
                .iter()
                .any(|definition| definition["function"]["name"] == tool.replace('.', "_")),
            "authenticated Desktop model request omitted {tool}"
        );
    }
    let messages = body["messages"].as_array().unwrap();
    let results = messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect::<Vec<_>>();
    let (finish, message) = if turn == 0 {
        assert!(results.is_empty());
        assert!(
            definitions
                .iter()
                .any(|tool| tool["function"]["name"] == "tool_search")
        );
        (
            "tool_calls",
            json!({"role":"assistant","content":null,"tool_calls":[{
            "id":"discovery-only","type":"function","function":{"name":"tool_search",
            "arguments":json!({"query":"+computer","max_results":50}).to_string()}}]}),
        )
    } else {
        assert_eq!(
            results.len(),
            1,
            "only the requested discovery call may run"
        );
        assert_eq!(results[0]["tool_call_id"], "discovery-only");
        let observation: Value =
            serde_json::from_str(results[0]["content"].as_str().unwrap()).unwrap();
        let metadata = &observation["_colossusToolObservation"];
        assert_eq!(metadata["toolName"], "tool.search");
        assert_eq!(metadata["callId"], "discovery-only");
        assert_eq!(metadata["exitCode"], 0);
        assert_eq!(metadata["truncated"], false);
        let result = &observation["data"];
        assert_eq!(result["query"], "+computer");
        assert_eq!(result["truncated"], false);
        let actual = result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .filter(|name| name.starts_with("computer."))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual,
            BTreeSet::from(TOOLS),
            "released discovery result must contain all 13 tools"
        );
        (
            "stop",
            json!({"role":"assistant","content":"Computer discovery passed without desktop effects."}),
        )
    };
    json!({"id":"synthetic-discovery","object":"chat.completion","created":0,"model":"synthetic-discovery",
        "choices":[{"index":0,"message":message,"finish_reason":finish}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
}
