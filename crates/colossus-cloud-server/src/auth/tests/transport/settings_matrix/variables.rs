//! Retained nonsecret values never cross account or project boundaries.
use super::*;

const PATH: &str = "/api/projects/project-a/chat/variables/personal";

pub(super) async fn write(app: &App, user: &HeaderMap) {
    let initial = app.read(user, PATH).await;
    assert_eq!(initial["variables"], json!({}));
    let body = json!({"revision":initial["revision"],"variables":{"MATRIX":"personal"}});
    let mut no_csrf = user.clone();
    no_csrf.remove("x-colossus-csrf");
    assert_eq!(
        app.request(&no_csrf, Method::PUT, PATH, Some(body.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let saved = app.ok(user, Method::PUT, PATH, body.clone()).await;
    assert_eq!(saved["variables"]["MATRIX"], "personal");
    assert_eq!(
        app.request(user, Method::PUT, PATH, Some(body)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.settings("project-a").await["variables"]["MATRIX"],
        "global",
        "a private retained value is separate from the effective project catalog"
    );
    assert_eq!(
        app.read(user, "/api/projects/project-b/chat/variables/personal")
            .await["variables"],
        json!({})
    );
}

pub(super) async fn retained_and_isolated(app: &App, owner: &HeaderMap, other: &HeaderMap) {
    let retained = app.read(owner, PATH).await;
    assert_eq!(retained["variables"]["MATRIX"], "personal");
    assert_eq!(app.read(other, PATH).await["variables"], json!({}));
    assert_eq!(
        app.read(owner, "/api/projects/project-b/chat/variables/personal")
            .await["variables"],
        json!({})
    );
    app.ok(
        owner,
        Method::PUT,
        PATH,
        json!({"revision":retained["revision"],"variables":{}}),
    )
    .await;
    assert_eq!(app.read(owner, PATH).await["variables"], json!({}));
}
