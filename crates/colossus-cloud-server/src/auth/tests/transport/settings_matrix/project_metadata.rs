//! Authenticated General edits retain project scope, hierarchy and reviewed revisions.
use super::*;
use colossus_cloud::CloudPermission;

const PATH: &str = "/api/projects/metadata-child";

pub(super) async fn verify(app: &mut App) {
    app.ok(
        &app.admin,
        Method::POST,
        "/api/projects",
        json!({"id":"metadata-child","name":"Metadata child","parent_project_id":"project-b"}),
    )
    .await;
    let (user, headers) = app.fresh_user("metadata-project-admin").await;
    app.member("metadata-child", &user, ProjectRole::ProjectAdmin)
        .await;
    assert_eq!(
        app.read(&headers, "/api/me").await["user"]["is_admin"],
        false
    );
    let initial = app.read(&headers, PATH).await["project"].clone();
    assert_eq!(initial["parent_project_id"], "project-b");

    let name = "N".repeat(256);
    let description = "D".repeat(4096);
    let body = json!({
        "revision":initial["revision"],"name":name,"description":description,
        "parent_project_id":"project-b","archived":false
    });
    let mut no_csrf = headers.clone();
    no_csrf.remove("x-colossus-csrf");
    assert_eq!(
        app.request(&no_csrf, Method::PATCH, PATH, Some(body.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let saved = app.ok(&headers, Method::PATCH, PATH, body.clone()).await["project"].clone();
    assert_eq!(saved["name"], name);
    assert_eq!(saved["description"], description);
    assert_eq!(saved["parent_project_id"], initial["parent_project_id"]);
    assert_eq!(saved["created_at"], initial["created_at"]);
    assert_eq!(saved["revision"], initial["revision"].as_u64().unwrap() + 1);
    assert_eq!(app.read(&headers, PATH).await["project"], saved);
    assert_eq!(
        app.request(&headers, Method::PATCH, PATH, Some(body))
            .await
            .0,
        StatusCode::CONFLICT,
        "a stale General form cannot overwrite a reviewed save"
    );

    for edit in [
        json!({"revision":saved["revision"],"parent_project_id":null}),
        json!({"revision":saved["revision"],"parent_project_id":"project-a"}),
    ] {
        assert_eq!(
            app.request(&headers, Method::PATCH, PATH, Some(edit))
                .await
                .0,
            StatusCode::FORBIDDEN,
            "Project Administration cannot move its hierarchy"
        );
        assert_eq!(app.read(&headers, PATH).await["project"], saved);
    }
    for edit in [
        json!({"revision":saved["revision"],"name":"N".repeat(257)}),
        json!({"revision":saved["revision"],"description":"D".repeat(4097)}),
    ] {
        assert_eq!(
            app.request(&headers, Method::PATCH, PATH, Some(edit))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(app.read(&headers, PATH).await["project"], saved);
    }
    let other = app.read(&headers, "/api/projects/project-a").await["project"].clone();
    assert_eq!(
        app.request(
            &headers,
            Method::PATCH,
            "/api/projects/project-a",
            Some(json!({"revision":other["revision"],"name":"Wrong project"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.read(&headers, "/api/projects/project-a").await["project"],
        other
    );
    assert_eq!(
        app.request(
            &headers,
            Method::POST,
            "/api/projects",
            Some(json!({"id":"unapproved-project","name":"No creation authority","parent_project_id":null}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let archived = app
        .ok(
            &headers,
            Method::PATCH,
            PATH,
            json!({"revision":saved["revision"],"archived":true}),
        )
        .await["project"]
        .clone();
    assert_eq!(archived["archived"], true);
    assert_eq!(archived["parent_project_id"], "project-b");
    let caller = app
        .state
        .auth
        .caller(&headers, "metadata-child", false)
        .await
        .unwrap();
    assert!(caller.require(CloudPermission::Read).is_ok());
    assert!(caller.require(CloudPermission::Administer).is_ok());
    assert!(caller.require(CloudPermission::Execute).is_err());
    assert!(caller.require(CloudPermission::Control).is_err());
    app.restart().await;
    assert_eq!(app.read(&headers, PATH).await["project"], archived);
    let reopened = app
        .ok(
            &headers,
            Method::PATCH,
            PATH,
            json!({"revision":archived["revision"],"archived":false}),
        )
        .await["project"]
        .clone();
    assert_eq!(reopened["archived"], false);
    assert_eq!(reopened["name"], name);
    assert_eq!(reopened["description"], description);
    assert_eq!(reopened["parent_project_id"], "project-b");
    assert_eq!(app.read(&headers, PATH).await["project"], reopened);
}
