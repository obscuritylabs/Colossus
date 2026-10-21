//! Real shared SQL/independent-anchor acceptance with two API/execution replicas.
use super::*;
use crate::anchor_authority::{AnchorAuthority, AnchorAuthorityConfig};
use colossus_cloud_chat::{ChatRuntime, SharedChatKeyConfig};
use colossus_cloud_postgres::{ChatRecordWrite, CloudDatabaseConfig, CloudDatabaseTls};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    sync::RwLock,
};

const TEST: &str = "auth::tests::transport::hosted::shared::shared_hosted_chat_two_replicas";
const CHILD_ENV: &str = "COLOSSUS_SHARED_CHAT_TEST_CHILD";
const TOKEN: &str = "COLOSSUS_SHARED_CHAT_TEST_ANCHOR_TOKEN";
const ANCHOR_KEY: &str = "COLOSSUS_SHARED_CHAT_TEST_ANCHOR_KEY";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires two disposable local PostgreSQL databases; all model/OAuth credentials are synthetic"]
async fn shared_hosted_chat_two_replicas() {
    if std::env::var(CHILD_ENV).as_deref() != Ok(TEST) {
        let directory = isolated::directory();
        let home =
            ColossusHome::ensure_at(directory.path().canonicalize().unwrap().join("home")).unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(300),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--ignored", "--test-threads=1"])
                .env(CHILD_ENV, TEST)
                .env("COLOSSUS_HOME", home.root())
                .env(JOURNAL, "a".repeat(64))
                .env(SIGNING, "b".repeat(64))
                .env(WRAPPING, "c".repeat(64))
                .env(TOKEN, "synthetic-shared-chat-anchor-token")
                .env(ANCHOR_KEY, "d".repeat(64))
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("shared acceptance child deadline")
        .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    acceptance().await;
}

fn database(variable: &str, schema: String) -> CloudDatabaseConfig {
    CloudDatabaseConfig {
        connection_variable: variable.into(),
        schema,
        tls: CloudDatabaseTls::Disabled,
        max_connections: 4,
        connection_timeout_ms: 5000,
        statement_timeout_ms: 15000,
    }
}

async fn passthrough(listener: TcpListener, backend: Arc<RwLock<std::net::SocketAddr>>) {
    loop {
        let (mut downstream, _) = listener.accept().await.unwrap();
        let address = *backend.read().await;
        tokio::spawn(async move {
            if let Ok(mut upstream) = tokio::net::TcpStream::connect(address).await {
                let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
            }
        });
    }
}

async fn independent_anchor(
    schema: String,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let bind = listener.local_addr().unwrap();
    let origin = format!("http://{bind}/v1/anchors");
    let (stop, close) = tokio::sync::oneshot::channel();
    let (ready, started) = tokio::sync::oneshot::channel();
    // The journal's synchronous checkpoint must not block this authority's
    // reactor. An independent OS thread models the separately deployed service.
    let task = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let anchor = AnchorAuthority::open(AnchorAuthorityConfig {
                    bind,
                    database: database("COLOSSUS_CHAT_TEST_ANCHOR_DATABASE_URL", schema),
                    bearer_variable: TOKEN.into(),
                    storage_key_variable: ANCHOR_KEY.into(),
                    trusted_tls_ingress: false,
                })
                .await
                .unwrap();
                let listener = TcpListener::from_std(listener).unwrap();
                ready.send(()).unwrap();
                axum::serve(listener, anchor.router())
                    .with_graceful_shutdown(async {
                        let _ = close.await;
                    })
                    .await
                    .unwrap();
            });
    });
    started.await.unwrap();
    (origin, stop, task)
}

