use super::*;
use colossus_contracts::DecisionOutcome;
use colossus_policy::{AllowApproval, BuiltInPolicy, SafetyKernel};
use colossus_ports::{CredentialResolutionError, CredentialResolver};
use colossus_testkit::InMemoryEventJournal;
use futures::FutureExt as _;
use rmcp::model::{DiscoverResult, ServerCapabilities};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const TASK_ID: &str = "remote-task-private-handle-91ec427";
const CREDENTIAL: &str = "task-fixture-original-token";
const ROTATED_CREDENTIAL: &str = "task-fixture-rotated-token";
const CREDENTIAL_REFERENCE: &str = "host:task-fixture";
const TEST_LIMIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct FixtureCredentials(AtomicBool);

impl CredentialResolver for FixtureCredentials {
    fn resolve(&self, reference: &str) -> Result<String, CredentialResolutionError> {
        assert_eq!(reference, CREDENTIAL_REFERENCE);
        Ok(if self.0.load(Ordering::Acquire) {
            ROTATED_CREDENTIAL
        } else {
            CREDENTIAL
        }
        .into())
    }
}

struct NoProcess;

#[async_trait]
impl EffectExecutor for NoProcess {
    async fn execute(
        &self,
        _: &EffectRequest,
        _: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        panic!("the HTTP MCP fixture must never use a subprocess");
    }
}

struct Fixture {
    listener: TcpListener,
    journal: Arc<InMemoryEventJournal>,
    credentials: Arc<FixtureCredentials>,
    executor: McpExecutor,
    gateway: EffectGateway,
    control: RunControl,
}

