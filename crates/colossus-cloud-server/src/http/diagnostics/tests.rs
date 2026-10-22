use super::*;
use axum::{
    Json, Router,
    body::Body,
    http::StatusCode,
    middleware::{Next, from_fn},
    response::{IntoResponse, Response},
    routing::post,
};
use tower::ServiceExt as _;

async fn capture(request: Request, next: Next) -> Response {
    let failure = RequestFailure::capture(&request);
    let mut response = next.run(request).await;
    if let Some(line) = failure.line(&response, FailureCategory::Handler) {
        response.extensions_mut().insert(line);
    }
    response
}

#[tokio::test]
async fn http_failure_diagnostics_use_matched_templates_without_request_content() {
    let router = Router::new()
        .route(
            "/api/projects/{project}/threads/{thread}",
            post(|| async { StatusCode::SERVICE_UNAVAILABLE }),
        )
        .layer(from_fn(capture));
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/projects/private-project/threads/private-thread?token=private-query")
                .header("authorization", "Bearer private-header")
                .body(Body::from("private-body"))
                .unwrap(),
        )
        .await
        .unwrap();
    let line = response.extensions().get::<String>().unwrap();
    assert_eq!(
        line,
        "Cloud HTTP request failed: method=POST route=/api/projects/{project}/threads/{thread} status=503 category=handler"
    );
    assert!(!line.contains("private"));
}

#[test]
fn http_failure_diagnostics_suppress_success_and_json_client_errors() {
    let failure = RequestFailure::capture(&Request::new(Body::empty()));
    for code in 100..500 {
        let response = (
            StatusCode::from_u16(code).unwrap(),
            Json(serde_json::json!({"error":"permission_denied"})),
        )
            .into_response();
        assert!(failure.line(&response, FailureCategory::Handler).is_none());
    }
    let response = StatusCode::OK.into_response();
    assert!(failure.line(&response, FailureCategory::Handler).is_none());
}

#[test]
fn http_failure_diagnostics_report_non_json_client_rejections() {
    let failure = RequestFailure::capture(&Request::new(Body::empty()));
    let response = (StatusCode::UNPROCESSABLE_ENTITY, "private-extractor-error").into_response();
    assert_eq!(
        failure.line(&response, FailureCategory::Handler).unwrap(),
        "Cloud HTTP request failed: method=GET route=<unmatched> status=422 category=handler"
    );
}

#[test]
fn http_failure_diagnostics_bound_unknown_routes_methods_and_timeout_category() {
    let request = Request::builder()
        .method("PRIVATE-METHOD")
        .uri("/private-path?token=private-query")
        .body(Body::empty())
        .unwrap();
    let failure = RequestFailure::capture(&request);
    let line = failure
        .line(
            &StatusCode::SERVICE_UNAVAILABLE.into_response(),
            FailureCategory::AdmissionTimeout,
        )
        .unwrap();
    assert_eq!(
        line,
        "Cloud HTTP request failed: method=OTHER route=<unmatched> status=503 category=admission_timeout"
    );
    assert!(!line.contains("private"));
}
