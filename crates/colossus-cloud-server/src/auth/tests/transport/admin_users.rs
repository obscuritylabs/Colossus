//! User administration crosses the real HTTP authentication, CAS and session boundary.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

const USER: &str = "/api/admin/users/local-user";
const PASSWORD_PATH: &str = "/api/admin/users/local-user/password";
const ROTATED: &str = "rotated HTTP fixture password 2026";

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

struct Fixture {
    router: Router,
    admin: HeaderMap,
    user: HeaderMap,
    issuer: tokio::task::JoinHandle<()>,
    _server: crate::server::CloudServer,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn start() -> Self {
        use super::super::local::{PASSWORD, fixture_local, local_cookie};
        let (auth, issuer) = fixture_local().await;
        let admin = local_cookie(&auth, "administrator", PASSWORD).await;
        let user = local_cookie(&auth, "operator", PASSWORD).await;
        let root = tempfile::tempdir().unwrap();
        let mut config = auth.config.clone();
        tls_files(root.path(), &mut config);
        let server = crate::server::CloudServer::with_store(config, auth.store.clone())
            .await
            .unwrap();
        Self {
            router: crate::http::router(server.state.clone()),
            admin,
            user,
            issuer,
            _server: server,
            _root: root,
        }
    }

    async fn request(
        &self,
        headers: &HeaderMap,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        *request.headers_mut().unwrap() = headers.clone();
        let body = if let Some(body) = body {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&body).unwrap())
        } else {
            Body::empty()
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
        Reply {
            status,
            headers,
            body: if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            },
        }
    }

    async fn account(&self) -> Value {
        let reply = self.request(&self.admin, Method::GET, USER, None).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.body.get("password").is_none());
        assert!(!reply.body.to_string().contains("password_hash"));
        reply.body["user"].clone()
    }

    async fn denied(&self, method: Method, path: &str, body: &Value) {
        let before = self.account().await;
        let mut no_csrf = self.admin.clone();
        no_csrf.remove("x-colossus-csrf");
        for headers in [HeaderMap::new(), self.user.clone(), no_csrf] {
            assert_eq!(
                self.request(&headers, method.clone(), path, Some(body.clone()))
                    .await
                    .status,
                StatusCode::FORBIDDEN
            );
            assert_eq!(self.account().await, before);
        }
    }

    async fn login(&self, password: &str) -> Reply {
        let mut headers = self.admin.clone();
        headers.remove(header::COOKIE);
        self.request(
            &headers,
            Method::POST,
            "/auth/local",
            Some(json!({"username":"operator","password":password})),
        )
        .await
    }

    fn session(&self, reply: &Reply) -> HeaderMap {
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        let cookie = reply.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let mut headers = self.admin.clone();
        headers.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        headers
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.issuer.abort();
    }
}

#[tokio::test]
async fn user_edits_http_require_admin_csrf_current_revision_and_retire_disabled_sessions() {
    use super::super::local::PASSWORD;
    let fixture = Fixture::start().await;
    let initial = fixture.account().await;
    let edit = json!({"revision":initial["revision"],"display_name":"Reviewed operator","email":"operator@example.test"});
    fixture.denied(Method::PATCH, USER, &edit).await;
    let saved = fixture
        .request(&fixture.admin, Method::PATCH, USER, Some(edit.clone()))
        .await;
    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(saved.body["user"]["display_name"], "Reviewed operator");
    assert_eq!(saved.body["user"]["email"], "operator@example.test");
    assert_eq!(saved.body["user"]["is_admin"], false);
    assert_eq!(fixture.account().await, saved.body["user"]);
    assert_eq!(
        fixture
            .request(&fixture.admin, Method::PATCH, USER, Some(edit))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(fixture.account().await, saved.body["user"]);

    let disable = json!({"revision":saved.body["user"]["revision"],"active":false,"email":null});
    fixture.denied(Method::PATCH, USER, &disable).await;
    let disabled = fixture
        .request(&fixture.admin, Method::PATCH, USER, Some(disable.clone()))
        .await;
    assert_eq!(disabled.status, StatusCode::OK);
    assert_eq!(disabled.body["user"]["active"], false);
    assert_eq!(disabled.body["user"]["email"], Value::Null);
    assert_eq!(fixture.account().await, disabled.body["user"]);
    assert_eq!(
        fixture
            .request(&fixture.user, Method::GET, "/api/me", None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(fixture.login(PASSWORD).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        fixture
            .request(&fixture.admin, Method::PATCH, USER, Some(disable))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(fixture.account().await, disabled.body["user"]);
    let enabled = fixture
        .request(
            &fixture.admin,
            Method::PATCH,
            USER,
            Some(json!({"revision":disabled.body["user"]["revision"],"active":true})),
        )
        .await;
    assert_eq!(enabled.status, StatusCode::OK);
    assert_eq!(enabled.body["user"]["active"], true);
    assert_eq!(fixture.account().await, enabled.body["user"]);
    assert_eq!(
        fixture
            .request(&fixture.user, Method::GET, "/api/me", None)
            .await
            .status,
        StatusCode::FORBIDDEN,
        "reenabling the account must not revive a retired session"
    );
    let fresh = fixture.session(&fixture.login(PASSWORD).await);
    assert_eq!(
        fixture
            .request(&fresh, Method::GET, "/api/me", None)
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_reset_http_requires_admin_csrf_current_revision_and_retires_old_credentials() {
    use super::super::local::PASSWORD;
    let fixture = Fixture::start().await;
    let initial = fixture.account().await;
    let body = json!({"revision":initial["revision"],"new_password":ROTATED});
    fixture.denied(Method::POST, PASSWORD_PATH, &body).await;
    let mut stale = body.clone();
    stale["revision"] = json!(initial["revision"].as_u64().unwrap() + 1);
    assert_eq!(
        fixture
            .request(&fixture.admin, Method::POST, PASSWORD_PATH, Some(stale))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(fixture.account().await, initial);
    assert_eq!(
        fixture
            .request(&fixture.user, Method::GET, "/api/me", None)
            .await
            .status,
        StatusCode::OK
    );
    let before_reset = fixture.session(&fixture.login(PASSWORD).await);
    let saved = fixture
        .request(
            &fixture.admin,
            Method::POST,
            PASSWORD_PATH,
            Some(body.clone()),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(
        saved.body["user"]["revision"].as_u64().unwrap(),
        initial["revision"].as_u64().unwrap() + 1
    );
    assert_eq!(fixture.account().await, saved.body["user"]);
    for session in [&fixture.user, &before_reset] {
        assert_eq!(
            fixture
                .request(session, Method::GET, "/api/me", None)
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(fixture.login(PASSWORD).await.status, StatusCode::FORBIDDEN);
    let fresh = fixture.session(&fixture.login(ROTATED).await);
    let identity = fixture.request(&fresh, Method::GET, "/api/me", None).await;
    assert_eq!(identity.status, StatusCode::OK);
    assert_eq!(identity.body["user"]["id"], "local-user");
    assert_eq!(
        fixture
            .request(&fixture.admin, Method::POST, PASSWORD_PATH, Some(body))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(fixture.account().await, saved.body["user"]);
    assert_eq!(
        fixture
            .request(&fresh, Method::GET, "/api/me", None)
            .await
            .status,
        StatusCode::OK,
        "a stale reset must not rotate the password or retire current sessions"
    );
}