impl Fixture {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let config: McpConfig = serde_json::from_value(json!({
            "servers": {
                "fixture": {
                    "transport": "streamable_http",
                    "url": format!("{origin}/mcp"),
                    "protocolVersion": "2026-07-28",
                    "enableTasks": true,
                    "allowedTools": ["probe"],
                    "credentialHeaders": {
                        "Authorization": {"scheme": "Bearer", "reference": CREDENTIAL_REFERENCE}
                    }
                }
            }
        }))
        .unwrap();
        let journal = Arc::new(InMemoryEventJournal::default());
        let credentials = Arc::new(FixtureCredentials::default());
        let executor = McpExecutor::new(&config, Path::new("."), "native", Arc::new(NoProcess))
            .unwrap()
            .with_credentials(credentials.clone());
        let gateway = EffectGateway::new(
            journal.clone(),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.call", DecisionOutcome::Allow)
                    .with_action("mcp.tasks.get", DecisionOutcome::Allow)
                    .with_action("mcp.tasks.cancel", DecisionOutcome::Allow)
                    .with_sandbox("native", "mcp-task-fixture", false)
                    .with_network_destination(origin)
                    .with_limits(5_000, 1024 * 1024, 4, 64 * 1024 * 1024, 1),
            ),
            Arc::new(AllowApproval {
                approved_by: "mcp-task-fixture".into(),
            }),
            SafetyKernel::new(["mcp.invoke".into()]),
            [21_u8; 32],
        );
        Self {
            listener,
            journal,
            credentials,
            executor,
            gateway,
            control: RunControl::default(),
        }
    }

    async fn invoke(&self) -> Result<Option<McpCallOutput>, McpTaskCallError> {
        let input_schema = json!({"type": "object"});
        let operation = McpOperation::CallTool {
            server: "fixture".into(),
            tool: "probe".into(),
            description: None,
            annotations: None,
            arguments: json!({"query":"report"}),
            schema_sha256: hex::encode(Sha256::digest(serde_json::to_vec(&input_schema).unwrap())),
            input_schema: Box::new(input_schema),
            output_schema: Some(Box::new(json!({
                "type": "object",
                "properties": {
                    "count": {"type": "integer"},
                    "details": {
                        "type": "array",
                        "items": {"type": "object", "additionalProperties": {"type": "string"}}
                    }
                },
                "required": ["count"],
                "additionalProperties": false
            }))),
        };
        let actor = Actor {
            actor_type: ActorType::Model,
            id: "mcp-task-fixture".into(),
        };
        let context = ExecutionContext {
            correlation_id: "task-fixture-correlation".into(),
            session_id: Some("task-fixture-session".into()),
            run_id: Some("task-fixture-run".into()),
            ..ExecutionContext::default()
        };
        with_mcp_task_run(
            true,
            self.control.clone(),
            self.journal.clone(),
            self.executor.invoke_task_call(
                &self.gateway,
                &self.executor,
                &actor,
                &context,
                &operation,
            ),
        )
        .await
    }

    async fn drive(
        &self,
        script: impl Future<Output = ()>,
    ) -> Result<Option<McpCallOutput>, McpTaskCallError> {
        let (result, ()) = timeout(TEST_LIMIT, async { tokio::join!(self.invoke(), script) })
            .await
            .expect("the gated call and expected HTTP exchanges must both finish");
        self.assert_no_more_http();
        result
    }

    fn assert_no_more_http(&self) {
        assert!(
            self.listener.accept().now_or_never().is_none(),
            "the endpoint accepted an extra HTTP connection"
        );
    }

    async fn receive(&self, method: &str) -> (TcpStream, Value) {
        let (socket, _) = self.listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut line = String::new();
        socket.read_line(&mut line).await.unwrap();
        assert_eq!(line, "POST /mcp HTTP/1.1\r\n");
        let mut headers = BTreeMap::new();
        loop {
            line.clear();
            assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
        let mut body = vec![0; headers["content-length"].parse().unwrap()];
        socket.read_exact(&mut body).await.unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["method"], method);
        assert_eq!(headers["mcp-method"], method);
        assert_eq!(headers["mcp-protocol-version"], "2026-07-28");
        assert_eq!(headers["authorization"], format!("Bearer {CREDENTIAL}"));
        assert!(!headers.contains_key("mcp-session-id"));
        match method {
            "server/discover" => {}
            "tools/call" => {
                assert_eq!(headers["mcp-name"], "probe");
                assert_eq!(request["params"]["name"], "probe");
                assert_eq!(request["params"]["arguments"], json!({"query":"report"}));
            }
            "tasks/get" | "tasks/cancel" => {
                assert_eq!(headers["mcp-name"], TASK_ID);
                assert_eq!(request["params"]["taskId"], TASK_ID);
            }
            _ => panic!("unexpected method {method}"),
        }
        (socket.into_inner(), request["id"].clone())
    }

    async fn reply(&self, method: &str, result: Value) {
        let (socket, id) = self.receive(method).await;
        reply(socket, id, result).await;
    }

    async fn discover(&self) {
        let capabilities: ServerCapabilities = serde_json::from_value(json!({
            "tools": {},
            "extensions": {"io.modelcontextprotocol/tasks": {}}
        }))
        .unwrap();
        self.reply(
            "server/discover",
            serde_json::to_value(DiscoverResult::new(
                vec![ProtocolVersion::V_2026_07_28],
                capabilities,
            ))
            .unwrap(),
        )
        .await;
    }

    async fn start(&self) {
        self.discover().await;
        self.reply("tools/call", task("task", "working")).await;
    }

    fn receipts(&self) -> Vec<(ActorType, recovery::Receipt)> {
        let streams = self
            .journal
            .list_stream_ids(recovery::PREFIX, None, 2)
            .unwrap();
        if streams.is_empty() {
            return Vec::new();
        }
        assert_eq!(streams.len(), 1, "only one task custody stream is allowed");
        self.journal
            .read_stream_from(&streams[0], 0, 10)
            .unwrap()
            .into_iter()
            .map(|event| {
                assert_eq!(event.event_type, recovery::EVENT);
                assert_eq!(event.context.run_id.as_deref(), Some("task-fixture-run"));
                assert_eq!(
                    event.context.session_id.as_deref(),
                    Some("task-fixture-session")
                );
                let payload = self.journal.decrypt_payload(&event).unwrap();
                assert_private_values_absent(&payload.to_string());
                let receipt: recovery::Receipt = serde_json::from_value(payload).unwrap();
                assert_eq!(
                    streams[0],
                    format!("{}{}", recovery::PREFIX, receipt.call_request_id)
                );
                assert_eq!(receipt.binding.len(), 64);
                assert_eq!(receipt.call_request_hash.len(), 64);
                (event.actor.actor_type, receipt)
            })
            .collect()
    }

    fn assert_states(&self, expected: &[Status]) {
        let states = self
            .receipts()
            .into_iter()
            .map(|(_, receipt)| receipt.state)
            .collect::<Vec<_>>();
        assert_eq!(states, expected);
    }

    fn assert_releases(&self, expected: &[&str]) {
        let mut released_requests = BTreeSet::new();
        let mut actions = Vec::new();
        let events = self.journal.read_global(1, 200).unwrap();
        for event in events {
            let payload = self.journal.decrypt_payload(&event).unwrap();
            assert_private_values_absent(&payload.to_string());
            if event.event_type == "effect.release_requested.v1" {
                assert!(released_requests.insert(event.stream_id));
                actions.push(payload["action"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(actions, expected);
    }
}

async fn reply(mut socket: TcpStream, id: Value, result: Value) {
    let body = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}

fn task(result_type: &str, status: &str) -> Value {
    json!({
        "resultType": result_type,
        "taskId": TASK_ID,
        "status": status,
        "createdAt": "2026-07-28T12:00:00Z",
        "lastUpdatedAt": "2026-07-28T12:00:00Z",
        "ttlMs": 30_000,
        "pollIntervalMs": 100
    })
}

fn completed(count: Value) -> Value {
    let mut result = task("complete", "completed");
    result["result"] = json!({
        "content": [{"type": "text", "text": format!("report {TASK_ID} {CREDENTIAL}")}],
        "structuredContent": {"count": count}
    });
    result
}

fn assert_private_values_absent(text: &str) {
    for secret in [TASK_ID, CREDENTIAL, ROTATED_CREDENTIAL] {
        assert!(!text.contains(secret), "a private fixture value escaped");
    }
}

fn assert_unknown(result: Result<Option<McpCallOutput>, McpTaskCallError>) {
    let Err(error) = result else {
        panic!("the tool outcome should remain uncertain");
    };
    assert_private_values_absent(&error.to_string());
    assert!(matches!(
        error,
        McpTaskCallError::Gateway(GatewayError::OutcomeUnknown(_))
    ));
}

#[tokio::test]
async fn one_gated_call_releases_result_after_pending_poll_and_lost_connection() {
    let fixture = Fixture::new().await;
    let output = fixture
        .drive(async {
            fixture.start().await;
            fixture
                .reply("tasks/get", task("complete", "working"))
                .await;
            drop(fixture.receive("tasks/get").await);
            fixture.reply("tasks/get", completed(json!(3))).await;
        })
        .await
        .expect("the original task should finish after a lost observation")
        .expect("task-enabled operation");
    assert_eq!(output.server, "fixture");
    assert_eq!(output.tool, "probe");
    assert_eq!(output.result.structured_content, Some(json!({"count": 3})));
    let released = serde_json::to_value(output).unwrap();
    assert_eq!(
        released["result"]["content"][0]["text"],
        "report <redacted> <redacted>"
    );
    assert_private_values_absent(&released.to_string());
    fixture.assert_states(&[Status::Working, Status::Completed]);
    fixture.assert_releases(&[
        "mcp.call",
        "mcp.tasks.get",
        "mcp.tasks.get",
        "mcp.tasks.get",
    ]);
}

#[tokio::test]
async fn completed_task_with_wrong_output_schema_stays_unknown() {
    let fixture = Fixture::new().await;
    let result = fixture
        .drive(async {
            fixture.start().await;
            fixture.reply("tasks/get", completed(json!("three"))).await;
        })
        .await;
    assert_unknown(result);
    fixture.assert_states(&[Status::Working, Status::Unknown]);
    fixture.assert_releases(&["mcp.call"]);
}

#[tokio::test]
async fn completed_task_with_credential_in_nested_key_stays_unknown() {
    let fixture = Fixture::new().await;
    let result = fixture
        .drive(async {
            fixture.start().await;
            let mut result = completed(json!(3));
            result["result"]["structuredContent"]["details"] = json!([
                {(format!("detail-{CREDENTIAL}")): "report"}
            ]);
            fixture.reply("tasks/get", result).await;
        })
        .await;
    assert_unknown(result);
    fixture.assert_states(&[Status::Working, Status::Unknown]);
    fixture.assert_releases(&["mcp.call"]);
}

#[tokio::test]
async fn lost_creation_is_unknown_and_never_repeats_the_call() {
    let fixture = Fixture::new().await;
    let result = fixture
        .drive(async {
            fixture.discover().await;
            drop(fixture.receive("tools/call").await);
        })
        .await;
    assert_unknown(result);
    fixture.assert_states(&[]);
    fixture.assert_releases(&[]);
    assert!(
        fixture
            .journal
            .read_global(1, 200)
            .unwrap()
            .iter()
            .any(|event| { event.event_type == "effect.outcome_unknown.v1" }),
        "the gateway must retain the uncertain original call in the journal"
    );
}

#[tokio::test]
async fn shortened_task_ttl_prevents_another_observation() {
    let fixture = Fixture::new().await;
    let result = fixture
        .drive(async {
            fixture.start().await;
            let mut observation = task("complete", "working");
            // The first poll already occurs past this TTL measured from creation.
            observation["ttlMs"] = json!(1);
            fixture.reply("tasks/get", observation).await;
        })
        .await;
    assert_unknown(result);
    fixture.assert_states(&[Status::Working, Status::Unknown]);
    fixture.assert_releases(&["mcp.call", "mcp.tasks.get"]);
}

#[tokio::test]
async fn cancel_acknowledgement_waits_for_confirmed_terminal_observation() {
    let fixture = Fixture::new().await;
    let result = fixture
        .drive(async {
            fixture.start().await;
            let (socket, id) = fixture.receive("tasks/get").await;
            fixture.control.cancel();
            reply(socket, id, task("complete", "working")).await;
            fixture
                .reply("tasks/cancel", json!({"resultType": "complete"}))
                .await;
            let (socket, id) = fixture.receive("tasks/get").await;
            fixture.assert_states(&[Status::Working, Status::CancelRequested]);
            reply(socket, id, task("complete", "working")).await;
            fixture
                .reply("tasks/get", task("complete", "cancelled"))
                .await;
        })
        .await;
    assert!(matches!(
        result,
        Err(McpTaskCallError::Gateway(GatewayError::Execution(message)))
            if message == "Remote MCP task confirmed cancelled"
    ));
    fixture.assert_states(&[Status::Working, Status::CancelRequested, Status::Cancelled]);
    fixture.assert_releases(&[
        "mcp.call",
        "mcp.tasks.get",
        "mcp.tasks.cancel",
        "mcp.tasks.get",
        "mcp.tasks.get",
    ]);
}

#[tokio::test]
async fn rotated_credential_cannot_observe_a_task_from_the_original_credential() {
    let fixture = Fixture::new().await;
    let result = fixture
        .drive(async {
            fixture.discover().await;
            let (socket, id) = fixture.receive("tools/call").await;
            fixture.credentials.0.store(true, Ordering::Release);
            reply(socket, id, task("task", "working")).await;
        })
        .await;
    assert_unknown(result);
    fixture.assert_states(&[Status::Working, Status::Unknown]);
    fixture.assert_releases(&["mcp.call"]);
}

#[tokio::test]
async fn restart_reconciles_interrupted_custody_to_unknown_without_http() {
    let fixture = Fixture::new().await;
    let _pending_poll = timeout(TEST_LIMIT, async {
        tokio::select! {
            result = fixture.invoke() => panic!("call finished before interruption: {result:?}"),
            pending = async {
                fixture.start().await;
                fixture.receive("tasks/get").await
            } => pending,
        }
    })
    .await
    .expect("interrupt after the first observation request was sent");
    fixture.assert_states(&[Status::Working]);
    fixture.journal.require_bounded_reads();
    recover_mcp_task_receipts(fixture.journal.as_ref()).unwrap();
    recover_mcp_task_receipts(fixture.journal.as_ref()).unwrap();
    let receipts = fixture.receipts();
    let [
        (ActorType::Model, interrupted),
        (ActorType::System, recovery),
    ] = receipts.as_slice()
    else {
        panic!("expected exactly one durable recovery receipt");
    };
    assert_eq!(interrupted.state, Status::Working);
    assert_eq!(recovery.state, Status::Unknown);
    assert_eq!(interrupted.call_request_id, recovery.call_request_id);
    assert_eq!(interrupted.call_request_hash, recovery.call_request_hash);
    assert_eq!(interrupted.binding, recovery.binding);
    fixture.assert_no_more_http();
}
