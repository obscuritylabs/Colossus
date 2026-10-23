//! Instance updates respect durable accepted work and refresh inherited caches.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use colossus_cloud::ProjectRole;
use colossus_cloud_chat::{
    ChatAuthentication, ChatConnectionKind, ChatCredentialResolver, CredentialStore,
    StoredCredentialKind, StoredCredentialMetadata,
};
use colossus_contracts::HostSecret;
use std::collections::BTreeMap;
use tower::ServiceExt;

const CHILD: &str = "COLOSSUS_GLOBAL_LIFECYCLE_TEST_CHILD";
const WRAPPING: &str = "COLOSSUS_GLOBAL_LIFECYCLE_TEST_WRAPPING";
const TEST: &str = "auth::tests::transport::global_lifecycle::global_updates_refresh_inheritance_and_refuse_cold_accepted_work";

async fn replace(router: Router, headers: HeaderMap, value: Value) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri("/api/admin/hosted-settings");
    *request.headers_mut().unwrap() = headers;
    let response = router
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn read(router: Router, headers: HeaderMap, path: &str) -> Value {
    let mut request = Request::builder().uri(path);
    *request.headers_mut().unwrap() = headers;
    let response = router
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
}

async fn post(router: Router, headers: HeaderMap, path: &str, body: Value) -> (StatusCode, Value) {
    let mut request = Request::builder().method(Method::POST).uri(path);
    *request.headers_mut().unwrap() = headers;
    let response = router
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn unconfigured_inventory(
    router: &Router,
    headers: &HeaderMap,
    state: &crate::server::State,
) {
    let actor = state.auth.admin(headers, false).await.unwrap();
    state
        .repo
        .save_project(
            &actor,
            colossus_cloud::CloudProject {
                id: "without-hosted".into(),
                name: "No hosted authority".into(),
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
    let inventory = read(
        router.clone(),
        headers.clone(),
        "/api/admin/credential-store?project=without-hosted",
    )
    .await;
    assert_eq!(inventory["connections"], json!([]));
    assert!(!inventory["credentials"].as_array().unwrap().is_empty());
    let mut missing = Request::builder().uri("/api/admin/credential-store?project=missing");
    *missing.headers_mut().unwrap() = headers.clone();
    assert_eq!(
        router
            .clone()
            .oneshot(missing.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(post(router.clone(),headers.clone(),"/api/admin/credential-store/attachments/without-hosted/provider/main",json!({"revision":inventory["revision"],"credential_id":"instance-service","credential_binding":"unavailable-target"})).await.0,StatusCode::NOT_FOUND);
}

async fn stale_legacy_credentials(
    router: &Router,
    headers: &HeaderMap,
    host: &crate::hosted::HostedExecution,
    caller: &colossus_cloud::CloudCaller,
    before: &crate::config::ChatProjectConfig,
) {
    let current = host.project_config("project-a").await.unwrap().unwrap();
    assert_eq!(current.revision, before.revision);
    assert_eq!(
        current.inheritance.global_revision,
        before.inheritance.global_revision + 1
    );
    for (kind, path) in [
        (
            ChatConnectionKind::Provider,
            "/api/projects/project-a/chat/settings/providers/inherited/credential",
        ),
        (
            ChatConnectionKind::Mcp,
            "/api/projects/project-a/chat/settings/mcp/inherited/credential",
        ),
    ] {
        let old = host
            .connection_binding(caller, before, kind, "inherited")
            .unwrap()
            .unwrap();
        let binding = host
            .connection_binding(caller, &current, kind, "inherited")
            .unwrap()
            .unwrap();
        assert_ne!(old.record_id(), binding.record_id());
        for credential_binding in [None, Some(old.record_id())] {
            let mut input = json!({"revision":before.revision,"secret":"must-not-attach"});
            if let Some(binding) = credential_binding {
                input["credential_binding"] = json!(binding);
            }
            assert_eq!(
                post(router.clone(), headers.clone(), path, input).await.0,
                StatusCode::CONFLICT
            );
            assert!(
                !ChatCredentialResolver::contains(
                    host.project_vault("project-a").unwrap().as_ref(),
                    &binding
                )
                .unwrap()
            );
        }
    }
    for (kind, path) in [
        (
            ChatConnectionKind::Provider,
            "/api/projects/project-a/chat/settings/providers/inherited/credential",
        ),
        (
            ChatConnectionKind::Mcp,
            "/api/projects/project-a/chat/settings/mcp/inherited/credential",
        ),
    ] {
        let current = host.project_config("project-a").await.unwrap().unwrap();
        let binding = host
            .connection_binding(caller, &current, kind, "inherited")
            .unwrap()
            .unwrap();
        let (status,result) = post(router.clone(),headers.clone(),path,json!({"revision":current.revision,"secret":"reviewed-target-secret","credential_binding":binding.record_id()})).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert!(
            ChatCredentialResolver::contains(
                host.project_vault("project-a").unwrap().as_ref(),
                &binding
            )
            .unwrap()
        );
    }
}

async fn credential_statuses(
    router: &Router,
    headers: &HeaderMap,
    host: &crate::hosted::HostedExecution,
    owner: &colossus_cloud::CloudCaller,
) {
    let configuration = host.project_config("project-a").await.unwrap().unwrap();
    let bindings = [
        host.connection_binding(owner, &configuration, ChatConnectionKind::Provider, "main")
            .unwrap()
            .unwrap(),
        host.connection_binding(owner, &configuration, ChatConnectionKind::Mcp, "remote")
            .unwrap()
            .unwrap(),
    ];
    let vault = host.instance_vault().unwrap();
    let mut store = CredentialStore::load(vault.as_ref()).unwrap();
    store
        .create(
            StoredCredentialMetadata {
                id: "instance-service".into(),
                label: "Shared service".into(),
                kind: StoredCredentialKind::ApiKey,
            },
            &HostSecret::new("instance-service-secret").unwrap(),
        )
        .unwrap();
    for binding in &bindings {
        store.attach(binding, Some("instance-service")).unwrap();
    }
    store.commit(vault.as_ref()).unwrap();
    for expected in [true, false] {
        let providers = read(
            router.clone(),
            headers.clone(),
            "/api/projects/project-a/chat/providers",
        )
        .await;
        assert_eq!(providers["providers"][0]["authenticated"], expected);
        let connections = read(
            router.clone(),
            headers.clone(),
            "/api/projects/project-a/chat/connections",
        )
        .await;
        assert_eq!(connections["connections"][0]["authenticated"], expected);
        if expected {
            for binding in &bindings {
                // These retained project keys must remain dormant after a global disconnect.
                ChatCredentialResolver::save(
                    host.project_vault("project-a").unwrap().as_ref(),
                    binding,
                    &HostSecret::new("dormant-project-secret").unwrap(),
                )
                .unwrap();
                store.attach(binding, None).unwrap();
            }
            store.commit(vault.as_ref()).unwrap();
        }
    }
}

fn project() -> crate::config::ChatProjectConfig {
    let mut runtime =
        RuntimeConfig::controlled_chat_template("/private/hosted/global-lifecycle.redb");
    runtime.storage.keys = serde_json::from_value(json!({"kind":"environment","journal_variable":"UNUSED_GLOBAL_LIFECYCLE_JOURNAL","journal_key_id":"test","signing_variable":"UNUSED_GLOBAL_LIFECYCLE_SIGNING","anchor_path":"/private/hosted/global-lifecycle.anchor"})).unwrap();
    runtime.providers = serde_json::from_value(json!({"profiles":{"main":{"kind":"open_ai_compatible","baseUrl":"https://models.example/v1"}}})).unwrap();
    runtime.models = serde_json::from_value(json!({"profiles":{"main":{"providerProfile":"main","model":"project-model","contextWindowTokens":32768,"maxOutputTokens":4096}},"roles":{"primary":"main"}})).unwrap();
    runtime.sandbox.network_destinations = vec!["https://models.example".into()];
    runtime.mcp.servers = serde_json::from_value(json!({"remote":{"transport":"streamable_http","url":"https://models.example/mcp","allowedTools":["ping"],"credentialHeaders":{"Authorization":{"reference":"host:managed","scheme":"Bearer"}}}})).unwrap();
    let mut configuration = crate::config::ChatProjectConfig::new(runtime);
    configuration
        .provider_authentication
        .insert("main".into(), ChatAuthentication::Shared);
    configuration
        .mcp_authentication
        .insert("remote".into(), ChatAuthentication::Shared);
    configuration
}

#[tokio::test]
async fn global_updates_refresh_inheritance_and_refuse_cold_accepted_work() {
    if std::env::var_os(CHILD).is_none() {
        let output = tokio::time::timeout(
            Duration::from_secs(90),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--test-threads=1"])
                .env(CHILD, "1")
                .env(WRAPPING, "e".repeat(64))
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "isolated global lifecycle failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    use super::super::local::{PASSWORD, fixture_local, local_cookie};
    let (auth, issuer) = fixture_local().await;
    let admin = local_cookie(&auth, "administrator", PASSWORD).await;
    let user = local_cookie(&auth, "operator", PASSWORD).await;
    let administrator = auth.caller(&admin, "project-a", false).await.unwrap();
    auth.repository()
        .unwrap()
        .save_membership(&administrator, "local-user", ProjectRole::Operator, 0)
        .await
        .unwrap();
    let owner = auth.caller(&user, "project-a", false).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut config = auth.config.clone();
    tls_files(root.path(), &mut config);
    config.chat = Some(crate::config::ChatConfig {
        home: root.path().join("hosted"),
        wrapping_key_variable: WRAPPING.into(),
        shared: None,
        max_active_runtimes: 1,
        idle_seconds: 300,
        projects: BTreeMap::from([("project-a".into(), project())]),
    });
    let server = crate::server::CloudServer::with_store(config, auth.store.clone())
        .await
        .unwrap();
    let router = crate::http::router(server.state.clone());
    let host = server.state.hosted.as_ref().unwrap();
    if let Some(supervisor) = host.supervisor.lock().await.take() {
        supervisor.abort();
        let _ = supervisor.await;
    }
    credential_statuses(&router, &user, host, &owner).await;
    unconfigured_inventory(&router, &admin, &server.state).await;
    let mut catalog = crate::global_settings::types::HostedCatalog::default();
    let seed = project();
    catalog.providers.profiles.insert(
        "inherited".into(),
        seed.runtime.providers.profiles["main"].clone(),
    );
    catalog
        .provider_authentication
        .insert("inherited".into(), ChatAuthentication::Shared);
    catalog.mcp.servers.insert(
        "inherited".into(),
        seed.runtime.mcp.servers["remote"].clone(),
    );
    catalog
        .mcp_authentication
        .insert("inherited".into(), ChatAuthentication::Shared);
    catalog.variables.insert("REGION".into(), "region-a".into());
    let (status, saved) = replace(
        router.clone(),
        admin.clone(),
        serde_json::to_value(&catalog).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        host.project_config("project-a")
            .await
            .unwrap()
            .unwrap()
            .variables["REGION"],
        "region-a"
    );
    let reviewed = host.project_config("project-a").await.unwrap().unwrap();
    for (kind, path, rows) in [
        (
            ChatConnectionKind::Provider,
            "/api/projects/project-a/chat/providers",
            "providers",
        ),
        (
            ChatConnectionKind::Mcp,
            "/api/projects/project-a/chat/connections",
            "connections",
        ),
    ] {
        let snapshot = read(router.clone(), admin.clone(), path).await;
        let row = snapshot[rows]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "inherited")
            .unwrap();
        assert_eq!(
            row["credential_binding"],
            host.connection_binding(&administrator, &reviewed, kind, "inherited")
                .unwrap()
                .unwrap()
                .record_id()
        );
    }
    catalog.revision = 1;
    catalog
        .providers
        .profiles
        .get_mut("inherited")
        .unwrap()
        .base_url = Some("https://models.example/v2".into());
    catalog.mcp.servers.get_mut("inherited").unwrap().url =
        Some("https://models.example/mcp/v2".into());
    catalog.variables.insert("REGION".into(), "region-b".into());
    let (status, saved) = replace(
        router.clone(),
        admin.clone(),
        serde_json::to_value(&catalog).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let effective = host.project_config("project-a").await.unwrap().unwrap();
    assert_eq!(effective.variables["REGION"], "region-b");
    assert_eq!(effective.inheritance.global_revision, 2);
    assert!(host.live.lock().await.is_empty());
    stale_legacy_credentials(&router, &admin, host, &administrator, &reviewed).await;

    let target = server
        .state
        .repo
        .register_hosted_target(
            &administrator,
            owner.subject(),
            BTreeSet::from(["primary".into()]),
        )
        .await
        .unwrap();
    let (_,task) = server.state.repo.create_personal_thread(&owner,&target.node_id,None,serde_json::from_value(json!({"plugin_skill_ids":[],"input":[{"text":"Durably accepted without a runtime"}],"session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_depth":null,"research_sources":[],"plan_action":null,"branch":null,"max_turns":4,"idempotency_key":"global-lifecycle-queued"})).unwrap()).await.unwrap();
    assert!(host.live.lock().await.is_empty());
    assert!(host.active_project_run_status("project-a").await.unwrap());
    catalog.revision = 2;
    catalog
        .variables
        .insert("REGION".into(), "must-not-apply".into());
    let (status, _) = replace(router, admin, serde_json::to_value(&catalog).unwrap()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let retained = crate::global_settings::catalog(host).unwrap();
    assert_eq!(retained.revision, 2);
    assert_eq!(retained.variables["REGION"], "region-b");
    assert_eq!(
        server
            .state
            .repo
            .get_task(&owner, &task.task_id)
            .await
            .unwrap()
            .task_id,
        task.task_id
    );
    assert!(host.live.lock().await.is_empty());
    issuer.abort();
}
