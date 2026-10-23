//! Global → project → fresh-user settings exercise real private runtime effects.
use super::*;
use axum::http::{Method, StatusCode};
use colossus_cloud::ProjectRole;
use services::Services;

mod effects;
mod fixture;
mod lifecycles;
mod mutations;
mod project_authorization;
mod project_metadata;
mod services;
mod variables;
use fixture::App;

const CHILD: &str = "COLOSSUS_SETTINGS_MATRIX_CHILD";
const JOURNAL: &str = "COLOSSUS_SETTINGS_MATRIX_JOURNAL";
const SIGNING: &str = "COLOSSUS_SETTINGS_MATRIX_SIGNING";
const WRAPPING: &str = "COLOSSUS_SETTINGS_MATRIX_WRAPPING";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthenticationCase {
    None,
    Personal,
    Project,
    Global,
}
impl AuthenticationCase {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Personal => "personal",
            Self::Project => "project",
            Self::Global => "global",
        }
    }
    fn authentication(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Personal => "personal",
            Self::Project | Self::Global => "shared",
        }
    }
    fn token(self, project: &str, kind: &str) -> String {
        format!("matrix-{}-{project}-{kind}", self.name())
    }
}

macro_rules! onboarding_case {
    ($name:ident, $mode:ident) => {
        #[tokio::test]
        async fn $name() {
            Box::pin(run_case(
                AuthenticationCase::$mode,
                concat!(
                    "auth::tests::transport::settings_matrix::",
                    stringify!($name)
                ),
            ))
            .await;
        }
    };
}
onboarding_case!(settings_inheritance_onboarding_matrix_none, None);
onboarding_case!(settings_inheritance_onboarding_matrix_personal, Personal);
onboarding_case!(settings_inheritance_onboarding_matrix_project, Project);
onboarding_case!(settings_inheritance_onboarding_matrix_global, Global);

