//! Native password custody, model selection, live/resumed SSE and approval against
//! the real controller, mTLS connector and runtime. Only the loopback provider is synthetic.
use super::{TestKeys, isolated, tls_files};
use crate::{
    config::{Config, LocalAuthConfig},
    http::now,
    server::CloudServer,
};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
};
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, RequestId, scopes,
};
use colossus_api_runtime::{PublicInteractionRouter, RuntimeAgentRunApi};
use colossus_cloud::{
    CloudCaller, CloudPermission, CloudProject, CloudRepository, CloudTask, CloudUser, Enrollment,
    LocalCredential, LoginIdentityMetadata, ProjectRole, UserAccount,
    storage::{CloudStore, MemoryCloudStore},
};
use colossus_connector::{ConnectorResources, ConnectorStatus, EnrollmentStore, RuntimeConnector};
use colossus_control_plane_client::{ControlPlaneClient, Operation, StreamEvent, WatchScope};
use colossus_credentials::PlatformCredentialVault;
use colossus_home::ColossusHome;
use colossus_policy::DenyApproval;
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions};
use colossus_sdk::{
    ConfiguredModelProfile, ContextBoundAgentRunClient, InteractionContent, InteractionStatus,
    RunStatus,
};
use futures::StreamExt as _;
use serde_json::{Value, json};
use sha2::Digest as _;
use std::{
    collections::BTreeSet,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc, watch},
    task::{AbortHandle, JoinHandle},
};
use zeroize::Zeroizing;

const PROJECT: &str = "native-project";
const PASSWORD: &str = "disposable native acceptance password";
const LIVE_TEXT: &str = "Native selected-model output is live.\n";
const REPLAY_TEXT: &str = "Native replay marker emitted while disconnected.\n";

#[derive(Default)]
struct Tasks(Vec<AbortHandle>);
impl Tasks {
    fn track<T>(&mut self, task: &JoinHandle<T>) {
        self.0.push(task.abort_handle());
    }
}
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

struct Provider {
    requests: Mutex<Vec<Value>>,
    begin: Arc<Semaphore>,
    replay: Arc<Semaphore>,
    finish: Arc<Semaphore>,
}
fn chunk(delta: Value, finish_reason: Value) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"native-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish_reason}]}),
    )
}
async fn gate(semaphore: Arc<Semaphore>) {
    tokio::time::timeout(Duration::from_secs(15), semaphore.acquire())
        .await
        .expect("native acceptance must release its provider gate")
        .unwrap()
        .forget();
}
async fn provider(State(state): State<Arc<Provider>>, Json(request): Json<Value>) -> Response {
    let turn = {
        let mut requests = state.requests.lock().unwrap();
        requests.push(request);
        requests.len()
    };
    if turn == 1 {
        let begin = state.begin.clone();
        let replay = state.replay.clone();
        let finish = state.finish.clone();
        let output = futures::stream::once(async move {
            gate(begin).await;
            // More than one release batch: this cannot depend on a later tool
            // or completion event flushing a short pending text delta.
            Ok::<_, Infallible>(chunk(json!({"content":LIVE_TEXT.repeat(256)}), Value::Null))
        });
        let retained = futures::stream::once(async move {
            gate(replay).await;
            // Force release while the client has no active subscription and the
            // later tool/completion gate remains closed.
            Ok::<_, Infallible>(chunk(
                json!({"content":REPLAY_TEXT.repeat(256)}),
                Value::Null,
            ))
        });
        let tool = futures::stream::once(async move {
            gate(finish).await;
            Ok::<_, Infallible>(format!(
                "{}data: [DONE]\n\n",
                chunk(
                    json!({"tool_calls":[{"index":0,"id":"native-write","type":"function","function":{"name":"filesystem_write","arguments":json!({"path":"approved.txt","content":"native-approved","mode":"create"}).to_string()}}]}),
                    json!("tool_calls"),
                )
            ))
        });
        return (
            [("content-type", "text/event-stream")],
            Body::from_stream(output.chain(retained).chain(tool)),
        )
            .into_response();
    }
    (
        [("content-type", "text/event-stream")],
        format!(
            "{}data: [DONE]\n\n",
            chunk(
                json!({"content":"Native approval completed."}),
                json!("stop")
            ),
        ),
    )
        .into_response()
}

