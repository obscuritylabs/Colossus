//! HTTP catalog, inheritance and credential transitions used by every auth case.
use super::*;
use std::collections::BTreeMap;

pub(super) async fn publish(app: &App, origin: &str, mode: AuthenticationCase) {
    app.idle().await;
    let current = app.read(&app.admin, "/api/admin/hosted-settings").await;
    let authentication = mode.authentication();
    let headers = if mode == AuthenticationCase::None {
        json!({})
    } else {
        json!({"Authorization":{"reference":"host:managed","scheme":"Bearer"}})
    };
    let search_reference = (mode != AuthenticationCase::None).then_some("host:managed");
    let selector = match mode {
        AuthenticationCase::None => "ping",
        AuthenticationCase::Personal => "p*",
        AuthenticationCase::Project => "*",
        AuthenticationCase::Global => "*ing",
    };
    let catalog = json!({"revision":current["revision"],
        "labels":{"providers":{"matrix":"Global matrix provider"},"models":{"matrix-model":"Global matrix model"},"search":{"matrix":"Global matrix search"}},
        "providers":{"profiles":{"matrix":{"kind":"open_ai_compatible","baseUrl":format!("{origin}/v1"),"timeoutMs":5000}}},
        "models":{"profiles":{"matrix-model":{"providerProfile":"matrix","model":"matrix-model","contextWindowTokens":32768,"maxOutputTokens":4096,"capabilities":{"toolCalls":true,"streaming":true}}},"roles":{"primary":"matrix-model","subagent_default":"matrix-model"}},
        "mcp":{"oauthCredentialStore":"platform","servers":{"matrix":{"transport":"streamable_http","url":format!("{origin}/mcp"),"protocolVersion":"2026-07-28","allowStateless":true,"allowedTools":[selector],"credentialHeaders":headers}}},
        "search":{"profiles":{"matrix":{"kind":"searxng","endpoint":format!("{origin}/search"),"credentialReference":search_reference,"authHeader":"Authorization","timeoutMs":5000}},"roles":{"agent":"matrix","research":"matrix"}},
        "provider_authentication":{"matrix":authentication},"mcp_authentication":{"matrix":authentication},"search_authentication":{"matrix":authentication},
        "variables":{"MATRIX":"global"},"observability":{}});
    let saved = app
        .ok(
            &app.admin,
            Method::PUT,
            "/api/admin/hosted-settings",
            catalog,
        )
        .await;
    assert_eq!(
        saved["revision"].as_u64(),
        current["revision"].as_u64().map(|revision| revision + 1)
    );
}

pub(super) async fn assert_inherited(app: &App, project: &str) {
    let settings = app.settings(project).await;
    for (section, id) in [
        ("providers", "matrix"),
        ("models", "matrix-model"),
        ("mcp", "matrix"),
        ("search", "matrix"),
        ("roles", "subagent_default"),
        ("roles", "primary"),
    ] {
        assert_eq!(
            settings["inheritance"]["provenance"][section][id], "global",
            "{project} {section}"
        );
        assert_eq!(
            settings["connection_availability"][section][id]["enabled"],
            true
        );
    }
    assert_eq!(settings["connection_network_policy"]["mode"], "allow_all");
    assert_eq!(settings["variables"]["MATRIX"], "global");
}

/// Operator runtime seeds are explicit project overrides, even when they use
/// the Echo template. Resetting the reviewed route is the explicit opt-in to the
/// published global Primary; subsequent fresh-user chats use ordinary Primary.
pub(super) async fn adopt_global_primary(app: &App) {
    for project in ["project-a", "project-b"] {
        let mut settings = app.settings(project).await;
        assert_eq!(settings["models"]["roles"]["primary"], "echo");
        assert_eq!(
            settings["inheritance"]["provenance"]["roles"]["primary"],
            "project"
        );
        settings["inheritance"] = json!({"global_revision":settings["inheritance"]["global_revision"],"reset":{"roles":["primary"]}});
        app.save_settings(project, settings).await;
        let adopted = app.settings(project).await;
        assert_eq!(adopted["models"]["roles"]["primary"], "matrix-model");
        assert_eq!(
            adopted["inheritance"]["provenance"]["roles"]["primary"],
            "global"
        );
    }
}