async fn oauth_fixture() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let token_exchanges = Arc::new(AtomicUsize::new(0));
    let handler_origin = origin.clone();
    let count = token_exchanges.clone();
    let handle = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let origin = handler_origin.clone();
            let count = count.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                    let mut buffer = [0; 2048];
                    let amount = socket.read(&mut buffer).await.unwrap();
                    if amount == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..amount]);
                    assert!(bytes.len() < 16384);
                }
                let request = String::from_utf8_lossy(&bytes);
                let first = request.lines().next().unwrap();
                let (status, headers, body) = if first.contains("/token ") {
                    count.fetch_add(1, Ordering::SeqCst);
                    (
                        "200 OK",
                        String::new(),
                        json!({"access_token":"synthetic-private-access","refresh_token":"synthetic-private-refresh","token_type":"Bearer","expires_in":3600,"scope":"read"}),
                    )
                } else if first.contains("oauth-protected-resource") {
                    (
                        "200 OK",
                        String::new(),
                        json!({"resource":format!("{origin}/mcp"),"authorization_servers":[origin],"scopes_supported":["read"]}),
                    )
                } else if first.contains(".well-known/") {
                    (
                        "200 OK",
                        String::new(),
                        json!({"issuer":origin,"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"]}),
                    )
                } else {
                    (
                        "401 Unauthorized",
                        format!(
                            "WWW-Authenticate: Bearer resource_metadata=\"{origin}/.well-known/oauth-protected-resource/mcp\"\r\n"
                        ),
                        json!({}),
                    )
                };
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            });
        }
    });
    (origin, token_exchanges, handle)
}

async fn native_evidence(state: &crate::server::State, identity: &str, session: &str) -> String {
    let chat = state.hosted.as_ref().unwrap();
    let lease = chat.shared.as_ref().unwrap().sql.read_lease(identity);
    let authority = lease
        .as_ref()
        .map(|lease| chat.shared.as_ref().unwrap().sql.verify_lease(lease));
    let Some(live) = chat.live.lock().await.get(identity).cloned() else {
        return format!("lease={lease:?}; authority={authority:?}; live=absent");
    };
    // SDK get_run can recover an orphan; capture non-mutating evidence first.
    let pending = live.runtime.has_pending_executions();
    let canonical_messages = live.runtime.runtime().session_messages(session);
    let stream_lease = state
        .repo
        .storage()
        .read_lease(&live.project_id, &live.node_id, crate::http::now())
        .await;
    let stream_authority = match &stream_lease {
        Ok(lease) => Some(
            state
                .repo
                .storage()
                .verify_lease(lease, crate::http::now())
                .await,
        ),
        _ => None,
    };
    let presence = state.presence.lock().unwrap().iter().map(|(key, value)| {
        (key.clone(), json!({"connection":value.connection_id,"ready":value.ready,"heartbeat_age_ms":value.heartbeat.map(|heartbeat| heartbeat.elapsed().as_millis())}))
    }).collect::<Vec<_>>();
    let runs = live
        .runtime
        .runs()
        .list_runs(colossus_sdk::ListRunsRequest {
            session_id: Some(session.into()),
            statuses: Vec::new(),
            page: None,
            include_archived: true,
        })
        .await;
    let mut snapshots = Vec::new();
    if let Ok(runs) = &runs {
        for run in &runs.runs {
            snapshots.push(
                live.runtime
                    .runs()
                    .get_run(colossus_sdk::GetRunRequest {
                        run_id: run.run_id.clone(),
                    })
                    .await,
            );
        }
    }
    format!(
        "lease={lease:?}; authority={authority:?}; stream_lease={stream_lease:?}; stream_authority={stream_authority:?}; presence={presence:?}; pending_before_probe={pending}; canonical_messages_before_probe={canonical_messages:?}; runs={runs:?}; snapshots={snapshots:?}"
    )
}

