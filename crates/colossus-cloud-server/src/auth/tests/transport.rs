//! Real OIDC, HTTP/SSE, encrypted enrollment, mTLS, connector, runtime, prompt,
//! approval, cancellation, rotation and host-restart acceptance. No model credentials.
use super::*;
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, RequestId, scopes,
};
use colossus_api_runtime::{PublicInteractionRouter, RuntimeAgentRunApi};
use colossus_cloud::CloudTask;
use colossus_connector::{ConnectorStatus, EnrollmentStore, RuntimeConnector};
use colossus_contracts::VaultRecord;
use colossus_credentials::{PlatformCredentialVault, PlatformKeyStore};
use colossus_home::ColossusHome;
use colossus_policy::DenyApproval;
use colossus_ports::{CredentialError, CredentialKey, CredentialVault};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions};
use colossus_sdk::{ContextBoundAgentRunClient, InteractionContent, InteractionStatus, RunStatus};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::watch;
use zeroize::Zeroizing;

mod managed;
mod resources;

#[derive(Default)]
struct TestKeys(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);
impl PlatformKeyStore for TestKeys {
    fn read(
        &self,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, colossus_ports::CredentialError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn write(&self, account: &str, bytes: &[u8]) -> Result<(), colossus_ports::CredentialError> {
        self.0
            .lock()
            .unwrap()
            .insert(account.into(), Zeroizing::new(bytes.to_vec()));
        Ok(())
    }
}
struct FaultVault {
    inner: PlatformCredentialVault,
    fail_write: AtomicUsize,
}
impl CredentialVault for FaultVault {
    fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        self.inner.read(key)
    }
    fn write(&self, key: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
        if self
            .fail_write
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            == Ok(1)
        {
            return Err(CredentialError::Io);
        }
        self.inner.write(key, record)
    }
    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
        self.inner.delete(key)
    }
}
fn tls_files(root: &Path, config: &mut Config) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca = params.self_signed(&key).unwrap();
    let issuer = Issuer::new(params, key);
    let server_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server = params.signed_by(&server_key, &issuer).unwrap();
    for (name, value) in [
        ("ca.pem", ca.pem()),
        ("ca-key.pem", issuer.key().serialize_pem()),
        ("server.pem", server.pem()),
        ("server-key.pem", server_key.serialize_pem()),
    ] {
        std::fs::write(root.join(name), value).unwrap();
    }
    config.ca_certificate = root.join("ca.pem");
    config.ca_key = root.join("ca-key.pem");
    config.server_certificate = root.join("server.pem");
    config.server_key = root.join("server-key.pem");
}
async fn sign_in(client: &reqwest::Client, origin: &str, issuer: &IssuerState) -> HeaderMap {
    let login = client
        .get(format!("{origin}/auth/login"))
        .send()
        .await
        .unwrap();
    let url = url::Url::parse(login.headers()["location"].to_str().unwrap()).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    *issuer.challenge.lock().unwrap() = query["code_challenge"].clone();
    *issuer.nonce.lock().unwrap() = query["nonce"].clone();
    let callback = client
        .get(format!("{origin}/auth/callback"))
        .query(&[("state", query["state"].as_str()), ("code", "fixture-code")])
        .header(
            "cookie",
            login.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), 303);
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        HeaderValue::from_str(
            callback.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap(),
        )
        .unwrap(),
    );
    headers.insert("origin", HeaderValue::from_str(origin).unwrap());
    headers.insert("x-colossus-csrf", HeaderValue::from_static("1"));
    headers
}
async fn post(
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    path: &str,
    value: Value,
) -> Value {
    let response = client
        .post(format!("{origin}{path}"))
        .headers(headers.clone())
        .json(&value)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    response.json().await.unwrap()
}
async fn wait_task(
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    id: &str,
    predicate: impl Fn(&CloudTask) -> bool,
) -> CloudTask {
    let mut last = None;
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let response: Value = client
                .get(format!("{origin}/api/projects/project-a/tasks/{id}"))
                .headers(headers.clone())
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let task: CloudTask = serde_json::from_value(response["task"].clone()).unwrap();
            last = Some(task.clone());
            if predicate(&task) {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    result.unwrap_or_else(|_| panic!("cloud task deadline: {last:?}"))
}
async fn provider(
    axum::extract::State(turn): axum::extract::State<Arc<AtomicUsize>>,
) -> impl axum::response::IntoResponse {
    let turn = turn.fetch_add(1, Ordering::SeqCst);
    let choice = match turn {
        1 => {
            json!({"delta":{"tool_calls":[{"index":0,"id":"write","type":"function","function":{"name":"filesystem_write","arguments":json!({"path":"approved.txt","content":"cloud-approved","mode":"create"}).to_string()}}]},"finish_reason":"tool_calls"})
        }
        2 => json!({"delta":{"content":"Cloud approval completed."},"finish_reason":"stop"}),
        _ => {
            json!({"delta":{"tool_calls":[{"index":0,"id":format!("ask-{turn}"),"type":"function","function":{"name":"user_ask","arguments":json!({"question":"Continue the isolated acceptance task?","choices":["Continue"],"allow_free_form":true}).to_string()}}]},"finish_reason":"tool_calls"})
        }
    };
    let event = json!({"id":"cloud-fixture","choices":[{"index":0,"delta":choice["delta"],"finish_reason":choice["finish_reason"]}]});
    (
        [("content-type", "text/event-stream")],
        format!("data: {event}\n\ndata: [DONE]\n\n"),
    )
}

#[tokio::test]
async fn oidc_to_runtime_prompt_approval_cancel_rotate_and_recover() {
    acceptance(false).await;
}

#[tokio::test]
#[ignore = "operator-owned: local PostgreSQL plus COLOSSUS_CLOUD_TEST_DATABASE"]
async fn postgres_oidc_to_runtime_and_recover() {
    acceptance(true).await;
}

async fn acceptance(postgres: bool) {
    let (auth, issuer, oidc) = fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let home = ColossusHome::ensure_at(root.join("native")).unwrap();
    let browser = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = auth.config.clone();
    config.http_bind = browser.local_addr().unwrap();
    config.grpc_bind = grpc.local_addr().unwrap();
    config.public_origin = format!("http://{}", config.http_bind);
    config.grpc_endpoint = format!("https://localhost:{}", config.grpc_bind.port());
    config.database.schema = format!("cloud_e2e_{}", uuid::Uuid::now_v7().simple());
    config.database.tls = colossus_cloud_postgres::CloudDatabaseTls::Disabled;
    config.auth_key_variable = None;
    let cloud_store: Arc<dyn colossus_cloud::storage::CloudStore> = if postgres {
        Arc::new(
            colossus_cloud_postgres::CloudPostgresStore::open(
                config.database.clone(),
                &colossus_network::AdditionalRootCertificates::default(),
            )
            .await
            .unwrap(),
        )
    } else {
        Arc::new(colossus_cloud::storage::MemoryCloudStore::default())
    };
    config.memberships[0].permissions = BTreeSet::from([
        CloudPermission::Read,
        CloudPermission::Execute,
        CloudPermission::Control,
        CloudPermission::Approve,
        CloudPermission::Administer,
    ]);
    tls_files(&root, &mut config);
    drop(auth);
    let origin = config.public_origin.clone();
    let server = crate::server::CloudServer::with_store(config.clone(), cloud_store.clone())
        .await
        .unwrap();
    let (stop, shutdown) = watch::channel(false);
    let host = tokio::spawn(server.serve_listeners(browser, grpc, shutdown));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let failed_sign_in = client
        .get(format!("{origin}/auth/callback"))
        .query(&[("state", "untrusted-state"), ("code", "untrusted-code")])
        .send()
        .await
        .unwrap();
    assert_eq!(failed_sign_in.status(), 403);
    assert!(
        failed_sign_in.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(!failed_sign_in.headers().contains_key("set-cookie"));
    let failure_page = failed_sign_in.text().await.unwrap();
    assert!(failure_page.contains("Return to Colossus"));
    assert!(!failure_page.contains("untrusted-state"));
    assert!(!failure_page.contains("untrusted-code"));
    let cancelled_sign_in = client
        .get(format!("{origin}/auth/callback"))
        .query(&[
            ("error", "access_denied"),
            ("error_description", "untrusted-description"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled_sign_in.status(), 403);
    assert!(!cancelled_sign_in.headers().contains_key("set-cookie"));
    let cancelled_page = cancelled_sign_in.text().await.unwrap();
    assert!(cancelled_page.contains("Return to Colossus"));
    assert!(!cancelled_page.contains("untrusted-description"));
    let headers = sign_in(&client, &origin, &issuer).await;
    assert_eq!(
        client
            .get(format!("{origin}/api/projects/foreign/tasks"))
            .headers(headers.clone())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let mut no_csrf = headers.clone();
    no_csrf.remove("x-colossus-csrf");
    assert_eq!(
        client
            .post(format!("{origin}/api/projects/project-a/invitations"))
            .headers(no_csrf)
            .json(&json!({"label":"test","roles":["primary"]}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let invite = post(
        &client,
        &origin,
        &headers,
        "/api/projects/project-a/invitations",
        json!({"label":"Acceptance runtime","roles":["primary"]}),
    )
    .await;
    let vault = Arc::new(FaultVault {
        inner: PlatformCredentialVault::with_key_store(
            home.confined_root().clone(),
            "cloud-acceptance",
            Arc::new(TestKeys::default()),
        )
        .unwrap(),
        fail_write: AtomicUsize::new(0),
    });
    let enrollment = EnrollmentStore::new(vault.clone(), "node-a").unwrap();
    let connection = enrollment
        .enroll(
            invite["enrollment_url"].as_str().unwrap().into(),
            Zeroizing::new(invite["token"].as_str().unwrap().into()),
            "runtime-a".into(),
            vec![],
            true,
        )
        .await
        .unwrap();
    let (_, key) = enrollment.load().unwrap().unwrap();

    let model_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_origin = format!("http://{}", model_listener.local_addr().unwrap());
    let turns = Arc::new(AtomicUsize::new(0));
    let model_router = Router::new()
        .route("/v1/chat/completions", axum::routing::post(provider))
        .with_state(turns.clone());
    let model =
        tokio::spawn(async move { axum::serve(model_listener, model_router).await.unwrap() });
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let mut runtime_config = RuntimeConfig::offline_template(workspace.join("runtime.redb"));
    runtime_config.workflows.repository = workspace.join("workflows");
    runtime_config.workflows.user = workspace.join("workflows");
    std::fs::create_dir(&runtime_config.workflows.repository).unwrap();
    runtime_config.access=serde_json::from_value(json!({"profile":"pinned","tools":{"include":["user.ask","filesystem.write"]},"actions":{"allow":["provider.openai.chat"],"requireApproval":["filesystem.write"]}})).unwrap();
    runtime_config.providers=serde_json::from_value(json!({"profiles":{"fixture":{"kind":"open_ai_compatible","baseUrl":format!("{model_origin}/v1"),"timeoutMs":5000}}})).unwrap();
    runtime_config.models=serde_json::from_value(json!({"profiles":{"fixture":{"providerProfile":"fixture","model":"fixture","contextWindowTokens":32768,"maxOutputTokens":4096,"capabilities":{"toolCalls":true,"streaming":true}}},"roles":{"primary":"fixture"}})).unwrap();
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
    let api = Arc::new(RuntimeAgentRunApi::new(
        runtime.clone(),
        interactions,
        "primary",
        "Perform the acceptance task.",
    ));
    let principal = ApplicationPrincipal::authenticated(
        "app:cloud-acceptance",
        "credential-acceptance",
        ApplicationKind::Enrolled,
        [
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
            scopes::PROMPTS_RESPOND,
            scopes::APPROVALS_RESPOND,
            scopes::WORKFLOWS_READ,
            scopes::WORKFLOWS_REGISTER,
            scopes::SCHEDULES_READ,
            scopes::SCHEDULES_CREATE,
            scopes::SCHEDULES_CONTROL,
            scopes::WORKFLOW_RUNS_READ,
            scopes::WORKFLOW_RUNS_START,
        ]
        .into_iter()
        .map(|scope| ApiScope::new(scope).unwrap()),
        ["primary".into()],
        ["user.ask".into(), "filesystem.write".into()],
    )
    .unwrap();
    let resource_caller =
        CallerContext::authenticated(principal, RequestId::new("cloud-request").unwrap());
    let workflow_resources = Arc::new(colossus_sdk::ContextBoundWorkflowClient::new(
        Arc::new(colossus_api_runtime::RuntimeWorkflowApi::new(
            runtime.clone(),
        )),
        resource_caller.clone(),
    ));
    let runs = Arc::new(ContextBoundAgentRunClient::new(api, resource_caller));
    let (disconnect, close) = watch::channel(false);
    let (status, mut online) = watch::channel(ConnectorStatus::Connecting);
    let connector = RuntimeConnector::new(connection.clone(), key, runs.clone())
        .unwrap()
        .with_resources(colossus_connector::ConnectorResources {
            workflows: Some(workflow_resources),
            plugins: None,
            capabilities: vec![
                "workflows.read".into(),
                "workflows.register".into(),
                "schedules.read".into(),
                "schedules.create".into(),
                "schedules.control".into(),
                "schedules.delete".into(),
                "workflow_runs.read".into(),
                "workflow_runs.start".into(),
                "workflow_runs.history".into(),
            ],
        })
        .with_enrollment_store(enrollment.clone());
    let connected = tokio::spawn(connector.run(close, status));
    tokio::time::timeout(Duration::from_secs(5), async {
        while *online.borrow() != ConnectorStatus::Connected {
            online.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    resources::exercise(&client, &origin, &headers, &connection.node_id).await;
    let create = |id: &str| json!({"node_id":connection.node_id,"request":{"plugin_skill_ids":[],"input":[{"text":"Acceptance task"}],"session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_depth":null,"research_sources":[],"plan_action":null,"branch":null,"max_turns":8,"idempotency_key":id}});
    let allocated = post(
        &client,
        &origin,
        &headers,
        "/api/projects/project-a/tasks",
        create("once"),
    )
    .await;
    let id = allocated["task"]["task_id"].as_str().unwrap();
    assert_eq!(
        post(
            &client,
            &origin,
            &headers,
            "/api/projects/project-a/tasks",
            create("once")
        )
        .await["task"]["task_id"],
        id
    );
    for approval in [false, true] {
        let task = wait_task(&client, &origin, &headers, id, |task| {
            task.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.pending_interactions.iter().any(|item| {
                    item.status == InteractionStatus::Pending
                        && matches!(item.content, InteractionContent::Approval(_)) == approval
                })
            })
        })
        .await;
        let interaction = task
            .snapshot
            .unwrap()
            .pending_interactions
            .into_iter()
            .find(|item| {
                item.status == InteractionStatus::Pending
                    && matches!(item.content, InteractionContent::Approval(_)) == approval
            })
            .unwrap();
        assert!(!workspace.join("approved.txt").exists());
        let answer = match &interaction.content {
            InteractionContent::UserPrompt(prompt) => {
                assert!(!approval);
                json!({"prompt":{"choice":{"choice_id":prompt.choices[0].choice_id,"label":prompt.choices[0].label}}})
            }
            InteractionContent::Approval(content) => {
                assert!(approval);
                json!({"approval":{"approved":true,"request_hash":content.request_hash}})
            }
        };
        post(&client,&origin,&headers,&format!("/api/projects/project-a/tasks/{id}/respond"),json!({"mutation_id":format!("answer-{approval}"),"request":{"run_id":interaction.run_id,"interaction_id":interaction.interaction_id,"etag":interaction.etag,"response":answer,"idempotency_key":format!("answer-{approval}")}})).await;
    }
    let completed = wait_task(&client, &origin, &headers, id, |task| {
        task.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.run.status == RunStatus::Completed
                && task.last_sequence == snapshot.run.last_sequence
        })
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(workspace.join("approved.txt")).unwrap(),
        "cloud-approved"
    );
    let mut events = client
        .get(format!(
            "{origin}/api/projects/project-a/tasks/{id}/events?after=0"
        ))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    let chunk = tokio::time::timeout(Duration::from_secs(3), events.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&chunk).contains("id: 1"));
    drop(events);
    let cancelled = post(
        &client,
        &origin,
        &headers,
        "/api/projects/project-a/tasks",
        create("cancel-once"),
    )
    .await;
    let cancel_id = cancelled["task"]["task_id"].as_str().unwrap();
    wait_task(&client, &origin, &headers, cancel_id, |task| {
        task.snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.pending_interactions.is_empty())
    })
    .await;
    post(
        &client,
        &origin,
        &headers,
        &format!("/api/projects/project-a/tasks/{cancel_id}/cancel"),
        json!({"mutation_id":"cancel-once"}),
    )
    .await;
    wait_task(&client, &origin, &headers, cancel_id, |task| {
        task.snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.run.status == RunStatus::Cancelled)
    })
    .await;
    disconnect.send_replace(true);
    connected.await.unwrap().unwrap();
    // The host commits rotation, but saving the returned leaf fails. Restart
    // with the old leaf and persisted pending CSR; automatic renewal must replay.
    vault.fail_write.store(2, Ordering::SeqCst);
    assert!(enrollment.renew(true).await.is_err());
    let (rotated, key) = enrollment.load().unwrap().unwrap();
    assert_eq!(rotated.certificate_pem, connection.certificate_pem);
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(5), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let server = crate::server::CloudServer::with_store(config.clone(), cloud_store.clone())
        .await
        .unwrap();
    let (stop, close) = watch::channel(false);
    let browser = tokio::net::TcpListener::bind(config.http_bind)
        .await
        .unwrap();
    let grpc = tokio::net::TcpListener::bind(config.grpc_bind)
        .await
        .unwrap();
    let host = tokio::spawn(server.serve_listeners(browser, grpc, close));
    let headers = sign_in(&client, &origin, &issuer).await;
    let recovered = wait_task(&client, &origin, &headers, id, |task| {
        task.last_sequence == completed.last_sequence
    })
    .await;
    assert_eq!(recovered.run_id, completed.run_id);
    let (disconnect, close) = watch::channel(false);
    let (status, mut online) = watch::channel(ConnectorStatus::Connecting);
    let connected = tokio::spawn(
        RuntimeConnector::new(rotated.clone(), key, runs)
            .unwrap()
            .with_enrollment_store(enrollment.clone())
            .run(close, status),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while *online.borrow() != ConnectorStatus::Connected {
            online.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_ne!(
        enrollment.load().unwrap().unwrap().0.certificate_pem,
        connection.certificate_pem
    );
    // Keep this connector alive through a host restart. An authenticated peer
    // that never completes HTTP/2 must not defeat the channel startup deadline.
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(5), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let listener = tokio::net::TcpListener::bind(config.grpc_bind)
        .await
        .unwrap();
    let stalled_config = config.clone();
    let (accepted, peer_accepted) = tokio::sync::oneshot::channel();
    let stalled = tokio::spawn(async move {
        use futures::StreamExt as _;
        let ca = std::fs::read_to_string(&stalled_config.ca_certificate).unwrap();
        let mut incoming = Box::pin(crate::tls::incoming(listener, &stalled_config, &ca).unwrap());
        let peer = incoming.next().await.unwrap().unwrap();
        accepted.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(peer);
    });
    tokio::time::timeout(Duration::from_secs(10), peer_accepted)
        .await
        .unwrap()
        .unwrap();
    drop(online.borrow_and_update());
    tokio::time::timeout(Duration::from_secs(12), online.changed())
        .await
        .expect("authenticated HTTP/2 stall must leave channel startup")
        .unwrap();
    assert_eq!(*online.borrow(), ConnectorStatus::Reconnecting);
    stalled.abort();
    assert!(stalled.await.unwrap_err().is_cancelled());
    let server = crate::server::CloudServer::with_store(config.clone(), cloud_store.clone())
        .await
        .unwrap();
    let (stop, close) = watch::channel(false);
    let browser = tokio::net::TcpListener::bind(config.http_bind)
        .await
        .unwrap();
    let grpc = tokio::net::TcpListener::bind(config.grpc_bind)
        .await
        .unwrap();
    let host = tokio::spawn(server.serve_listeners(browser, grpc, close));
    tokio::time::timeout(Duration::from_secs(10), async {
        while *online.borrow() != ConnectorStatus::Connected {
            online.changed().await.unwrap();
        }
    })
    .await
    .expect("same connector recovers after channel startup timeout");
    let headers = sign_in(&client, &origin, &issuer).await;
    let retained = wait_task(&client, &origin, &headers, id, |_| true).await;
    assert_eq!(retained.run_id, completed.run_id);
    assert_eq!(retained.last_sequence, completed.last_sequence);
    let fleet: Value = client
        .get(format!("{origin}/api/projects/project-a/nodes"))
        .headers(headers.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let node = &fleet["nodes"][0]["node"];
    post(
        &client,
        &origin,
        &headers,
        &format!("/api/projects/project-a/nodes/{}/revoke", rotated.node_id),
        json!({"revision":node["revision"]}),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), connected)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(*online.borrow(), ConnectorStatus::Revoked);
    // A native acknowledgement lost after host revocation remains replayable.
    enrollment.revoke().await.unwrap();
    enrollment.revoke().await.unwrap();
    assert!(enrollment.load().unwrap().unwrap().0.revoked);
    drop(disconnect);
    stop.send_replace(true);
    host.await.unwrap().unwrap();
    model.abort();
    oidc.abort();
}
