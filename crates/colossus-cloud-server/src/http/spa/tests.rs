use super::*;
use axum::{body::to_bytes, http::HeaderMap};
use tower::ServiceExt as _;

const SHELL: &str = "<!doctype html><html><body>Control Plane shell fixture</body></html>";
const ASSET: &[u8] = b"const fixture = true;";

fn fixture() -> (tempfile::TempDir, Router) {
    let directory = tempfile::tempdir().expect("SPA fixture directory");
    std::fs::write(directory.path().join("index.html"), SHELL).expect("SPA fixture shell");
    std::fs::create_dir(directory.path().join("assets")).expect("SPA fixture assets");
    std::fs::write(directory.path().join("assets/app.js"), ASSET).expect("SPA fixture asset");
    for namespace in ["api", "auth", "health"] {
        std::fs::create_dir(directory.path().join(namespace)).expect("reserved fixture directory");
        std::fs::write(
            directory.path().join(namespace).join("missing"),
            b"reserved file",
        )
        .expect("reserved fixture file");
    }
    let router = service(directory.path());
    (directory, router)
}

async fn response(router: &Router, method: Method, path: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("fixture request"),
        )
        .await
        .expect("fixture response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 4096)
        .await
        .expect("bounded fixture body")
        .to_vec();
    (status, headers, body)
}

#[tokio::test]
async fn spa_get_recognized_pages_returns_exact_shell() {
    let (_directory, router) = fixture();
    for path in [
        "/",
        "/fleet",
        "/projects",
        "/admin",
        "/admin/users",
        "/admin/projects",
        "/admin/settings",
        "/settings",
        "/projects/project-a/overview",
        "/projects/project-a/tasks",
        "/projects/project-a/analytics",
        "/projects/project-a/access",
        "/projects/project-a/settings",
        "/projects/project-a/agents/node-a/overview",
        "/projects/project-a/agents/node-a/threads",
        "/projects/project-a/agents/node-a/analytics",
        "/projects/project-a/agents/node-a/policy",
        "/projects/project-a/agents/node-a/workflows",
        "/projects/project-a/agents/node-a/schedules",
        "/projects/project-a/agents/node-a/capabilities",
        "/projects/project-a/agents/node-a/plugins",
        "/projects/project-a/agents/node-a/library",
        "/projects/project-a/agents/node-a/connections",
        "/projects/project-a/hosts/host-a",
        "/projects/project-a/threads/thread-a",
        "/projects/project-a/tasks/task-a",
        "/projects/project%3Aa/agents/node%3Ab/threads?compose=1",
        "/fleet?project=project%3Aa",
    ] {
        let (status, headers, body) = response(&router, Method::GET, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, SHELL.as_bytes(), "{path}");
        assert!(
            headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
    }
    let (status, _, body) = response(&router, Method::GET, "/projects/project-a/overview/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, SHELL.as_bytes());
}

#[tokio::test]
async fn spa_head_preserves_shell_headers_without_body() {
    let (_directory, router) = fixture();
    for path in [
        "/",
        "/projects/project-a/threads/thread-a",
        "/projects/project-a/hosts/host-a",
        "/projects/project-a/agents/node-a/workflows",
        "/projects/project-a/agents/node-a/schedules",
        "/projects/project-a/agents/node-a/capabilities",
        "/projects/project-a/agents/node-a/plugins",
        "/projects/project-a/agents/node-a/library",
        "/projects/project-a/agents/node-a/connections",
    ] {
        let (get_status, get_headers, get_body) = response(&router, Method::GET, path).await;
        let (head_status, head_headers, head_body) = response(&router, Method::HEAD, path).await;
        assert_eq!(get_status, StatusCode::OK);
        assert_eq!(head_status, StatusCode::OK);
        assert!(head_body.is_empty());
        assert_eq!(head_headers["content-type"], get_headers["content-type"]);
        assert_eq!(
            head_headers["content-length"],
            get_headers["content-length"]
        );
        assert_eq!(get_body, SHELL.as_bytes());
    }
}

#[tokio::test]
async fn spa_assets_reserved_namespaces_and_unknown_paths_stay_distinct() {
    let (_directory, router) = fixture();
    let (status, _, body) = response(&router, Method::GET, "/assets/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, ASSET);
    for path in [
        "/missing-page",
        "/assets/missing.js",
        "/api/missing",
        "/auth/missing",
        "/health/missing",
        "/%61pi/missing",
        "/projects/project-a/missing-view",
        "/projects/project-a/agents/node-a/missing-view",
        "/admin/missing-view",
    ] {
        for method in [Method::GET, Method::HEAD] {
            let (status, _, body) = response(&router, method, path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert_ne!(body, SHELL.as_bytes(), "{path}");
            assert_ne!(body, b"reserved file", "{path}");
        }
    }
}

#[tokio::test]
async fn spa_malformed_identifiers_and_traversal_cannot_select_shell_or_asset() {
    let (_directory, router) = fixture();
    let oversized = format!("/projects/{}/overview", "a".repeat(257));
    let oversized_path = format!("/assets/{}", "a".repeat(2048));
    for path in [
        "/projects/../overview",
        "/projects/%2e%2e/overview",
        "/projects/project%2fa/overview",
        "/projects/project%5ca/overview",
        "/projects/bad%20id/overview",
        "/projects/bad%00id/overview",
        "/projects/%ff/overview",
        "/projects/%zz/overview",
        "/projects/project-a/overview//",
        "/assets/../index.html",
        "/assets/%2e%2e/index.html",
        "/assets/%2findex.html",
        "/assets/%5cindex.html",
        oversized.as_str(),
        oversized_path.as_str(),
    ] {
        let (status, _, body) = response(&router, Method::GET, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_ne!(body, SHELL.as_bytes(), "{path}");
    }
    let at_limit = format!("/projects/{}/overview", "a".repeat(256));
    assert_eq!(
        response(&router, Method::GET, &at_limit).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn spa_non_get_preserves_static_method_rejection() {
    let (_directory, router) = fixture();
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS] {
        for path in [
            "/fleet",
            "/projects/project-a/threads/thread-a",
            "/assets/app.js",
        ] {
            let (status, headers, body) = response(&router, method.clone(), path).await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            assert_eq!(headers["allow"], "GET,HEAD");
            assert_ne!(body, SHELL.as_bytes());
        }
    }
}