async fn run_case(mode: AuthenticationCase, test: &str) {
    if std::env::var_os(CHILD).is_none() {
        let output = tokio::time::timeout(
            Duration::from_secs(240),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--test-threads=1"])
                .env(CHILD, "1")
                .env(JOURNAL, "1".repeat(64))
                .env(SIGNING, "2".repeat(64))
                .env(WRAPPING, "3".repeat(64))
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("isolated settings matrix deadline")
        .unwrap();
        assert!(
            output.status.success(),
            "isolated settings matrix failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    if mode == AuthenticationCase::None {
        invalid_binding_is_not_admitted().await;
    }
    let services = Services::start().await;
    let mut app = App::start().await;
    eprintln!("settings onboarding matrix: {} authentication", mode.name());
    mutations::publish(&app, &services.origin, mode).await;
    if mode == AuthenticationCase::None {
        invalid_search_headers_are_not_persisted(&app).await;
    }
    app.ok(&app.admin, Method::POST, "/api/projects", json!({"id":"project-b","name":"Created after catalog publication","parent_project_id":null})).await;
    app.member("project-b", "local-admin", ProjectRole::ProjectAdmin)
        .await;
    if mode == AuthenticationCase::None {
        project_metadata::verify(&mut app).await;
    }
    mutations::adopt_global_primary(&app).await;
    mutations::inheritance_controls(&app).await;
    for project in ["project-a", "project-b"] {
        mutations::assert_inherited(&app, project).await;
    }
    let (user, headers) = app.fresh_user(&format!("matrix-{}", mode.name())).await;
    assert_eq!(
        app.request(
            &headers,
            Method::GET,
            "/api/projects/project-a/chat/settings",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut no_csrf = app.admin.clone();
    no_csrf.remove("x-colossus-csrf");
    let proposal = app.settings("project-a").await;
    assert_eq!(
        app.request(
            &no_csrf,
            Method::PUT,
            "/api/projects/project-a/chat/settings",
            Some(without_status(proposal))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    if mode != AuthenticationCase::None {
        services.clear();
        let failed = effects::run(
            &app,
            &headers,
            "project-a",
            "[matrix:plain]",
            &format!("{}-missing", mode.name()),
        )
        .await;
        assert!(
            model_failed(&failed),
            "a fresh user must not acquire an unassigned credential"
        );
        assert!(
            services.observations().is_empty(),
            "missing credentials must fail before provider traffic"
        );
    }
    let global_ids = mutations::prepare_credentials(&app, &headers, mode, "project-a", None).await;
    for project in ["project-a", "project-b"] {
        if project == "project-b" {
            if mode != AuthenticationCase::None {
                services.clear();
                let failed = effects::run(
                    &app,
                    &headers,
                    project,
                    "[matrix:plain]",
                    &format!("{}-other-project-unassigned", mode.name()),
                )
                .await;
                assert!(model_failed(&failed));
                assert!(
                    services.observations().is_empty(),
                    "project-a custody must not leak to project-b"
                );
            }
            mutations::prepare_credentials(&app, &headers, mode, project, global_ids.as_ref())
                .await;
        }
        for marker in ["[matrix:mcp]", "[matrix:search]"] {
            eprintln!(
                "settings onboarding effect: {} {project} {marker}",
                mode.name()
            );
            services.clear();
            let task = effects::run(
                &app,
                &headers,
                project,
                marker,
                &format!("{}-{project}-{marker}", mode.name()),
            )
            .await;
            assert!(
                effects::successful(&task),
                "{mode:?} {project} {marker}: {:?}",
                task.snapshot
            );
            assert_effects(&services, mode, project, marker);
            assert_eq!(task.subject, user);
        }
    }
    mutations::negative_boundaries(&mut app, &services, &headers, mode).await;
    app.stop().await;
    services.stop().await;
}

async fn invalid_binding_is_not_admitted() {
    let app = App::with_invalid_binding(true).await;
    let request = json!({
        "plugin_skill_ids":[],"input":[{"text":"No effect may start"}],"session_id":null,"end_user_id":null,
        "role":"primary","mode":"execute","research_sources":[],"max_turns":1,"idempotency_key":"invalid-binding"
    });
    let node = format!(
        "chat-{}",
        colossus_cloud_chat::ChatRuntime::identity_for("project-a", "local-admin").unwrap()
    );
    for (path, body) in [
        (
            "/api/projects/project-a/chat/threads",
            json!({"request":request}),
        ),
        (
            "/api/projects/project-a/tasks",
            json!({"node_id":node,"request":request}),
        ),
        (
            "/api/projects/project-a/threads",
            json!({"node_id":node,"title":null,"request":request}),
        ),
    ] {
        let (status, _) = app
            .request(&app.admin, Method::POST, path, Some(body))
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} must reject invalid custody before creating work"
        );
    }
    assert!(
        !app.state
            .hosted
            .as_ref()
            .unwrap()
            .active_project_run_status("project-a")
            .await
            .unwrap(),
        "a binding error must be returned before a durable task is queued"
    );
    let tasks = app.read(&app.admin, "/api/projects/project-a/tasks").await;
    assert_eq!(tasks["tasks"], json!([]));
    assert_eq!(
        app.read(&app.admin, "/api/projects/project-a/chat/threads")
            .await["threads"],
        json!([])
    );
    app.stop().await;
}

async fn invalid_search_headers_are_not_persisted(app: &App) {
    let global = app.read(&app.admin, "/api/admin/hosted-settings").await;
    let mut invalid = global.clone();
    for metadata in ["credential_status", "telemetry"] {
        invalid.as_object_mut().unwrap().remove(metadata);
    }
    invalid["search"]["profiles"]["matrix"]["userAgent"] = json!("Colossus\r\nInjected: invalid");
    assert_eq!(
        app.request(
            &app.admin,
            Method::PUT,
            "/api/admin/hosted-settings",
            Some(invalid)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.read(&app.admin, "/api/admin/hosted-settings").await["revision"],
        global["revision"]
    );
    let project = app.settings("project-a").await;
    let mut invalid = without_status(project.clone());
    invalid["search"]["profiles"]["matrix"]["userAgent"] = json!("Colossus\u{7f}");
    assert_eq!(
        app.request(
            &app.admin,
            Method::PUT,
            "/api/projects/project-a/chat/settings",
            Some(invalid)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.settings("project-a").await["revision"],
        project["revision"]
    );
}

fn without_status(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("credential_status");
    value
}
fn model_failed(task: &colossus_cloud::CloudTask) -> bool {
    task.dispatch_error.is_some()
        || task
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.run.status == colossus_sdk::RunStatus::Failed)
}
fn assert_effects(services: &Services, mode: AuthenticationCase, project: &str, marker: &str) {
    assert_observations(services, mode, project, marker, None);
}
fn assert_observations(
    services: &Services,
    mode: AuthenticationCase,
    project: &str,
    marker: &str,
    rotated_kind: Option<&str>,
) {
    let observations = services.observations();
    assert!(
        observations
            .iter()
            .any(|row| row.path == "/v1/chat/completions")
    );
    if marker != "[matrix:plain]" {
        let destination = if marker == "[matrix:mcp]" {
            "/mcp"
        } else {
            "/search"
        };
        assert!(observations.iter().any(|row| row.path == destination));
    }
    for row in observations {
        assert_eq!(
            row.method,
            if row.path == "/search" || row.path == "/v1/models" {
                "GET"
            } else {
                "POST"
            }
        );
        let kind = match row.path.as_str() {
            "/v1/chat/completions" | "/v1/models" => "provider",
            "/mcp" => "mcp",
            "/search" => "search",
            path => panic!("unexpected effect {path}"),
        };
        let token_project = if mode == AuthenticationCase::Global {
            "project-a"
        } else {
            project
        };
        let suffix = if rotated_kind == Some(kind) {
            "-rotated"
        } else {
            ""
        };
        let expected = (mode != AuthenticationCase::None)
            .then(|| format!("Bearer {}{suffix}", mode.token(token_project, kind)));
        assert_eq!(
            row.authorization, expected,
            "{mode:?} {project} {}",
            row.path
        );
        if row.path == "/v1/chat/completions" {
            assert_eq!(row.payload.as_ref().unwrap()["model"], "matrix-model");
        }
    }
}