async fn provision(store: Arc<dyn CloudStore>) -> CloudRepository {
    let repository = CloudRepository::new(store).unwrap();
    let password_hash = crate::auth::local::hash_password(
        Arc::new(Semaphore::new(1)),
        Zeroizing::new(PASSWORD.into()),
    )
    .await
    .unwrap();
    for (id, admin) in [("native-operator", true), ("native-approver", false)] {
        repository
            .create_account(
                "native-acceptance-provisioning",
                UserAccount {
                    user: CloudUser {
                        id: id.into(),
                        display_name: id.into(),
                        email: None,
                        active: true,
                        is_admin: admin,
                        revision: 1,
                        created_at: "2026-10-16T00:00:00Z".into(),
                        updated_at: "2026-10-16T00:00:00Z".into(),
                        identities: vec![LoginIdentityMetadata {
                            kind: "local".into(),
                            label: "Local account".into(),
                            username: Some(id.into()),
                            issuer: None,
                            subject: None,
                        }],
                    },
                    security_epoch: 1,
                },
                None,
                Some(LocalCredential {
                    user_id: id.into(),
                    username: id.into(),
                    password_hash: password_hash.clone(),
                }),
            )
            .await
            .unwrap();
    }
    let operator = repository.account("native-operator").await.unwrap();
    repository
        .save_project(
            &operator.user,
            CloudProject {
                id: PROJECT.into(),
                name: "Native acceptance".into(),
                description: String::new(),
                parent_project_id: None,
                archived: false,
                revision: 0,
                created_at: String::new(),
                updated_at: String::new(),
            },
        )
        .await
        .unwrap();
    let caller = CloudCaller::new(
        operator.user.id,
        PROJECT.into(),
        BTreeSet::from([CloudPermission::Administer]),
    )
    .unwrap();
    for (id, role) in [
        ("native-operator", ProjectRole::ProjectAdmin),
        ("native-approver", ProjectRole::Approver),
    ] {
        repository
            .save_membership(&caller, id, role, 0)
            .await
            .unwrap();
    }
    repository
}

