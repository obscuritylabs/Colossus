//! Real authenticated project administration never borrows instance or another project's authority.
use super::*;

const SETTINGS_A: &str = "/api/projects/project-a/chat/settings";
const SETTINGS_B: &str = "/api/projects/project-b/chat/settings";
const STORE_A: &str = "/api/projects/project-a/chat/credential-store/project";
const STORE_B: &str = "/api/projects/project-b/chat/credential-store/project";

#[tokio::test]
async fn project_admin_http_settings_credentials_and_members_remain_project_scoped() {
    Box::pin(isolated_case()).await;
}

async fn isolated_case() {
    if std::env::var_os(CHILD).is_none() {
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    concat!(
                        "auth::tests::transport::settings_matrix::project_authorization::",
                        "project_admin_http_settings_credentials_and_members_remain_project_scoped"
                    ),
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .env(JOURNAL, "1".repeat(64))
                .env(SIGNING, "2".repeat(64))
                .env(WRAPPING, "3".repeat(64))
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("isolated project authorization deadline")
        .unwrap();
        assert!(
            output.status.success(),
            "isolated project authorization failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let services = Services::start().await;
    let mut app = App::start().await;
    mutations::publish(&app, &services.origin, AuthenticationCase::Project).await;
    app.ok(
        &app.admin,
        Method::POST,
        "/api/projects",
        json!({"id":"project-b","name":"Second project","parent_project_id":null}),
    )
    .await;
    app.member("project-b", "local-admin", ProjectRole::ProjectAdmin)
        .await;
    let (user_a, admin_a) = app.fresh_user("scoped-admin-a").await;
    let (user_b, admin_b) = app.fresh_user("scoped-admin-b").await;
    promote(&app, "project-a", &user_a).await;
    promote(&app, "project-b", &user_b).await;
    for headers in [&admin_a, &admin_b] {
        assert_eq!(
            app.read(headers, "/api/me").await["user"]["is_admin"],
            false
        );
    }

    settings_and_network(&app, &admin_a, &admin_b, &services.origin).await;
    monitoring_policy(&app, &admin_a, &admin_b).await;
    let assigned = shared_credentials(&app, &admin_a, &admin_b).await;
    members(&app, &admin_a, &admin_b, &user_a, &user_b).await;

    app.restart().await;
    let settings = app.read(&admin_a, SETTINGS_A).await;
    assert_eq!(
        settings["providers"]["profiles"]["matrix"]["timeoutMs"],
        6000
    );
    assert_eq!(
        settings["connection_network_policy"]["allowed_origins"],
        json!([services.origin])
    );
    assert_eq!(app.read(&admin_a, STORE_A).await, assigned);
    assert_eq!(
        app.read(&admin_b, SETTINGS_B).await["providers"]["profiles"]["matrix"]["timeoutMs"],
        5000
    );
    assert_eq!(app.read(&admin_b, STORE_B).await["credentials"], json!([]));
    assert_eq!(
        app.read(&admin_b, "/api/projects/project-b/members").await["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["user_id"] == user_b)
            .unwrap()["role"],
        "project_admin"
    );
    assert_eq!(
        app.request(&admin_b, Method::GET, "/api/projects/project-a", None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    app.stop().await;
    services.stop().await;
}

async fn promote(app: &App, project: &str, user: &str) {
    let member = membership(app, &app.admin, project, user).await;
    app.ok(
        &app.admin,
        Method::PATCH,
        &format!("/api/projects/{project}/members/{user}"),
        json!({"user_id":user,"role":"project_admin","revision":member["revision"]}),
    )
    .await;
}

async fn membership(app: &App, headers: &HeaderMap, project: &str, user: &str) -> Value {
    app.read(headers, &format!("/api/projects/{project}/members"))
        .await["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["user_id"] == user)
        .expect("fixture membership exists")
        .clone()
}

async fn forbidden(
    app: &App,
    headers: &HeaderMap,
    method: Method,
    path: &str,
    body: Option<Value>,
) {
    let (status, value) = app.request(headers, method.clone(), path, body).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {value}");
}

async fn settings_and_network(app: &App, admin_a: &HeaderMap, admin_b: &HeaderMap, origin: &str) {
    let initial = app.read(admin_a, SETTINGS_A).await;
    let other = app.read(admin_b, SETTINGS_B).await;
    assert_eq!(initial["connection_network_policy"]["mode"], "allow_all");
    let mut edit = without_status(initial);
    edit["providers"]["profiles"]["matrix"]["timeoutMs"] = json!(6000);
    edit["models"]["profiles"]["matrix-model"]["model"] = json!("project-admin-model");
    edit["mcp"]["servers"]["matrix"]["allowedTools"] = json!(["p*"]);
    edit["search"]["profiles"]["matrix"]["timeoutMs"] = json!(6000);
    edit["variables"]["MATRIX"] = json!("project-admin");
    edit["connection_network_policy"] = json!({"mode":"allow_list","allowed_origins":[]});
    forbidden(app, admin_b, Method::PUT, SETTINGS_A, Some(edit.clone())).await;
    forbidden(
        app,
        admin_a,
        Method::PUT,
        SETTINGS_B,
        Some(without_status(other.clone())),
    )
    .await;
    let mut no_csrf = admin_a.clone();
    no_csrf.remove("x-colossus-csrf");
    forbidden(app, &no_csrf, Method::PUT, SETTINGS_A, Some(edit.clone())).await;
    app.ok(admin_a, Method::PUT, SETTINGS_A, edit.clone()).await;
    let saved = app.read(admin_a, SETTINGS_A).await;
    assert_eq!(saved["providers"]["profiles"]["matrix"]["timeoutMs"], 6000);
    assert_eq!(
        saved["models"]["profiles"]["matrix-model"]["model"],
        "project-admin-model"
    );
    assert_eq!(
        saved["mcp"]["servers"]["matrix"]["allowedTools"],
        json!(["p*"])
    );
    assert_eq!(saved["search"]["profiles"]["matrix"]["timeoutMs"], 6000);
    assert_eq!(saved["variables"]["MATRIX"], "project-admin");
    for kind in ["providers", "mcp", "search"] {
        assert_eq!(
            saved["connection_availability"][kind]["matrix"]["enabled"],
            false
        );
    }
    assert_eq!(
        app.request(admin_a, Method::PUT, SETTINGS_A, Some(edit))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(app.read(admin_a, SETTINGS_A).await, saved);
    let mut allowed = without_status(saved);
    allowed["connection_network_policy"] = json!({"mode":"allow_list","allowed_origins":[origin]});
    app.ok(admin_a, Method::PUT, SETTINGS_A, allowed).await;
    let allowed = app.read(admin_a, SETTINGS_A).await;
    for kind in ["providers", "mcp", "search"] {
        assert_eq!(
            allowed["connection_availability"][kind]["matrix"]["enabled"],
            true
        );
    }
    assert_eq!(app.read(admin_b, SETTINGS_B).await, other);
    let mut global = app.read(&app.admin, "/api/admin/hosted-settings").await;
    assert_eq!(global["providers"]["profiles"]["matrix"]["timeoutMs"], 5000);
    global.as_object_mut().unwrap().remove("credential_status");
    global.as_object_mut().unwrap().remove("telemetry");
    forbidden(
        app,
        admin_a,
        Method::PUT,
        "/api/admin/hosted-settings",
        Some(global),
    )
    .await;
}

async fn monitoring_policy(app: &App, admin_a: &HeaderMap, admin_b: &HeaderMap) {
    let path_a = "/api/projects/project-a/policy";
    let path_b = "/api/projects/project-b/policy";
    let mut policy = app.read(admin_a, path_a).await;
    let other = app.read(admin_b, path_b).await;
    policy["required_sandbox_profile"] = json!("offline");
    policy["allowed_approval_modes"] = json!(["ask"]);
    policy["allowed_tools"] = json!(["mcp.call"]);
    forbidden(app, admin_b, Method::PATCH, path_a, Some(policy.clone())).await;
    forbidden(app, admin_a, Method::PATCH, path_b, Some(other.clone())).await;
    let saved = app.ok(admin_a, Method::PATCH, path_a, policy.clone()).await;
    assert_eq!(saved["required_sandbox_profile"], "offline");
    assert_eq!(saved["allowed_approval_modes"], json!(["ask"]));
    assert_eq!(saved["allowed_tools"], json!(["mcp.call"]));
    assert_eq!(
        app.request(admin_a, Method::PATCH, path_a, Some(policy))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(app.read(admin_a, path_a).await, saved);
    assert_eq!(app.read(admin_b, path_b).await, other);
}

async fn shared_credentials(app: &App, admin_a: &HeaderMap, admin_b: &HeaderMap) -> Value {
    let initial_a = app.read(admin_a, STORE_A).await;
    let initial_b = app.read(admin_b, STORE_B).await;
    let create = json!({"revision":initial_a["revision"],"label":"Project-admin key","kind":"api_key","secret":"matrix-scoped-project-token"});
    forbidden(app, admin_b, Method::POST, STORE_A, Some(create.clone())).await;
    forbidden(app, admin_a, Method::POST, STORE_B, Some(create.clone())).await;
    let created = app.ok(admin_a, Method::POST, STORE_A, create.clone()).await;
    assert_eq!(
        app.request(admin_a, Method::POST, STORE_A, Some(create))
            .await
            .0,
        StatusCode::CONFLICT
    );
    for kind in ["provider", "mcp", "search"] {
        let inventory = app.read(admin_a, STORE_A).await;
        let target = connection(&inventory, kind);
        let attach = json!({"revision":inventory["revision"],"credential_id":created["id"],"credential_binding":target["credential_binding"]});
        let path = format!("{STORE_A}/attachments/{kind}/matrix");
        forbidden(app, admin_b, Method::POST, &path, Some(attach.clone())).await;
        app.ok(admin_a, Method::POST, &path, attach).await;
        assert_eq!(
            connection(&app.read(admin_a, STORE_A).await, kind)["credential_id"],
            created["id"]
        );

        // Even an administrator of B cannot attach the named secret stored in A.
        let other_target = connection(&initial_b, kind);
        let other_attach = json!({"revision":initial_b["revision"],"credential_id":created["id"],"credential_binding":other_target["credential_binding"]});
        let other_path = format!("{STORE_B}/attachments/{kind}/matrix");
        forbidden(
            app,
            admin_a,
            Method::POST,
            &other_path,
            Some(other_attach.clone()),
        )
        .await;
        assert_eq!(
            app.request(admin_b, Method::POST, &other_path, Some(other_attach))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(app.read(admin_b, STORE_B).await, initial_b);
    }
    let assigned = app.read(admin_a, STORE_A).await;
    assert_eq!(assigned["credentials"].as_array().unwrap().len(), 1);
    assert_eq!(
        assigned["credentials"][0]["used_by"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(!assigned.to_string().contains("matrix-scoped-project-token"));
    forbidden(app, admin_b, Method::GET, STORE_A, None).await;
    forbidden(app, admin_a, Method::GET, STORE_B, None).await;
    assigned
}

fn connection(inventory: &Value, kind: &str) -> Value {
    inventory["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == kind && row["id"] == "matrix")
        .expect("shared connection exists")
        .clone()
}

async fn members(app: &App, admin_a: &HeaderMap, admin_b: &HeaderMap, user_a: &str, user_b: &str) {
    let path_a = format!("/api/projects/project-a/members/{user_b}");
    let before_a = membership(app, admin_a, "project-a", user_b).await;
    let before_b = membership(app, admin_b, "project-b", user_b).await;
    let edit = json!({"user_id":user_b,"role":"viewer","revision":before_a["revision"]});
    forbidden(app, admin_b, Method::PATCH, &path_a, Some(edit.clone())).await;
    let path_b = format!("/api/projects/project-b/members/{user_b}");
    forbidden(
        app,
        admin_a,
        Method::PATCH,
        &path_b,
        Some(json!({"user_id":user_b,"role":"viewer","revision":before_b["revision"]})),
    )
    .await;
    let saved = app.ok(admin_a, Method::PATCH, &path_a, edit.clone()).await["member"].clone();
    assert_eq!(saved["role"], "viewer");
    assert_eq!(
        membership(app, admin_a, "project-a", user_b).await["revision"],
        saved["revision"]
    );
    assert_eq!(
        app.request(admin_a, Method::PATCH, &path_a, Some(edit))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        membership(app, admin_b, "project-b", user_b).await,
        before_b
    );
    assert_eq!(
        membership(app, admin_a, "project-a", user_a).await["role"],
        "project_admin"
    );
    let delete_a = format!("{path_a}?revision={}", saved["revision"]);
    forbidden(app, admin_b, Method::DELETE, &delete_a, None).await;
    forbidden(
        app,
        admin_a,
        Method::DELETE,
        &format!("{path_b}?revision={}", before_b["revision"]),
        None,
    )
    .await;
    assert_eq!(
        app.request(
            admin_a,
            Method::DELETE,
            &format!("{path_a}?revision={}", before_a["revision"]),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.request(admin_a, Method::DELETE, &delete_a, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        !app.read(admin_a, "/api/projects/project-a/members").await["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["user_id"] == user_b)
    );
    assert_eq!(
        membership(app, admin_b, "project-b", user_b).await,
        before_b
    );
    // The already-issued login cookie loses A access immediately, while B administration survives.
    forbidden(app, admin_b, Method::GET, "/api/projects/project-a", None).await;
    assert_eq!(
        app.read(admin_b, SETTINGS_B).await["providers"]["profiles"]["matrix"]["timeoutMs"],
        5000
    );
}