async fn released_transcript(
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    path: &str,
    turns: &[(&Value, &str)],
) -> Result<Value, Value> {
    let mut observed = Value::Null;
    let converged = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            observed = get(client, origin, headers, path).await;
            let messages = observed["messages"].as_array().unwrap();
            if messages.len() == turns.len() * 2
                && turns.iter().all(|(task, text)| {
                    ["user", "assistant"].iter().all(|role| {
                        messages.iter().any(|message| {
                            message["task_id"] == **task
                                && message["role"] == *role
                                && message["text"] == *text
                        })
                    })
                })
            {
                break observed.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    converged.map_err(|_| observed)
}

// SQL execution includes independent anchor round trips. This collection deadline
// does not extend provider/effect deadlines or change lease fencing and recovery.
async fn complete_sql(client: &reqwest::Client, origin: &str, headers: &HeaderMap, task: &Value) {
    let path = format!(
        "/api/projects/project-a/tasks/{}",
        task["task_id"].as_str().unwrap()
    );
    let mut last = Value::Null;
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            last = get(client, origin, headers, &path).await;
            let task = &last["task"];
            if task["snapshot"]["run"]["status"] == "completed"
                && (task["output_limited"] == true
                    || task["last_sequence"] == task["snapshot"]["run"]["last_sequence"])
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(result.is_ok(), "SQL execution did not settle: {last}");
}

async fn acceptance() {
    let (auth, issuer, oidc) = fixture().await;
    let directory = isolated::directory();
    let root = directory.path().canonicalize().unwrap();
    let browser_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let browser_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ingress = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ingress_address = ingress.local_addr().unwrap();
    let target = Arc::new(RwLock::new(grpc_a.local_addr().unwrap()));
    let load_balancer = tokio::spawn(passthrough(ingress, target.clone()));
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let (anchor_origin, stop_anchor, anchor_task) =
        independent_anchor(format!("chat_anchor_{suffix}")).await;
    let (mcp_origin, exchanges, mcp) = oauth_fixture().await;
    let mut config = auth.config.clone();
    config.http_bind = browser_a.local_addr().unwrap();
    config.grpc_bind = grpc_a.local_addr().unwrap();
    config.public_origin = format!("http://{}", config.http_bind);
    config.grpc_endpoint = format!("https://localhost:{}", ingress_address.port());
    config.auth_key_variable = Some(WRAPPING.into());
    config.database = database(
        "COLOSSUS_CLOUD_TEST_DATABASE_URL",
        format!("chat_two_replicas_{suffix}"),
    );
    config.memberships[0].permissions = BTreeSet::from([
        CloudPermission::Read,
        CloudPermission::Execute,
        CloudPermission::Control,
        CloudPermission::Administer,
    ]);
    config.memberships.push(Membership {
        subject: "not-a-member".into(),
        project_id: "project-a".into(),
        permissions: BTreeSet::from([
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
        ]),
    });
    tls_files(&root, &mut config);
    let mut runtime = RuntimeConfig::controlled_chat_template(root.join("seed.redb"));
    runtime.storage.keys = colossus_runtime::KeyConfig::Environment {
        journal_variable: JOURNAL.into(),
        journal_key_id: "seed".into(),
        signing_variable: SIGNING.into(),
        anchor_path: root.join("seed.anchor"),
    };
    runtime.sandbox.network_destinations =
        vec!["https://provider.example.test".into(), mcp_origin.clone()];
    runtime.providers.profiles.insert(
        "personal".into(),
        serde_json::from_value(
            json!({"kind":"open_ai_compatible","baseUrl":"https://provider.example.test/v1"}),
        )
        .unwrap(),
    );
    runtime.mcp.servers.insert("docs".into(),serde_json::from_value(json!({"transport":"streamable_http","url":format!("{mcp_origin}/mcp"),"allowedTools":["read"],"oauth":{"clientId":"shared-test-client","callbackPort":8765,"scopes":["read"]}})).unwrap());
    let project = ChatProjectConfig {
        labels: Default::default(),
        codex_auth_shared: false,
        runtime,
        mcp_authentication: BTreeMap::new(),
        provider_authentication: BTreeMap::new(),
        provider_generations: BTreeMap::new(),
        mcp_generations: BTreeMap::new(),
        revision: 1,
        codex_auth_path: None,
    };
    config.chat = Some(ChatConfig {
        shared: Some(crate::config::ChatSharedConfig {
            keys: SharedChatKeyConfig {
                journal_variable: JOURNAL.into(),
                signing_variable: SIGNING.into(),
                anchor_endpoint: anchor_origin,
                anchor_bearer_variable: TOKEN.into(),
                anchor_ca_pem_path: None,
                allow_loopback_http: true,
            },
        }),
        home: root.join("cache-a"),
        wrapping_key_variable: WRAPPING.into(),
        max_active_runtimes: 4,
        idle_seconds: 300,
        projects: BTreeMap::from([("project-a".into(), project)]),
    });
    drop(auth);
    let origin_a = config.public_origin.clone();
    let origin_b = format!("http://{}", browser_b.local_addr().unwrap());
    let mut config_b = config.clone();
    config_b.http_bind = browser_b.local_addr().unwrap();
    config_b.grpc_bind = grpc_b.local_addr().unwrap();
    config_b.chat.as_mut().unwrap().home = root.join("cache-b");
    let server_a = crate::server::CloudServer::open(config.clone())
        .await
        .unwrap();
    let state_a = server_a.state.clone();
    let server_b = crate::server::CloudServer::open(config_b.clone())
        .await
        .unwrap();
    let state_b = server_b.state.clone();
    let (stop_a, shutdown_a) = watch::channel(false);
    let host_a = tokio::spawn(server_a.serve_listeners(browser_a, grpc_a, shutdown_a));
    let (stop_b, shutdown_b) = watch::channel(false);
    let backend_b = grpc_b.local_addr().unwrap();
    let host_b = tokio::spawn(server_b.serve_listeners(browser_b, grpc_b, shutdown_b));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let alice = sign_in(&client, &origin_a, &issuer).await;
    *issuer.invalid.lock().unwrap() = "subject";
    let bob = sign_in(&client, &origin_a, &issuer).await;
    *issuer.invalid.lock().unwrap() = "";
    super::credential_store::cross_replica(&client, &origin_a, &origin_b, &alice, &bob).await;
    // A configuration edit through B is visible on A; stale CAS cannot overwrite it.
    let mut settings = get(
        &client,
        &origin_a,
        &alice,
        "/api/projects/project-a/chat/settings",
    )
    .await;
    settings
        .as_object_mut()
        .unwrap()
        .remove("credential_status");
    let stale = settings.clone();
    settings["models"]["profiles"]["echo"]["model"] = json!("echo-shared");
    settings["labels"] =
        json!({"providers":{"echo":"Shared provider"},"models":{"echo":"Shared model"}});
    assert_eq!(
        client
            .put(format!("{origin_b}/api/projects/project-a/chat/settings"))
            .headers(alice.clone())
            .json(&settings)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        get(
            &client,
            &origin_a,
            &alice,
            "/api/projects/project-a/chat/settings"
        )
        .await["revision"],
        2
    );
    let retained = get(
        &client,
        &origin_a,
        &alice,
        "/api/projects/project-a/chat/settings",
    )
    .await;
    assert_eq!(retained["labels"], settings["labels"]);
    assert_eq!(
        client
            .put(format!("{origin_a}/api/projects/project-a/chat/settings"))
            .headers(alice.clone())
            .json(&stale)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let providers = get(
        &client,
        &origin_b,
        &alice,
        "/api/projects/project-a/chat/providers",
    )
    .await;
    let personal = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "personal")
        .unwrap();
    post(&client,&origin_b,&alice,"/api/projects/project-a/chat/providers/personal/credential",json!({"secret":"synthetic-personal-secret","credential_binding":personal["credential_binding"]})).await;
    assert_eq!(
        get(
            &client,
            &origin_a,
            &alice,
            "/api/projects/project-a/chat/providers"
        )
        .await["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "personal")
            .unwrap()["authenticated"],
        true
    );
    assert_eq!(
        get(
            &client,
            &origin_b,
            &bob,
            "/api/projects/project-a/chat/providers"
        )
        .await["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "personal")
            .unwrap()["authenticated"],
        false
    );
    let alice_node = ready(&client, &origin_a, &alice).await["node"]["node"].clone();
    super::view::cross_replica(&client, &origin_a, &origin_b, &alice).await;
    let namespace =
        ChatRuntime::identity_for("project-a", alice_node["owner_user_id"].as_str().unwrap())
            .unwrap();
    let shared = state_a.hosted.as_ref().unwrap().shared.as_ref().unwrap();
    let before = shared.sql.read_lease(&namespace).unwrap();
    let base = "/api/projects/project-a/chat/credential-store/personal";
    let inventory = get(&client, &origin_b, &alice, base).await;
    post(&client, &origin_b, &alice, base, json!({"revision":inventory["revision"],"label":"Unused while running","kind":"api_key","secret":"synthetic-idle-key"})).await;
    let after = shared.sql.read_lease(&namespace).unwrap();
    assert_eq!(
        (before.owner_id, before.generation),
        (after.owner_id, after.generation)
    );
    let catalog = post(
        &client,
        &origin_b,
        &alice,
        "/api/projects/project-a/chat/settings/providers/echo/models",
        json!({"revision":2}),
    )
    .await;
    assert_eq!(catalog["models"][0]["id"], "echo-shared");
    assert!(
        state_b
            .hosted
            .as_ref()
            .unwrap()
            .live
            .lock()
            .await
            .is_empty()
    );

    let first = post(
        &client,
        &origin_b,
        &alice,
        "/api/projects/project-a/chat/threads",
        json!({"request":request("shared-first","Created through B, executed on A")}),
    )
    .await;
    complete_sql(&client, &origin_b, &alice, &first["task"]).await;
    assert!(
        state_b
            .hosted
            .as_ref()
            .unwrap()
            .live
            .lock()
            .await
            .is_empty()
    );
    let path = format!(
        "/api/projects/project-a/threads/{}",
        first["thread"]["thread_id"].as_str().unwrap()
    );
    let detail = get(&client, &origin_b, &alice, &path).await;
    let session = detail["thread"]["session_id"].clone();
    let second=post(&client,&origin_b,&alice,&format!("{path}/messages"),json!({"expected_revision":detail["thread"]["revision"],"request":request("shared-second","Follow-up through B")})).await;
    complete_sql(&client, &origin_a, &alice, &second["task"]).await;
    assert_eq!(
        get(
            &client,
            &origin_b,
            &bob,
            "/api/projects/project-a/chat/threads"
        )
        .await["threads"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // Begin on A and complete on B with exact browser session/state binding and one exchange.
    let login = post(
        &client,
        &origin_a,
        &alice,
        "/api/projects/project-a/chat/connections/docs/auth",
        json!({}),
    )
    .await;
    let authorization = url::Url::parse(login["authorization_url"].as_str().unwrap()).unwrap();
    let query: HashMap<_, _> = authorization.query_pairs().into_owned().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    let callback = format!(
        "{origin_b}/auth/mcp/callback?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("state", &query["state"])
            .append_pair("code", "synthetic-authorization-code")
            .finish()
    );
    assert!(
        !client
            .get(&callback)
            .headers(bob.clone())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
    let other_alice_session = sign_in(&client, &origin_a, &issuer).await;
    assert!(
        !client
            .get(&callback)
            .headers(other_alice_session)
            .send()
            .await
            .unwrap()
            .status()
            .is_success(),
        "a different browser session cannot consume the original OAuth flow"
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
    assert!(
        client
            .get(&callback)
            .headers(alice.clone())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert!(
        !client
            .get(&callback)
            .headers(alice.clone())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    eprintln!("shared acceptance: OAuth completed once; inspect owner-routed status");
    assert_eq!(
        get(
            &client,
            &origin_b,
            &alice,
            "/api/projects/project-a/chat/connections"
        )
        .await["connections"][0]["authenticated"],
        true
    );
    // Expire the exact old owner while retaining its handle; stale writes are fenced.
    eprintln!("shared acceptance: shared status valid; expire old execution owner");
    let caller = state_a
        .auth
        .caller(&alice, "project-a", false)
        .await
        .unwrap();
    let identity = ChatRuntime::identity_for("project-a", caller.subject()).unwrap();
    let chat_a = state_a.hosted.as_ref().unwrap().clone();
    let shared = chat_a.shared.clone().unwrap();
    let fixture_key = colossus_ports::CredentialKey::new("acceptance", "lease-proof").unwrap();
    let fixture_record =
        colossus_contracts::VaultRecord::new(b"synthetic-execution-record".to_vec()).unwrap();
    let cached = Arc::downgrade(chat_a.live.lock().await.get(&identity).unwrap());
    assert!(!cached.upgrade().unwrap().runtime.has_pending_executions());
    let cached_lease = shared.sql.read_lease(&identity).unwrap();
    let cached_vault = chat_a
        .execution_vault("project-a", caller.subject(), Some(cached_lease.clone()))
        .unwrap();
    cached_vault.write(&fixture_key, &fixture_record).unwrap();
    shared.sql.release_lease(&cached_lease).unwrap();
    // Beat the monitor's next renewal: cached state must verify its captured
    // lease and retire the old generation before reopening on the same replica.
    let mut replacement = chat_a.ensure(&state_a, &caller).await;
    if replacement == Err(colossus_cloud::CloudError::Storage) {
        // An already-ended old connector can report cleanup failure after its
        // slot was removed. A second admission must then claim a new owner.
        replacement = chat_a.ensure(&state_a, &caller).await;
    }
    assert_eq!(
        replacement.unwrap(),
        alice_node["node_id"].as_str().unwrap()
    );
    let replacement = chat_a.live.lock().await.get(&identity).cloned().unwrap();
    assert!(!std::sync::Weak::ptr_eq(
        &cached,
        &Arc::downgrade(&replacement)
    ));
    assert!(
        cached.upgrade().is_none(),
        "retired lifecycle handles drained"
    );
    assert_eq!(chat_a.live.lock().await.len(), 1);
    drop(replacement);
    let old_lease = shared.sql.read_lease(&identity).unwrap();
    assert!(old_lease.generation > cached_lease.generation);
    assert!(
        chat_a
            .execution_vault("project-a", caller.subject(), Some(cached_lease))
            .is_err()
    );
    assert!(cached_vault.read(&fixture_key).is_err());
    assert!(cached_vault.write(&fixture_key, &fixture_record).is_err());
    ready(&client, &origin_a, &alice).await;
    let old_vault = chat_a
        .execution_vault("project-a", caller.subject(), Some(old_lease.clone()))
        .unwrap();
    old_vault.write(&fixture_key, &fixture_record).unwrap();
    shared.sql.release_lease(&old_lease).unwrap();
    assert!(
        shared
            .sql
            .write_records_fenced(
                &[ChatRecordWrite {
                    namespace: &identity,
                    record_id: "stale-test",
                    expected_revision: 0,
                    ciphertext: Some(b"stale"),
                    ttl_seconds: None
                }],
                Some(&old_lease)
            )
            .is_err()
    );
    *target.write().await = backend_b;
    stop_a.send_replace(true);
    tokio::time::timeout(Duration::from_secs(40), host_a)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(state_a);
    for cache in [root.join("cache-a/users"), root.join("cache-b/users")] {
        if cache.exists() {
            // Bundled plugin snapshots are deliberately read-only. Retire the
            // complete disposable cache entry without modifying their modes;
            // neither replica can find it under its configured users path.
            std::fs::rename(&cache, cache.with_extension("retired")).unwrap();
            assert!(!cache.exists());
        }
    }
    let resumed = ready(&client, &origin_b, &alice).await;
    eprintln!("shared acceptance: B takeover connected after clearing caches");
    assert!(
        chat_a
            .execution_vault("project-a", caller.subject(), Some(old_lease.clone()))
            .is_err(),
        "a stale constructor cannot adopt the replacement owner's lease"
    );
    assert!(old_vault.write(&fixture_key, &fixture_record).is_err());
    assert!(old_vault.read(&fixture_key).is_err());
    let new_lease = shared.sql.read_lease(&identity).unwrap();
    assert_ne!(new_lease.generation, old_lease.generation);
    let new_vault = state_b
        .hosted
        .as_ref()
        .unwrap()
        .execution_vault("project-a", caller.subject(), Some(new_lease))
        .unwrap();
    new_vault.write(&fixture_key, &fixture_record).unwrap();
    assert_eq!(
        new_vault.read(&fixture_key).unwrap().unwrap().expose(),
        fixture_record.expose()
    );
    assert_eq!(resumed["node"]["node"]["node_id"], alice_node["node_id"]);
    let prior_turns = [
        (
            &first["task"]["task_id"],
            "Created through B, executed on A",
        ),
        (&second["task"]["task_id"], "Follow-up through B"),
    ];
    // A connected runtime can still be replaying the prior owner's interrupted
    // released feed. Require both complete original turns before continuing.
    let detail = match released_transcript(&client, &origin_b, &alice, &path, &prior_turns).await {
        Ok(detail) => detail,
        Err(observed) => panic!(
            "recovered prior transcript deadline: cloud={observed:?}; native={}",
            native_evidence(&state_b, &identity, session.as_str().unwrap()).await
        ),
    };
    assert_eq!(detail["thread"]["session_id"], session);
    assert_eq!(detail["messages"].as_array().unwrap().len(), 4);
    let third_request = request("shared-third", "After owner loss and an empty cache");
    let third = post(
        &client,
        &origin_b,
        &alice,
        &format!("{path}/messages"),
        json!({"expected_revision":detail["thread"]["revision"],"request":third_request}),
    )
    .await;
    complete_sql(&client, &origin_b, &alice, &third["task"]).await;
    // Task snapshots and released transcript messages arrive in separate
    // connector updates. Require every exact original turn after convergence.
    let turns = [
        prior_turns[0],
        prior_turns[1],
        (
            &third["task"]["task_id"],
            "After owner loss and an empty cache",
        ),
    ];
    let detail = match released_transcript(&client, &origin_b, &alice, &path, &turns).await {
        Ok(detail) => detail,
        Err(observed) => {
            let messages = observed["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| {
                json!({"id":message["message_id"],"task":message["task_id"],"role":message["role"],"synthetic_text":message["text"]})
            })
            .collect::<Vec<_>>();
            let tasks = observed["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(
                    |task| json!({"id":task["task_id"],"status":task["snapshot"]["run"]["status"]}),
                )
                .collect::<Vec<_>>();
            panic!(
                "released transcript deadline: messages={messages:?}; tasks={tasks:?}; native={}",
                native_evidence(&state_b, &identity, session.as_str().unwrap()).await
            );
        }
    };
    let count = detail["messages"].as_array().unwrap().len();
    assert_eq!(count, 6, "each synthetic turn is released exactly once");
    assert_eq!(detail["thread"]["session_id"], session);
    let replay = post(
        &client,
        &origin_b,
        &alice,
        &format!("{path}/messages"),
        json!({"expected_revision":detail["thread"]["revision"],"request":third_request}),
    )
    .await;
    assert_eq!(replay["task"]["task_id"], third["task"]["task_id"]);
    assert_eq!(
        get(&client, &origin_b, &alice, &path).await["messages"]
            .as_array()
            .unwrap()
            .len(),
        count
    );
    eprintln!(
        "shared acceptance: B resumed same session and idempotent replay; inspect retained OAuth status"
    );
    assert_eq!(
        get(
            &client,
            &origin_b,
            &alice,
            "/api/projects/project-a/chat/connections"
        )
        .await["connections"][0]["authenticated"],
        true
    );
    stop_b.send_replace(true);
    tokio::time::timeout(Duration::from_secs(40), host_b)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(state_b);
    load_balancer.abort();
    stop_anchor.send(()).unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || anchor_task.join().unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    mcp.abort();
    oidc.abort();
}