pub(super) async fn inheritance_controls(app: &App) {
    let mut settings = app.settings("project-a").await;
    settings["providers"]["profiles"]["matrix"]["timeoutMs"] = json!(6000);
    settings["models"]["profiles"]["matrix-model"]["model"] = json!("project-override");
    settings["mcp"]["servers"]["matrix"]["allowedTools"] = json!(["project_*"]);
    settings["search"]["profiles"]["matrix"]["timeoutMs"] = json!(6000);
    settings["variables"]["MATRIX"] = json!("project");
    app.save_settings("project-a", settings).await;
    let mut settings = app.settings("project-a").await;
    for (section, id) in [
        ("providers", "matrix"),
        ("models", "matrix-model"),
        ("mcp", "matrix"),
        ("search", "matrix"),
        ("variables", "MATRIX"),
    ] {
        assert_eq!(
            settings["inheritance"]["provenance"][section][id],
            "project"
        );
    }
    settings["inheritance"] = json!({"global_revision":settings["inheritance"]["global_revision"],"reset":{"providers":["matrix"],"models":["matrix-model"],"mcp":["matrix"],"search":["matrix"],"variables":["MATRIX"]}});
    app.save_settings("project-a", settings).await;
    assert_inherited(app, "project-a").await;
    let mut settings = app.settings("project-a").await;
    for (root, child, id) in [
        ("providers", "profiles", "matrix"),
        ("models", "profiles", "matrix-model"),
        ("mcp", "servers", "matrix"),
        ("search", "profiles", "matrix"),
    ] {
        settings[root][child].as_object_mut().unwrap().remove(id);
    }
    settings["models"]["roles"]
        .as_object_mut()
        .unwrap()
        .remove("subagent_default");
    settings["models"]["roles"]["primary"] = json!("echo");
    settings["search"]["roles"] = json!({});
    settings["variables"]
        .as_object_mut()
        .unwrap()
        .remove("MATRIX");
    for section in ["providers", "models", "search"] {
        settings["labels"][section].as_object_mut().unwrap().clear();
    }
    for authentication in [
        "provider_authentication",
        "mcp_authentication",
        "search_authentication",
    ] {
        settings[authentication]
            .as_object_mut()
            .unwrap()
            .remove("matrix");
    }
    app.save_settings("project-a", settings).await;
    let mut disabled = app.settings("project-a").await;
    for (section, id) in [
        ("providers", "matrix"),
        ("models", "matrix-model"),
        ("mcp", "matrix"),
        ("search", "matrix"),
        ("variables", "MATRIX"),
    ] {
        assert!(
            disabled["inheritance"]["disabled"][section]
                .as_array()
                .unwrap()
                .contains(&json!(id))
        );
    }
    disabled["inheritance"] = json!({"global_revision":disabled["inheritance"]["global_revision"],"reset":{"providers":["matrix"],"models":["matrix-model"],"mcp":["matrix"],"search":["matrix"],"variables":["MATRIX"],"roles":["primary","subagent_default","search:agent","search:research"]}});
    app.save_settings("project-a", disabled).await;
    assert_inherited(app, "project-a").await;
}

pub(super) fn store_base(mode: AuthenticationCase, project: &str) -> String {
    match mode {
        AuthenticationCase::Personal => {
            format!("/api/projects/{project}/chat/credential-store/personal")
        }
        AuthenticationCase::Project => {
            format!("/api/projects/{project}/chat/credential-store/project")
        }
        AuthenticationCase::Global => "/api/admin/credential-store".into(),
        AuthenticationCase::None => unreachable!(),
    }
}
pub(super) fn inventory_path(mode: AuthenticationCase, project: &str) -> String {
    let base = store_base(mode, project);
    if mode == AuthenticationCase::Global {
        format!("{base}?project={project}")
    } else {
        base
    }
}

