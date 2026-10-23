//! Every fixture owns its keys, runtime homes, accounts and loopback requests.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use colossus_cloud::{ProjectRole, storage::CloudStore};
use tower::ServiceExt;

pub(super) struct App {
    pub state: Arc<crate::server::State>,
    pub router: Router,
    pub admin: HeaderMap,
    config: crate::config::Config,
    store: Arc<dyn CloudStore>,
    _directory: tempfile::TempDir,
    issuer: tokio::task::JoinHandle<()>,
}

impl App {
    pub async fn start() -> Self {
        Self::with_invalid_binding(false).await
    }

    pub async fn with_invalid_binding(invalid_binding: bool) -> Self {
        use super::super::super::local::{PASSWORD, fixture_local, local_cookie};
        let (auth, issuer) = fixture_local().await;
        let admin = local_cookie(&auth, "administrator", PASSWORD).await;
        let directory = isolated::directory();
        let root = directory.path().canonicalize().unwrap();
        let mut config = auth.config.clone();
        tls_files(&root, &mut config);
        let mut seed = RuntimeConfig::controlled_chat_template(root.join("seed.redb"));
        seed.storage.keys = colossus_runtime::KeyConfig::Environment {
            journal_variable: JOURNAL.into(),
            journal_key_id: "matrix-seed".into(),
            signing_variable: SIGNING.into(),
            anchor_path: root.join("seed.anchor"),
        };
        if invalid_binding {
            // A legacy operator seed can be syntactically valid while declaring
            // a credential reference under explicit None authentication.
            seed.providers.profiles.insert(
                "invalid".into(),
                serde_json::from_value(json!({
                    "kind":"open_ai_compatible","baseUrl":"https://invalid.example/v1",
                    "credentialReference":"host:managed"
                }))
                .unwrap(),
            );
            seed.sandbox
                .network_destinations
                .push("https://invalid.example".into());
            let mut model = seed.models.profiles["echo"].clone();
            model.provider_profile = "invalid".into();
            seed.models.profiles.insert("invalid".into(), model);
            seed.models.roles.insert("primary".into(), "invalid".into());
        }
        config.chat = Some(crate::config::ChatConfig {
            home: root.join("hosted"),
            wrapping_key_variable: WRAPPING.into(),
            shared: None,
            max_active_runtimes: 8,
            idle_seconds: 300,
            projects: ["project-a", "project-b"]
                .into_iter()
                .map(|project| {
                    (project.into(), {
                        let mut project = crate::config::ChatProjectConfig::new(seed.clone());
                        if invalid_binding {
                            project.provider_authentication.insert(
                                "invalid".into(),
                                colossus_cloud_chat::ChatAuthentication::None,
                            );
                        }
                        project
                    })
                })
                .collect(),
        });
        let store = auth.store.clone();
        let server = crate::server::CloudServer::with_store(config.clone(), store.clone())
            .await
            .unwrap();
        let app = Self {
            router: crate::http::router(server.state.clone()),
            state: server.state,
            admin,
            config,
            store,
            _directory: directory,
            issuer,
        };
        app.member("project-a", "local-admin", ProjectRole::ProjectAdmin)
            .await;
        app
    }

    pub async fn request(
        &self,
        headers: &HeaderMap,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        Box::pin(self.request_inner(headers, method, path, body)).await
    }
    async fn request_inner(
        &self,
        headers: &HeaderMap,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        *request.headers_mut().unwrap() = headers.clone();
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }
    pub async fn read(&self, headers: &HeaderMap, path: &str) -> Value {
        let (status, value) = self.request(headers, Method::GET, path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {value}");
        value
    }
    pub async fn ok(&self, headers: &HeaderMap, method: Method, path: &str, body: Value) -> Value {
        let (status, value) = self
            .request(headers, method.clone(), path, Some(body))
            .await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }
    pub async fn member(&self, project: &str, user: &str, role: ProjectRole) {
        self.ok(
            &self.admin,
            Method::POST,
            &format!("/api/projects/{project}/members"),
            json!({"user_id":user,"role":role,"revision":0}),
        )
        .await;
    }
    pub async fn fresh_user(&self, name: &str) -> (String, HeaderMap) {
        use super::super::super::local::PASSWORD;
        let value = self.ok(&self.admin, Method::POST, "/api/admin/users", json!({"username":name,"password":PASSWORD,"display_name":name,"email":null,"oidc_subject":null,"is_admin":false})).await;
        let id = value["user"]["id"].as_str().unwrap().to_owned();
        let mut headers = self.admin.clone();
        headers.remove("cookie");
        let mut request = Request::builder().method(Method::POST).uri("/auth/local");
        *request.headers_mut().unwrap() = headers.clone();
        let response = self
            .router
            .clone()
            .oneshot(
                request
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"username":name,"password":PASSWORD}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        headers.insert(
            "cookie",
            response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .parse()
                .unwrap(),
        );
        for project in ["project-a", "project-b"] {
            self.member(project, &id, ProjectRole::Operator).await;
        }
        (id, headers)
    }
    pub async fn settings(&self, project: &str) -> Value {
        self.read(
            &self.admin,
            &format!("/api/projects/{project}/chat/settings"),
        )
        .await
    }
    pub async fn save_settings(&self, project: &str, mut body: Value) -> Value {
        self.idle().await;
        body.as_object_mut().unwrap().remove("credential_status");
        self.ok(
            &self.admin,
            Method::PUT,
            &format!("/api/projects/{project}/chat/settings"),
            body,
        )
        .await
    }
    pub async fn idle(&self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let host = self.state.hosted.as_ref().unwrap();
                if !host.active_project_run_status("project-a").await.unwrap()
                    && !host.active_project_run_status("project-b").await.unwrap()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("matrix work must settle before a configuration mutation");
    }
    pub async fn restart(&mut self) {
        self.idle().await;
        self.state
            .hosted
            .as_ref()
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        let server =
            crate::server::CloudServer::with_store(self.config.clone(), self.store.clone())
                .await
                .unwrap();
        self.router = crate::http::router(server.state.clone());
        self.state = server.state;
    }
    pub async fn stop(self) {
        self.state
            .hosted
            .as_ref()
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        self.issuer.abort();
        assert!(self.issuer.await.unwrap_err().is_cancelled());
    }
}