async fn task(client: &ControlPlaneClient, id: &str) -> CloudTask {
    let value = client
        .request(Operation::Task {
            project: PROJECT.into(),
            task: id.into(),
        })
        .await
        .unwrap();
    serde_json::from_value(value["task"].clone()).unwrap()
}
async fn wait_task(
    client: &ControlPlaneClient,
    id: &str,
    predicate: impl Fn(&CloudTask) -> bool,
) -> CloudTask {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let value = task(client, id).await;
            if predicate(&value) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("native task must reach the expected state")
}
async fn update(
    receiver: &mut mpsc::Receiver<StreamEvent>,
    after: &mut u64,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match receiver.recv().await.expect("native watch remains open") {
                StreamEvent::RunUpdate { update, cursor } => {
                    let cursor: u64 = cursor.parse().unwrap();
                    assert!(cursor > *after, "SSE resumes at an exclusive cursor");
                    assert_eq!(update["sequence"].as_u64(), Some(cursor));
                    *after = cursor;
                    if predicate(&update) {
                        return update;
                    }
                }
                StreamEvent::Error { code, message } => panic!("{code}: {message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("native SSE must release the expected update")
}
async fn connected(receiver: &mut mpsc::Receiver<StreamEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match receiver.recv().await.expect("native watch remains open") {
                StreamEvent::Connection { connected: true } => return,
                StreamEvent::Error { code, message } => panic!("{code}: {message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("native SSE must connect before provider output starts");
}

#[tokio::test]
async fn local_native_client_selects_real_model_streams_resumes_and_approves() {
    if isolated::run(
        "auth::tests::transport::native::local_native_client_selects_real_model_streams_resumes_and_approves",
        false,
    )
    .await
    {
        return;
    }
    let directory = isolated::directory();
    let root = directory.path().canonicalize().unwrap();
    let mut tasks = Tasks::default();
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", http.local_addr().unwrap());
    let mut config = Config {
        http_bind: http.local_addr().unwrap(),
        grpc_bind: grpc.local_addr().unwrap(),
        public_origin: origin.clone(),
        grpc_endpoint: format!("https://localhost:{}", grpc.local_addr().unwrap().port()),
        ca_certificate: root.join("ca.pem"),
        ca_key: root.join("ca-key.pem"),
        server_certificate: root.join("server.pem"),
        server_key: root.join("server-key.pem"),
        oidc: None,
        local_auth: Some(LocalAuthConfig {
            session_seconds: 3600,
        }),
        bootstrap_admin: None,
        classification: None,
        memberships: vec![],
        web_root: root.join("web"),
        database: colossus_cloud_postgres::CloudDatabaseConfig {
            connection_variable: "UNUSED_NATIVE_ACCEPTANCE_DATABASE".into(),
            schema: "native_acceptance".into(),
            tls: colossus_cloud_postgres::CloudDatabaseTls::Disabled,
            max_connections: 4,
            connection_timeout_ms: 5000,
            statement_timeout_ms: 5000,
        },
        auth_key_variable: None,
        maintenance: Default::default(),
        local_development: true,
        chat: None,
    };
    tls_files(&root, &mut config);
    let store: Arc<dyn CloudStore> = Arc::new(MemoryCloudStore::default());
    let repository = provision(store.clone()).await;
    let token = hex::encode([13u8; 32]);
    repository
        .invite(
            &CloudCaller::new(
                "native-operator".into(),
                PROJECT.into(),
                BTreeSet::from([CloudPermission::Administer]),
            )
            .unwrap(),
            Enrollment {
                owner_user_id: None,
                token_hash: hex::encode(sha2::Sha256::digest(token.as_bytes())),
                project_id: PROJECT.into(),
                node_id: "native-node".into(),
                label: "Native acceptance runtime".into(),
                roles: BTreeSet::from(["primary".into()]),
                expires_at: now() + 600,
                redeemed_certificate: None,
                redeemed_csr: None,
                certificate_pem: None,
            },
            now(),
        )
        .await
        .unwrap();
    let server = CloudServer::with_store(config, store).await.unwrap();
    let (stop, shutdown) = watch::channel(false);
    let host = tokio::spawn(server.serve_listeners(http, grpc, shutdown));
    tasks.track(&host);

    let operator = ControlPlaneClient::default();
    assert_eq!(
        operator.connect(&origin).await.unwrap()["local_enabled"],
        true
    );
    assert_eq!(
        operator
            .sign_in("native-operator".into(), PASSWORD.into())
            .await
            .unwrap()["user"]["id"],
        "native-operator"
    );
    let approver = ControlPlaneClient::default();
    approver.connect(&origin).await.unwrap();
    approver
        .sign_in("native-approver".into(), PASSWORD.into())
        .await
        .unwrap();

    let home = ColossusHome::ensure_at(root.join("native")).unwrap();
    let vault = Arc::new(
        PlatformCredentialVault::with_key_store(
            home.confined_root().clone(),
            "native-client-acceptance",
            Arc::new(TestKeys::default()),
        )
        .unwrap(),
    );
    let enrollment = EnrollmentStore::new(vault, "native-node").unwrap();
    let connection = enrollment
        .enroll(
            format!("{origin}/api/enroll"),
            Zeroizing::new(token),
            "native-runtime".into(),
            vec![],
            true,
        )
        .await
        .unwrap();
    let (_, key) = enrollment.load().unwrap().unwrap();
    let model_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_origin = format!("http://{}", model_listener.local_addr().unwrap());
    let provider_state = Arc::new(Provider {
        requests: Mutex::new(Vec::new()),
        begin: Arc::new(Semaphore::new(0)),
        replay: Arc::new(Semaphore::new(0)),
        finish: Arc::new(Semaphore::new(0)),
    });
    let router = Router::new()
        .route("/v1/chat/completions", post(provider))
        .with_state(provider_state.clone());
    let model = tokio::spawn(async move { axum::serve(model_listener, router).await.unwrap() });
    tasks.track(&model);
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let mut runtime_config = RuntimeConfig::offline_template(workspace.join("runtime.redb"));
    runtime_config.workflows.repository = workspace.join("workflows");
    runtime_config.workflows.user = workspace.join("workflows");
    std::fs::create_dir(&runtime_config.workflows.repository).unwrap();
    runtime_config.access = serde_json::from_value(json!({"profile":"pinned","tools":{"include":["filesystem.write"]},"actions":{"allow":["provider.openai.chat"],"requireApproval":["filesystem.write"]}})).unwrap();
    runtime_config.providers = serde_json::from_value(json!({"profiles":{"fixture":{"kind":"open_ai_compatible","baseUrl":format!("{model_origin}/v1"),"timeoutMs":30000}}})).unwrap();
    runtime_config.models = serde_json::from_value(json!({"profiles":{
        "default":{"providerProfile":"fixture","model":"native-default","contextWindowTokens":32768,"maxOutputTokens":4096,"reasoningEffort":"low","capabilities":{"toolCalls":true,"streaming":true}},
        "selected":{"providerProfile":"fixture","model":"native-selected","contextWindowTokens":32768,"maxOutputTokens":4096,"reasoningEffort":"medium","capabilities":{"toolCalls":true,"streaming":true}}
    },"roles":{"primary":"default"}})).unwrap();
    runtime_config.sandbox.network_destinations = vec![model_origin];
    runtime_config.sandbox.filesystem =
        serde_json::from_value(json!([{"root":workspace,"mode":"write"}])).unwrap();
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let runtime = Arc::new(
        Runtime::open_with_options(
            &runtime_config,
            interactions.clone(),
            Some(interactions.clone()),
            RuntimeOpenOptions::for_workspace(&workspace).unwrap(),
        )
        .unwrap(),
    );
    let default_profile = runtime.provider_route("primary").unwrap().model_profile;
    let model_profiles = runtime
        .model_profiles()
        .into_iter()
        .map(|route| ConfiguredModelProfile {
            default_roles: if route.model_profile == default_profile {
                vec!["primary".into()]
            } else {
                vec![]
            },
            profile: route.model_profile,
            model: route.model,
            provider: route.provider,
            provider_profile: route.provider_profile,
            reasoning_effort: route.reasoning_effort.map(|effort| effort.as_str().into()),
        })
        .collect();
    let principal = ApplicationPrincipal::authenticated(
        "app:native-client-acceptance",
        "native-acceptance-credential",
        ApplicationKind::Enrolled,
        [
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
            scopes::APPROVALS_RESPOND,
        ]
        .into_iter()
        .map(|scope| ApiScope::new(scope).unwrap()),
        ["primary".into()],
        ["filesystem.write".into()],
    )
    .unwrap();
    let runs = Arc::new(ContextBoundAgentRunClient::new(
        Arc::new(RuntimeAgentRunApi::new(
            runtime.clone(),
            interactions,
            "primary",
            "Perform the native acceptance task.",
        )),
        CallerContext::authenticated(
            principal,
            RequestId::new("native-acceptance-request").unwrap(),
        ),
    ));
    let (disconnect, close) = watch::channel(false);
    let (status, mut online) = watch::channel(ConnectorStatus::Connecting);
    let connector = RuntimeConnector::new(connection.clone(), key, runs)
        .unwrap()
        .with_resources(ConnectorResources {
            capabilities: vec!["agent_runs.model_selection".into()],
            model_profiles,
            ..Default::default()
        })
        .with_enrollment_store(enrollment);
    let connected_runtime = tokio::spawn(connector.run(close, status));
    tasks.track(&connected_runtime);
    tokio::time::timeout(Duration::from_secs(5), async {
        while *online.borrow() != ConnectorStatus::Connected {
            online.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let lease = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let fleet = operator
                .request(Operation::Fleet {
                    project: PROJECT.into(),
                    after: None,
                })
                .await
                .unwrap();
            if let Some(node) = fleet["nodes"].as_array().unwrap().iter().find(|node| {
                node["node"]["node_id"] == connection.node_id && node["presence"]["ready"] == true
            }) {
                return node["presence"]["connection_id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("enrolled runtime becomes ready");
    let context = operator
        .request(Operation::RuntimeContext {
            project: PROJECT.into(),
            node: connection.node_id.clone(),
            connection_id: lease.clone(),
        })
        .await
        .unwrap();
    assert!(
        context["value"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "agent_runs.model_selection")
    );
    let catalog = operator
        .request(Operation::ModelCatalog {
            project: PROJECT.into(),
            node: connection.node_id.clone(),
            connection_id: lease,
        })
        .await
        .unwrap();
    let choice = catalog["value"]["model_profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["profile"] == "selected")
        .unwrap();
    assert_eq!(
        catalog["value"]["model_profiles"].as_array().unwrap().len(),
        2
    );
    assert_eq!(choice["model"], "native-selected");
    assert_eq!(choice["reasoning_effort"], "medium");
    let created = operator.request(Operation::CreateThread {
        project: PROJECT.into(), node: connection.node_id.clone(),
        request: json!({"model_profile":choice["profile"],"reasoning_effort":"high","plugin_skill_ids":[],"input":[{"text":"Perform the isolated file write after approval."}],"session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_depth":null,"research_sources":[],"plan_action":null,"branch":null,"max_turns":8,"idempotency_key":"native-model-once"}),
    }).await.unwrap();
    let id = created["task"]["task_id"].as_str().unwrap();
    let (sender, mut events) = mpsc::channel(128);
    let watch = operator
        .watch(
            WatchScope {
                project: PROJECT.into(),
                task: Some(id.into()),
                thread: None,
                after: None,
            },
            move |event| sender.try_send(event).is_ok(),
        )
        .await
        .unwrap();
    connected(&mut events).await;
    provider_state.begin.add_permits(1);
    let mut cursor = 0;
    let live = update(&mut events, &mut cursor, |update| {
        update["update"]["output_delta"]
            .as_str()
            .is_some_and(|text| text.contains(LIVE_TEXT.trim()))
    })
    .await;
    let running = task(&operator, id).await;
    assert_eq!(
        running.snapshot.as_ref().unwrap().run.status,
        RunStatus::Running
    );
    assert_eq!(live["run_id"].as_str(), running.run_id.as_deref());
    assert!(!workspace.join("approved.txt").exists());
    assert_eq!(provider_state.requests.lock().unwrap().len(), 1);

    operator.unwatch(watch).await;
    drop(events);
    provider_state.replay.add_permits(1);
    let retained = tokio::time::timeout(Duration::from_secs(15), async {
        let mut after = cursor;
        loop {
            let page = operator
                .request(Operation::TaskUpdates {
                    project: PROJECT.into(),
                    task: id.into(),
                    after,
                })
                .await
                .unwrap();
            for update in page["updates"].as_array().unwrap() {
                let sequence = update["sequence"].as_u64().unwrap();
                assert!(sequence > after);
                after = sequence;
                if update["update"]["output_delta"]
                    .as_str()
                    .is_some_and(|text| text.contains(REPLAY_TEXT.trim()))
                {
                    return update.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("disconnected output must be retained before reconnecting");
    let (sender, mut events) = mpsc::channel(128);
    let watch = operator
        .watch(
            WatchScope {
                project: PROJECT.into(),
                task: Some(id.into()),
                thread: None,
                after: Some(cursor.to_string()),
            },
            move |event| sender.try_send(event).is_ok(),
        )
        .await
        .unwrap();
    connected(&mut events).await;
    let replayed = update(&mut events, &mut cursor, |update| {
        update["sequence"] == retained["sequence"]
    })
    .await;
    assert_eq!(
        replayed, retained,
        "resume must replay the exact retained update"
    );
    provider_state.finish.add_permits(1);
    let waiting = wait_task(&approver, id, |task| {
        task.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .pending_interactions
                .iter()
                .any(|interaction| interaction.status == InteractionStatus::Pending)
        })
    })
    .await;
    assert!(
        !workspace.join("approved.txt").exists(),
        "policy-bound write waits for native human approval"
    );
    let interaction = waiting
        .snapshot
        .unwrap()
        .pending_interactions
        .into_iter()
        .find(|interaction| interaction.status == InteractionStatus::Pending)
        .unwrap();
    let InteractionContent::Approval(approval) = interaction.content else {
        panic!("expected a file-write approval");
    };
    let receipt = approver.request(Operation::Respond {
        project: PROJECT.into(), task: id.into(), mutation_id: "native-approval-once".into(),
        request: json!({"run_id":interaction.run_id,"interaction_id":interaction.interaction_id,"etag":interaction.etag,"response":{"approval":{"approved":true,"request_hash":approval.request_hash}},"idempotency_key":"native-approval-once"}),
    }).await.unwrap();
    let command = receipt["command"]["command_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let receipt = approver
                .request(Operation::Command {
                    project: PROJECT.into(),
                    task: id.into(),
                    command: command.into(),
                })
                .await
                .unwrap();
            if !receipt["command"]["reply"].is_null() {
                assert_eq!(receipt["command"]["reply"]["kind"], "responded");
                return;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let terminal = update(&mut events, &mut cursor, |update| {
        update["update"].get("result").is_some()
    })
    .await;
    assert_eq!(terminal["update"]["result"]["model_profile"], "selected");
    assert!(
        terminal["update"]["result"]["output"]
            .as_str()
            .unwrap()
            .contains("Native approval completed.")
    );
    let complete = wait_task(&operator, id, |task| {
        task.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.run.status == RunStatus::Completed
                && task.last_sequence == snapshot.run.last_sequence
        })
    })
    .await;
    assert_eq!(complete.run_id, running.run_id);
    assert_eq!(
        std::fs::read_to_string(workspace.join("approved.txt")).unwrap(),
        "native-approved"
    );
    let requests = provider_state.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request["model"], "native-selected");
        assert_eq!(request["reasoning_effort"], "high");
    }

    operator.unwatch(watch).await;
    operator.sign_out().await.unwrap();
    approver.sign_out().await.unwrap();
    disconnect.send_replace(true);
    tokio::time::timeout(Duration::from_secs(5), connected_runtime)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(5), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    model.abort();
    assert!(model.await.unwrap_err().is_cancelled());
}