pub(super) async fn prepare_credentials(
    app: &App,
    user: &HeaderMap,
    mode: AuthenticationCase,
    project: &str,
    global_ids: Option<&BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    if mode == AuthenticationCase::None {
        return None;
    }
    app.idle().await;
    let actor = if mode == AuthenticationCase::Personal {
        user
    } else {
        &app.admin
    };
    let base = store_base(mode, project);
    let inventory = inventory_path(mode, project);
    let mut ids = global_ids.cloned().unwrap_or_default();
    if global_ids.is_none() {
        for kind in ["provider", "mcp", "search"] {
            let snapshot = app.read(actor, &inventory).await;
            let target = snapshot["connections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == "matrix" && row["kind"] == kind)
                .unwrap();
            let token = mode.token(project, kind);
            let secret = if kind == "search" {
                format!("Bearer {token}")
            } else {
                token.clone()
            };
            let mut body = json!({"revision":snapshot["revision"],"label":format!("{kind} matrix credential"),"kind":"api_key","secret":secret});
            if mode != AuthenticationCase::Global {
                body["attachment"] = json!({"kind":kind,"connection":"matrix","credential_binding":target["credential_binding"]});
            }
            let created = app.ok(actor, Method::POST, &base, body).await;
            ids.insert(kind.into(), created["id"].as_str().unwrap().into());
            assert!(
                !app.read(actor, &inventory)
                    .await
                    .to_string()
                    .contains(&token)
            );
        }
    }
    if mode == AuthenticationCase::Global {
        if global_ids.is_none() {
            let failed = effects::run(
                app,
                user,
                project,
                "[matrix:plain]",
                "global-key-present-unassigned",
            )
            .await;
            assert!(
                model_failed(&failed),
                "stored global secrets do not grant a connection"
            );
            app.idle().await;
            // A project key exists, but an explicit global grant must take precedence.
            prepare_project_shadow(app, project).await;
        }
        for kind in ["provider", "mcp", "search"] {
            let snapshot = app.read(&app.admin, &inventory).await;
            let target = snapshot["connections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == "matrix" && row["kind"] == kind)
                .unwrap();
            app.ok(&app.admin, Method::POST, &format!("{base}/attachments/{project}/{kind}/matrix"), json!({"revision":snapshot["revision"],"credential_id":ids[kind],"credential_binding":target["credential_binding"]})).await;
        }
        Some(ids)
    } else {
        None
    }
}

async fn prepare_project_shadow(app: &App, project: &str) {
    let base = store_base(AuthenticationCase::Project, project);
    for kind in ["provider", "mcp", "search"] {
        let snapshot = app.read(&app.admin, &base).await;
        let target = snapshot["connections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "matrix" && row["kind"] == kind)
            .unwrap();
        let token = format!("matrix-shadow-{kind}");
        let secret = if kind == "search" {
            format!("Bearer {token}")
        } else {
            token
        };
        app.ok(&app.admin, Method::POST, &base, json!({"revision":snapshot["revision"],"label":"Dormant project credential","kind":"api_key","secret":secret,"attachment":{"kind":kind,"connection":"matrix","credential_binding":target["credential_binding"]}})).await;
    }
}

pub(super) async fn negative_boundaries(
    app: &mut App,
    services: &Services,
    user: &HeaderMap,
    mode: AuthenticationCase,
) {
    app.idle().await;
    let old = app.settings("project-a").await;
    let mut restricted = old.clone();
    restricted["connection_network_policy"] = json!({"mode":"allow_list","allowed_origins":[]});
    app.save_settings("project-a", restricted).await;
    services.clear();
    let blocked = app.request(user, Method::POST, "/api/projects/project-a/chat/threads", Some(json!({"request":{"plugin_skill_ids":[],"input":[{"text":"[matrix:plain]"}],"session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_sources":[],"max_turns":8,"idempotency_key":format!("{}-blocked",mode.name())}}))).await;
    assert_eq!(blocked.0, StatusCode::FORBIDDEN);
    assert!(services.observations().is_empty());
    blocked_continuations(app, user, mode).await;
    assert_eq!(
        app.request(
            &app.admin,
            Method::PUT,
            "/api/projects/project-a/chat/settings",
            Some(without_status(old))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut restored = app.settings("project-a").await;
    restored["connection_network_policy"] = json!({"mode":"allow_all","allowed_origins":[]});
    app.save_settings("project-a", restored).await;
    variables::write(app, user).await;
    app.restart().await;
    assert_inherited(app, "project-a").await;
    services.clear();
    let restarted = effects::run(
        app,
        user,
        "project-a",
        "[matrix:plain]",
        &format!("{}-restart", mode.name()),
    )
    .await;
    assert!(effects::successful(&restarted));
    assert_provider(services, mode, "project-a", false);
    let (_, other) = app
        .fresh_user(&format!("matrix-other-{}", mode.name()))
        .await;
    variables::retained_and_isolated(app, user, &other).await;
    let private = app
        .read(
            &other,
            "/api/projects/project-a/chat/credential-store/personal",
        )
        .await;
    assert_eq!(private["credentials"], json!([]));
    services.clear();
    let other_task = effects::run(
        app,
        &other,
        "project-a",
        "[matrix:plain]",
        &format!("{}-other-user", mode.name()),
    )
    .await;
    if mode == AuthenticationCase::Personal {
        assert!(model_failed(&other_task));
        assert!(services.observations().is_empty());
        let owner = app.read(user, &inventory_path(mode, "project-a")).await;
        let target = private["connections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "provider")
            .unwrap();
        assert_eq!(app.request(&other, Method::POST, "/api/projects/project-a/chat/credential-store/personal/attachments/provider/matrix", Some(json!({"revision":private["revision"],"credential_id":owner["credentials"][0]["id"],"credential_binding":target["credential_binding"]}))).await.0, StatusCode::NOT_FOUND);
    } else {
        assert!(effects::successful(&other_task));
        assert_provider(services, mode, "project-a", false);
        if mode != AuthenticationCase::None {
            for marker in ["[matrix:mcp]", "[matrix:search]"] {
                services.clear();
                let task = effects::run(
                    app,
                    &other,
                    "project-a",
                    marker,
                    &format!("{}-preassigned-{marker}", mode.name()),
                )
                .await;
                assert!(effects::successful(&task));
                assert_effects(services, mode, "project-a", marker);
            }
        }
    }
    if mode != AuthenticationCase::None {
        lifecycles::exercise(app, services, user, mode).await;
    }
}

async fn blocked_continuations(app: &App, user: &HeaderMap, mode: AuthenticationCase) {
    let threads = app.read(user, "/api/projects/project-a/chat/threads").await;
    let thread = threads["threads"].as_array().unwrap().first().unwrap();
    let id = thread["thread_id"].as_str().unwrap();
    let path = format!("/api/projects/project-a/threads/{id}");
    let before = app.read(user, &path).await;
    let request = json!({"plugin_skill_ids":[],"input":[{"text":"No blocked continuation"}],
        "session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_sources":[],
        "max_turns":1,"idempotency_key":format!("{}-blocked-continuation",mode.name())});
    let branches = format!("{path}/branches");
    for (route, body) in [
        (
            format!("{path}/messages"),
            json!({"expected_revision":before["thread"]["revision"],"request":request}),
        ),
        (
            branches.clone(),
            json!({"source_task_id":before["tasks"][0]["task_id"],"kind":"fork",
            "expected_revision":before["thread"]["revision"],"title":null,"request":request}),
        ),
    ] {
        assert_eq!(
            app.request(user, Method::POST, &route, Some(body)).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let after = app.read(user, &path).await;
    assert_eq!(after["thread"]["revision"], before["thread"]["revision"]);
    assert_eq!(after["tasks"], before["tasks"]);
    assert_eq!(app.read(user, &branches).await["threads"], json!([]));
}

fn assert_provider(services: &Services, mode: AuthenticationCase, project: &str, rotated: bool) {
    let rows = services.observations();
    assert!(rows.iter().any(|row| row.path == "/v1/chat/completions"));
    let suffix = if rotated { "-rotated" } else { "" };
    let expected = (mode != AuthenticationCase::None)
        .then(|| format!("Bearer {}{suffix}", mode.token(project, "provider")));
    assert!(rows.iter().all(|row| row.authorization == expected));
}
