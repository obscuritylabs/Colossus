mod transport;
use crate::{
    operations::Operation,
    transport::{session_cookie, validate_origin},
};

#[test]
fn server_selection_rejects_credentials_paths_and_insecure_release_origins() {
    for input in [
        "http://example.com",
        "https://user:password@example.com",
        "https://example.com/api",
        "https://example.com?token=x",
        "https://example.com/#fragment",
        "file:///tmp/server",
        "javascript:alert(1)",
    ] {
        assert!(validate_origin(input, false).is_err(), "{input}");
    }
    assert!(validate_origin("https://example.com:8443/", false).is_ok());
    assert!(validate_origin("http://localhost:8090", true).is_ok());
    assert!(validate_origin("http://10.0.2.2:8090", true).is_err());
}

#[test]
fn cookie_custody_accepts_only_the_exact_bounded_session_cookie() {
    assert_eq!(
        session_cookie("colossus_session=abc-123_DEF; HttpOnly; Secure"),
        Some("colossus_session=abc-123_DEF".into())
    );
    assert!(session_cookie("other=token").is_none());
    assert!(session_cookie("colossus_session=").is_none());
    assert!(session_cookie("colossus_session=a\r\nHeader: x").is_none());
    assert!(session_cookie(&format!("colossus_session={}", "a".repeat(257))).is_none());
}

#[test]
fn route_identifiers_cannot_change_origin_or_inject_query_parameters() {
    let origin = validate_origin("https://colossus.example", false).unwrap();
    let operation = Operation::Thread {
        project: "../../outside?x=1".into(),
        thread: "https://other.example/#private".into(),
    };
    assert!(operation.route(&origin).is_err());
    let (_, url, _) = Operation::Thread {
        project: "namespace:engineering".into(),
        thread: "thread-1".into(),
    }
    .route(&origin)
    .unwrap();
    assert_eq!(url.origin(), origin.origin());
    assert_eq!(
        url.path(),
        "/api/projects/namespace:engineering/threads/thread-1"
    );
    assert!(url.query().is_none());
    assert!(url.fragment().is_none());
}

#[test]
fn exact_dot_segments_are_rejected_before_route_construction() {
    let origin = validate_origin("https://colossus.example", false).unwrap();
    for project in [".", "..", ""] {
        assert!(
            Operation::Fleet {
                project: project.into(),
                after: None
            }
            .route(&origin)
            .is_err()
        );
    }
}
#[test]
fn continuation_uses_the_servers_expected_revision_field() {
    let origin = validate_origin("https://colossus.example", false).unwrap();
    let (_, _, body) = Operation::SendMessage {
        project: "engineering".into(),
        thread: "thread-1".into(),
        revision: 17,
        request: serde_json::json!({}),
    }
    .route(&origin)
    .unwrap();
    let body = body.unwrap();
    assert_eq!(body["expected_revision"], 17);
    assert!(body.get("revision").is_none());
}

#[test]
fn task_activity_reads_use_the_existing_bounded_retained_event_contract() {
    let origin = validate_origin("https://colossus.example", false).unwrap();
    let (method, url, body) = Operation::TaskUpdates {
        project: "engineering".into(),
        task: "task-1".into(),
        after: 42,
    }
    .route(&origin)
    .unwrap();
    assert_eq!(method, reqwest::Method::GET);
    assert_eq!(url.path(), "/api/projects/engineering/tasks/task-1/updates");
    assert_eq!(url.query(), Some("after=42"));
    assert!(body.is_none());
    assert!(
        serde_json::from_value::<Operation>(serde_json::json!({
            "kind":"task_updates","project":"engineering","task":"task-1","after":-1
        }))
        .is_err()
    );
}
