//! Every credential purpose exercises rotation, removal and explicit re-enrollment.
use super::{
    mutations::{inventory_path, store_base},
    *,
};

pub(super) async fn exercise(
    app: &App,
    services: &Services,
    user: &HeaderMap,
    mode: AuthenticationCase,
) {
    for (kind, marker, destination) in [
        ("provider", "[matrix:plain]", "/v1/chat/completions"),
        ("mcp", "[matrix:mcp]", "/mcp"),
        ("search", "[matrix:search]", "/search"),
    ] {
        app.idle().await;
        let actor = if mode == AuthenticationCase::Personal {
            user
        } else {
            &app.admin
        };
        let inventory = inventory_path(mode, "project-a");
        let base = store_base(mode, "project-a");
        let snapshot = app.read(actor, &inventory).await;
        let id = snapshot["credentials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| {
                row["used_by"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|usage| usage["kind"] == kind && usage["id"] == "matrix")
            })
            .unwrap()["id"]
            .as_str()
            .unwrap();
        let rotated = format!("{}-rotated", mode.token("project-a", kind));
        app.ok(
            actor,
            Method::POST,
            &format!("{base}/{id}"),
            json!({
                "revision":snapshot["revision"],"label":format!("Rotated matrix {kind}"),
                "kind":"api_key","secret":stored_secret(kind, &rotated)
            }),
        )
        .await;
        services.clear();
        let task = effects::run(
            app,
            user,
            "project-a",
            marker,
            &format!("{}-{kind}-rotated", mode.name()),
        )
        .await;
        assert!(
            effects::successful(&task),
            "rotated {mode:?} {kind}: {:?}",
            task.snapshot
        );
        assert_observations(services, mode, "project-a", marker, Some(kind));

        app.idle().await;
        let snapshot = app.read(actor, &inventory).await;
        app.ok(
            actor,
            Method::DELETE,
            &format!("{base}/{id}"),
            json!({"revision":snapshot["revision"]}),
        )
        .await;
        for project in ["project-a", "project-b"] {
            services.clear();
            let task = effects::run(
                app,
                user,
                project,
                marker,
                &format!("{}-{kind}-{project}-removed", mode.name()),
            )
            .await;
            if project == "project-a" || mode == AuthenticationCase::Global {
                assert!(
                    !effects::successful(&task),
                    "removed {mode:?} {kind} remained usable"
                );
                assert!(
                    services
                        .observations()
                        .iter()
                        .all(|row| row.path != destination),
                    "removed {mode:?} {kind} reached its destination or fell back to a dormant key"
                );
                if kind == "provider" {
                    assert!(
                        model_failed(&task),
                        "a missing provider credential must fail the model run without routing fallback"
                    );
                }
            } else {
                assert!(
                    effects::successful(&task),
                    "removing {kind} in project-a affected project-b"
                );
                assert_observations(services, mode, project, marker, None);
            }
        }
        restore(app, actor, mode, kind).await;
        for project in ["project-a", "project-b"] {
            if project == "project-b" && mode != AuthenticationCase::Global {
                continue;
            }
            services.clear();
            let restored = effects::run(
                app,
                user,
                project,
                marker,
                &format!("{}-{kind}-{project}-restored", mode.name()),
            )
            .await;
            assert!(
                effects::successful(&restored),
                "explicit re-enrollment did not restore {mode:?} {kind} in {project}"
            );
            assert_observations(services, mode, project, marker, None);
        }
    }
}

fn stored_secret(kind: &str, token: &str) -> String {
    if kind == "search" {
        format!("Bearer {token}")
    } else {
        token.into()
    }
}

async fn restore(app: &App, actor: &HeaderMap, mode: AuthenticationCase, kind: &str) {
    app.idle().await;
    let base = store_base(mode, "project-a");
    let inventory = inventory_path(mode, "project-a");
    let snapshot = app.read(actor, &inventory).await;
    let target = snapshot["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "matrix" && row["kind"] == kind)
        .unwrap();
    let mut body = json!({"revision":snapshot["revision"],"label":format!("Re-enrolled matrix {kind}"),
        "kind":"api_key","secret":stored_secret(kind, &mode.token("project-a",kind))});
    if mode != AuthenticationCase::Global {
        body["attachment"] = json!({"kind":kind,"connection":"matrix","credential_binding":target["credential_binding"]});
    }
    let created = app.ok(actor, Method::POST, &base, body).await;
    if mode == AuthenticationCase::Global {
        for project in ["project-a", "project-b"] {
            let snapshot = app.read(actor, &inventory_path(mode, project)).await;
            let target = snapshot["connections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == "matrix" && row["kind"] == kind)
                .unwrap();
            app.ok(actor, Method::POST, &format!("{base}/attachments/{project}/{kind}/matrix"), json!({
                "revision":snapshot["revision"],"credential_id":created["id"],"credential_binding":target["credential_binding"]
            })).await;
        }
    }
}
