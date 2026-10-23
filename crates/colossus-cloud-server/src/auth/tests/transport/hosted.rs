//! Hosted Chat acceptance exercises HTTP, direct SDK dispatch, file custody and restart.
use super::*;
use crate::config::{ChatConfig, ChatProjectConfig};
use std::collections::BTreeMap;

const CHILD: &str = "auth::tests::transport::hosted::hosted_chat_child";
const JOURNAL: &str = "COLOSSUS_HOSTED_TEST_JOURNAL";
const SIGNING: &str = "COLOSSUS_HOSTED_TEST_SIGNING";
const WRAPPING: &str = "COLOSSUS_HOSTED_TEST_WRAPPING";
mod catalog;
mod credential_store;
mod shared;
mod view;

#[tokio::test]
async fn personal_chat_http_mtls_credentials_settings_and_restart() {
    let directory = isolated::directory();
    let home =
        ColossusHome::ensure_at(directory.path().canonicalize().unwrap().join("home")).unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD, "--ignored", "--test-threads=1"])
            .env("COLOSSUS_HOME", home.root())
            .env(JOURNAL, "a".repeat(64))
            .env(SIGNING, "b".repeat(64))
            .env(WRAPPING, "c".repeat(64))
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("hosted chat child deadline")
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

async fn get(client: &reqwest::Client, origin: &str, headers: &HeaderMap, path: &str) -> Value {
    client
        .get(format!("{origin}{path}"))
        .headers(headers.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
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
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert!(status.is_success(), "{path}: {status} {value}");
    value
}

fn request(id: &str, text: &str) -> Value {
    json!({"plugin_skill_ids":[],"input":[{"text":text}],"session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_depth":null,"research_sources":[],"plan_action":null,"branch":null,"max_turns":4,"idempotency_key":id})
}

async fn ready(client: &reqwest::Client, origin: &str, headers: &HeaderMap) -> Value {
    let status = post(
        client,
        origin,
        headers,
        "/api/projects/project-a/chat/connect",
        json!({}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = get(client, origin, headers, "/api/projects/project-a/chat").await;
            if status["node"]["presence"]["ready"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("hosted connector readiness");
    status
}

async fn complete(client: &reqwest::Client, origin: &str, headers: &HeaderMap, task: &Value) {
    wait_task(
        client,
        origin,
        headers,
        task["task_id"].as_str().unwrap(),
        |task| {
            task.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.run.status == RunStatus::Completed
                    && (task.output_limited || task.last_sequence >= snapshot.run.last_sequence)
            })
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "isolated headless helper invoked by hosted chat acceptance; synthetic environment keys only"]
async fn hosted_chat_child() {
    let (auth, issuer, oidc) = fixture().await;
    let directory = isolated::directory();
    let root = directory.path().canonicalize().unwrap();
    let browser = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = auth.config.clone();
    config.http_bind = browser.local_addr().unwrap();
    config.grpc_bind = grpc.local_addr().unwrap();
    config.public_origin = format!("http://{}", config.http_bind);
    config.grpc_endpoint = format!("https://localhost:{}", config.grpc_bind.port());
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
    runtime.sandbox.network_destinations = vec!["https://provider.example.test".into()];
    runtime.providers.profiles.insert(
        "personal".into(),
        serde_json::from_value(json!({
            "kind":"open_ai_compatible", "baseUrl":"https://provider.example.test/v1"
        }))
        .unwrap(),
    );
    let project = ChatProjectConfig {
        labels: Default::default(),
        codex_auth_shared: false,
        runtime,
        mcp_authentication: BTreeMap::new(),
        provider_authentication: BTreeMap::new(),
        provider_generations: BTreeMap::new(),
        mcp_generations: BTreeMap::new(),
        search_authentication: BTreeMap::new(),
        search_generations: BTreeMap::new(),
        variables: BTreeMap::new(),
        inheritance: Default::default(),
        catalog_disabled: Default::default(),
        revision: 1,
        codex_auth_path: None,
    };
    config.chat = Some(ChatConfig {
        shared: None,
        home: root.join("chat"),
        wrapping_key_variable: WRAPPING.into(),
        max_active_runtimes: 4,
        idle_seconds: 300,
        projects: BTreeMap::from([("project-a".into(), project)]),
    });
    drop(auth);
    let origin = config.public_origin.clone();
    let store = Arc::new(colossus_cloud::storage::MemoryCloudStore::default());
    let server = crate::server::CloudServer::with_store(config.clone(), store.clone())
        .await
        .unwrap();
    let state = server.state.clone();
    let (stop, shutdown) = watch::channel(false);
    let host = tokio::spawn(server.serve_listeners(browser, grpc, shutdown));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let alice = sign_in(&client, &origin, &issuer).await;
    *issuer.invalid.lock().unwrap() = "subject";
    let bob = sign_in(&client, &origin, &issuer).await;
    *issuer.invalid.lock().unwrap() = "";
    credential_store::exercise(&client, &origin, &alice, &bob, &state).await;
    let status = get(&client, &origin, &alice, "/api/projects/project-a/chat").await;
    assert_eq!(status["enabled"], true);
    assert_eq!(status["node"]["execution"]["state"], "on_demand");
    assert_eq!(status["node"]["node"]["execution_kind"], "hosted");
    assert!(state.hosted.as_ref().unwrap().live.lock().await.is_empty());
    view::cold(&client, &origin, &alice, &state).await;
    let mut no_csrf = alice.clone();
    no_csrf.remove("x-colossus-csrf");
    assert_eq!(
        client
            .post(format!("{origin}/api/projects/project-a/chat/connect"))
            .headers(no_csrf)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let alice_node = ready(&client, &origin, &alice).await["node"]["node"].clone();
    let bob_node = ready(&client, &origin, &bob).await["node"]["node"].clone();
    catalog::acceptance(&client, &origin, &alice, &bob).await;
    assert_ne!(alice_node["node_id"], bob_node["node_id"]);
    assert_ne!(alice_node["owner_user_id"], bob_node["owner_user_id"]);
    assert_eq!(
        ready(&client, &origin, &alice).await["node"]["node"]["node_id"],
        alice_node["node_id"]
    );
    assert_eq!(state.hosted.as_ref().unwrap().live.lock().await.len(), 2);
    view::lifecycle(&client, &origin, &alice, &bob).await;
    let fleet = get(&client, &origin, &alice, "/api/projects/project-a/nodes").await;
    assert!(
        fleet["nodes"].as_array().unwrap().is_empty(),
        "Hosted targets are not Fleet enrollments"
    );
    {
        let chat = state.hosted.as_ref().unwrap();
        let _lifecycle = chat.lifecycle.lock().await;
        chat.evict_idle(&state, true).await.unwrap();
        assert_eq!(
            chat.live.lock().await.len(),
            1,
            "capacity pressure retires one idle context"
        );
    }
    use base64::Engine as _;
    let content = "name,value\nmanaged,42\n";
    let upload = json!({"file_name":"data.csv","media_type":"text/csv",
        "content_base64":base64::engine::general_purpose::STANDARD.encode(content),"idempotency_key":"managed-file"});
    let file = post(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/hosted/files",
        upload.clone(),
    )
    .await;
    assert_eq!(
        file,
        post(
            &client,
            &origin,
            &alice,
            "/api/projects/project-a/hosted/files",
            upload
        )
        .await
    );
    let file_path = format!(
        "/api/projects/project-a/hosted/files/{}",
        file["artifact"]["artifact_id"].as_str().unwrap()
    );
    let download = client
        .get(format!("{origin}{file_path}"))
        .headers(alice.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(download.status(), 200);
    assert!(
        download.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
    assert_eq!(download.text().await.unwrap(), content);
    assert_eq!(
        client
            .get(format!("{origin}{file_path}"))
            .headers(bob.clone())
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let mut first_request = request("first", "First personal message");
    first_request["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"artifact":file["artifact"]["artifact_id"]}));
    let first = post(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/threads",
        json!({"request":first_request}),
    )
    .await;
    complete(&client, &origin, &alice, &first["task"]).await;
    let thread_id = first["thread"]["thread_id"].as_str().unwrap();
    let thread_path = format!("/api/projects/project-a/threads/{thread_id}");
    let detail = get(&client, &origin, &alice, &thread_path).await;
    assert_eq!(
        detail["thread"]["can_continue"], true,
        "first-turn continuation metadata: {}",
        detail["thread"]
    );
    let second = post(&client, &origin, &alice, &format!("{thread_path}/messages"), json!({"expected_revision":detail["thread"]["revision"],"request":request("second", "Second personal message")})).await;
    complete(&client, &origin, &alice, &second["task"]).await;
    assert_eq!(first["thread"]["node_id"], second["thread"]["node_id"]);
    let history = get(&client, &origin, &alice, &thread_path).await;
    assert!(history["messages"].as_array().unwrap().len() >= 4);
    assert!(!history["thread"]["session_id"].is_null());
    view::history(&client, &origin, &alice, &bob).await;
    let bob_threads = get(
        &client,
        &origin,
        &bob,
        "/api/projects/project-a/chat/threads",
    )
    .await;
    assert!(bob_threads["threads"].as_array().unwrap().is_empty());
    for path in [
        &thread_path,
        &format!(
            "/api/projects/project-a/tasks/{}",
            first["task"]["task_id"].as_str().unwrap()
        ),
        &format!(
            "/api/projects/project-a/nodes/{}",
            alice_node["node_id"].as_str().unwrap()
        ),
    ] {
        assert!(
            !client
                .get(format!("{origin}{path}"))
                .headers(bob.clone())
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
    }
    assert!(!client.post(format!("{origin}{thread_path}/messages")).headers(bob.clone()).json(&json!({"expected_revision":history["thread"]["revision"],"request":request("foreign", "Forbidden")})).send().await.unwrap().status().is_success());
    // Accepted work fences settings even while execution and placement are stopped.
    // Restarting the supervisor must discover it without a browser prewarm or wakeup.
    let caller = state.auth.caller(&alice, "project-a", false).await.unwrap();
    let chat = state.hosted.as_ref().unwrap();
    if let Some(supervisor) = chat.supervisor.lock().await.take() {
        supervisor.abort();
        let _ = supervisor.await;
    }
    let queued = {
        let _lifecycle = chat.lifecycle.lock().await;
        chat.stop_user("project-a", caller.subject()).await.unwrap();
        state
            .repo
            .create_personal_thread(
                &caller,
                alice_node["node_id"].as_str().unwrap(),
                None,
                serde_json::from_value(request("queued-offline", "Queued while offline")).unwrap(),
            )
            .await
            .unwrap()
            .1
    };
    let mut pending_settings = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/settings",
    )
    .await;
    view::busy(&client, &origin, &alice, &queued.task_id).await;
    pending_settings
        .as_object_mut()
        .unwrap()
        .remove("credential_status");
    assert_eq!(
        client
            .put(format!("{origin}/api/projects/project-a/chat/settings"))
            .headers(alice.clone())
            .json(&pending_settings)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        state
            .repo
            .get_task(&caller, &queued.task_id)
            .await
            .unwrap()
            .task_id,
        queued.task_id
    );
    *chat.supervisor.lock().await = Some(crate::hosted::HostedExecution::start_supervisor(&state));
    complete(
        &client,
        &origin,
        &alice,
        &serde_json::to_value(queued).unwrap(),
    )
    .await;
    // A personal provider starts unconfigured even with no raw credential reference.
    let providers = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/providers",
    )
    .await;
    let personal = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "personal")
        .unwrap();
    assert_eq!(personal["authenticated"], false);
    let binding = personal["credential_binding"].clone();
    post(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/providers/personal/credential",
        json!({"secret":"fixture-personal-secret", "credential_binding":binding}),
    )
    .await;
    let providers = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/providers",
    )
    .await;
    assert_eq!(
        providers["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "personal")
            .unwrap()["authenticated"],
        true
    );
    let providers = get(
        &client,
        &origin,
        &bob,
        "/api/projects/project-a/chat/providers",
    )
    .await;
    assert_eq!(
        providers["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "personal")
            .unwrap()["authenticated"],
        false
    );
    // A live resolver and routes must reuse one exclusive vault lease.
    let left = state
        .hosted
        .as_ref()
        .unwrap()
        .user_vault("project-a", caller.subject())
        .unwrap();
    let right = state
        .hosted
        .as_ref()
        .unwrap()
        .user_vault("project-a", caller.subject())
        .unwrap();
    assert!(Arc::ptr_eq(&left, &right));
    drop((left, right));
    ready(&client, &origin, &alice).await;
    {
        let identity =
            colossus_cloud_chat::ChatRuntime::identity_for("project-a", caller.subject()).unwrap();
        let live = state
            .hosted
            .as_ref()
            .unwrap()
            .live
            .lock()
            .await
            .get(&identity)
            .cloned()
            .unwrap();
        let child = live
            .runtime
            .runtime()
            .queue_subagent(
                history["thread"]["session_id"].as_str().unwrap(),
                "Pending controlled child",
                "primary",
            )
            .await
            .unwrap();
        assert!(
            state
                .hosted
                .as_ref()
                .unwrap()
                .user_is_busy(&state, "project-a", caller.subject())
                .await
                .unwrap()
        );
        assert!(
            state
                .hosted
                .as_ref()
                .unwrap()
                .active_project_run_status("project-a")
                .await
                .unwrap()
        );
        live.runtime
            .runtime()
            .cancel_subagent(&child.id)
            .await
            .unwrap();
        assert!(
            !state
                .hosted
                .as_ref()
                .unwrap()
                .active_project_run_status("project-a")
                .await
                .unwrap()
        );
    }
    // Idle model changes preserve connection generations and their private keys.
    let mut settings = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/settings",
    )
    .await;
    assert!(!settings.to_string().contains("fixture-personal-secret"));
    settings
        .as_object_mut()
        .unwrap()
        .remove("credential_status");
    settings["models"]["profiles"]["echo"]["model"] = json!("echo-updated");
    settings["models"]["roles"]["subagent_default"] = json!("echo");
    let saved = client
        .put(format!("{origin}/api/projects/project-a/chat/settings"))
        .headers(alice.clone())
        .json(&settings)
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200, "{}", saved.text().await.unwrap());
    let configuration = state
        .hosted
        .as_ref()
        .unwrap()
        .project_config("project-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(configuration.revision, 2);
    assert_eq!(
        configuration.generation(
            colossus_cloud_chat::ChatConnectionKind::Provider,
            "personal"
        ),
        1
    );
    let providers = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/providers",
    )
    .await;
    assert_eq!(
        providers["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "personal")
            .unwrap()["authenticated"],
        true
    );
    assert!(
        !client
            .get(format!("{origin}/api/projects/project-a/chat/settings"))
            .headers(bob.clone())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    // Repointing the connection cannot carry the prior endpoint's credential.
    let mut settings = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/settings",
    )
    .await;
    settings
        .as_object_mut()
        .unwrap()
        .remove("credential_status");
    settings["providers"]["profiles"]["personal"]["baseUrl"] =
        json!("https://provider.example.test/v2");
    let changed = client
        .put(format!("{origin}/api/projects/project-a/chat/settings"))
        .headers(alice.clone())
        .json(&settings)
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), 200, "{}", changed.text().await.unwrap());
    let configuration = state
        .hosted
        .as_ref()
        .unwrap()
        .project_config("project-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        configuration.generation(
            colossus_cloud_chat::ChatConnectionKind::Provider,
            "personal"
        ),
        2
    );
    let providers = get(
        &client,
        &origin,
        &alice,
        "/api/projects/project-a/chat/providers",
    )
    .await;
    let personal = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "personal")
        .unwrap();
    assert_eq!(personal["authenticated"], false);
    assert_ne!(personal["credential_binding"], binding);
    assert_eq!(
        client
            .post(format!(
                "{origin}/api/projects/project-a/chat/providers/personal/credential"
            ))
            .headers(alice.clone())
            .json(&json!({"secret":"stale-endpoint-secret", "credential_binding":binding}))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    // The unchanged deployment seed cannot roll back retained project settings.
    drop(state);
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(40), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let browser = tokio::net::TcpListener::bind(config.http_bind)
        .await
        .unwrap();
    let grpc = tokio::net::TcpListener::bind(config.grpc_bind)
        .await
        .unwrap();
    let server = crate::server::CloudServer::with_store(config, store)
        .await
        .unwrap();
    assert_eq!(
        server
            .state
            .hosted
            .as_ref()
            .unwrap()
            .project_config("project-a")
            .await
            .unwrap()
            .unwrap()
            .revision,
        3
    );
    let (stop, shutdown) = watch::channel(false);
    let host = tokio::spawn(server.serve_listeners(browser, grpc, shutdown));
    let resumed = ready(&client, &origin, &alice).await;
    assert_eq!(resumed["node"]["node"]["node_id"], alice_node["node_id"]);
    assert!(
        resumed["node"]["node"]["roles"]
            .as_array()
            .unwrap()
            .contains(&json!("subagent_default"))
    );
    assert_eq!(
        resumed["node"]["node"]["tools_grant"],
        alice_node["tools_grant"]
    );
    let history = get(&client, &origin, &alice, &thread_path).await;
    let third = post(&client, &origin, &alice, &format!("{thread_path}/messages"), json!({"expected_revision":history["thread"]["revision"],"request":request("third", "After restart")})).await;
    complete(&client, &origin, &alice, &third["task"]).await;
    stop.send_replace(true);
    tokio::time::timeout(Duration::from_secs(40), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    oidc.abort();
}
