//! OCI wire acceptance against an isolated Distribution-v2 fixture, without Docker.

use super::*;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[derive(Default)]
struct RegistryState {
    blobs: BTreeMap<String, Vec<u8>>,
    manifest: Vec<u8>,
    uploads: BTreeMap<String, Vec<u8>>,
    requests: Vec<(String, String, Option<String>)>,
    deny: bool,
    challenge: Option<String>,
    interrupt_next_patch: bool,
    interrupt_empty_patch: bool,
    blob_redirect: Option<String>,
}

struct RegistryFixture {
    origin: String,
    state: Arc<Mutex<RegistryState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for RegistryFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl RegistryFixture {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("registry");
        let origin = format!("http://{}", listener.local_addr().expect("address"));
        let state = Arc::new(Mutex::new(RegistryState::default()));
        let shared = Arc::clone(&state);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let (head, body) = loop {
                    let mut chunk = [0; 8192];
                    let count = stream.read(&mut chunk).await.expect("request");
                    if count == 0 {
                        break (String::new(), Vec::new());
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    assert!(bytes.len() < 8 * 1024 * 1024, "fixture request bound");
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let head = String::from_utf8(bytes[..end].to_vec()).expect("HTTP header");
                        let length = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().expect("length"))
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break (head, bytes[end + 4..end + 4 + length].to_vec());
                        }
                    }
                };
                if head.is_empty() {
                    continue;
                }
                let (status, headers, body) =
                    respond(&mut shared.lock().expect("fixture state"), &head, body);
                if status == 0 {
                    continue;
                } // Commit a partial chunk, then lose the response.
                let response = format!(
                    "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });
        Self {
            origin,
            state,
            task,
        }
    }

    fn profile(&self) -> PluginRegistryProfile {
        PluginRegistryProfile {
            origin: self.origin.clone(),
            trust_profile: "offline".into(),
            allow_non_public: true,
            ..PluginRegistryProfile::default()
        }
    }
    fn reference(&self, selector: &str) -> String {
        format!(
            "{}/team/demo{selector}",
            self.origin.trim_start_matches("http://")
        )
    }
    fn client(&self) -> PluginRegistryClient {
        PluginRegistryClient::new(self.profile(), RegistryCredential::Anonymous)
            .expect("client")
            .with_timeout(Duration::from_secs(3))
    }
}

fn respond(state: &mut RegistryState, head: &str, body: Vec<u8>) -> (u16, String, Vec<u8>) {
    let mut request = head
        .lines()
        .next()
        .expect("request line")
        .split_whitespace();
    let method = request.next().expect("method");
    let path = request.next().expect("path");
    let authorization = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("authorization")
            .then(|| value.trim().to_owned())
    });
    state
        .requests
        .push((method.into(), path.into(), authorization.clone()));
    if state.deny {
        return (403, String::new(), Vec::new());
    }
    if path.starts_with("/token") {
        return (
            200,
            "Content-Type: application/json\r\n".into(),
            br#"{"token":"fixture-access"}"#.to_vec(),
        );
    }
    if let Some(realm) = &state.challenge
        && authorization.as_deref() != Some("Bearer fixture-access")
    {
        return (
            401,
            format!("WWW-Authenticate: Bearer realm=\"{realm}\",service=\"fixture\"\r\n"),
            Vec::new(),
        );
    }
    if path.contains("/referrers/") {
        return (
            200,
            "Content-Type: application/json\r\n".into(),
            br#"{"schemaVersion":2,"manifests":[]}"#.to_vec(),
        );
    }
    if path.contains("/manifests/") {
        if method == "PUT" {
            state.manifest = body;
            return (201, String::new(), Vec::new());
        }
        return (
            200,
            format!(
                "Content-Type: {OCI_IMAGE_MANIFEST_MEDIA_TYPE}\r\nDocker-Content-Digest: {}\r\n",
                sha256_digest(&state.manifest)
            ),
            state.manifest.clone(),
        );
    }
    if method == "POST" {
        let upload = format!("/upload/{}", state.uploads.len());
        state.uploads.insert(upload.clone(), Vec::new());
        return (202, format!("Location: {upload}\r\n"), Vec::new());
    }
    if path.starts_with("/upload/") {
        if method == "DELETE" {
            return (204, String::new(), Vec::new());
        }
        let base = path.split('?').next().expect("base");
        let upload = state.uploads.get_mut(base).expect("upload");
        if method == "GET" {
            return (
                204,
                format!(
                    "Location: {base}\r\nRange: 0-{}\r\n",
                    upload.len().saturating_sub(1)
                ),
                Vec::new(),
            );
        }
        if method == "PATCH" {
            if std::mem::take(&mut state.interrupt_empty_patch) {
                return (0, String::new(), Vec::new());
            }
            if std::mem::take(&mut state.interrupt_next_patch) {
                upload.extend_from_slice(&body[..body.len() / 2]);
                return (0, String::new(), Vec::new());
            }
            upload.extend(body);
            return (
                202,
                format!("Location: {base}\r\nRange: 0-{}\r\n", upload.len() - 1),
                Vec::new(),
            );
        }
        let digest = Url::parse(&format!("http://fixture{path}"))
            .expect("query")
            .query_pairs()
            .find(|(name, _)| name == "digest")
            .expect("digest")
            .1
            .into_owned();
        assert_eq!(sha256_digest(upload), digest);
        state.blobs.insert(digest, upload.clone());
        return (201, String::new(), Vec::new());
    }
    let digest = path.rsplit('/').next().expect("digest");
    if method == "GET"
        && let Some(origin) = &state.blob_redirect
    {
        return (
            307,
            format!("Location: {origin}/objects/{digest}\r\n"),
            Vec::new(),
        );
    }
    match state.blobs.get(digest) {
        Some(bytes) => (
            200,
            "Content-Type: application/octet-stream\r\n".into(),
            if method == "HEAD" {
                Vec::new()
            } else {
                bytes.clone()
            },
        ),
        None => (404, String::new(), Vec::new()),
    }
}

fn layout(path: &Path) -> BuiltPluginArtifact {
    let source = path.join("source");
    fs::create_dir(&source).expect("source");
    fs::write(source.join("plugin.json"), br#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"example","version":"1.0.0"}"#).expect("manifest");
    package_plugin_to_layout(&source, &path.join("layout"), None).expect("package")
}

#[tokio::test]
async fn distribution_push_pull_round_trip_pins_tags_and_rejects_digest_substitution() {
    let directory = tempfile::tempdir().expect("temporary");
    let artifact = layout(directory.path());
    let registry = RegistryFixture::start().await;
    let client = registry.client();
    let pushed = client
        .push(&directory.path().join("layout"), &registry.reference(":v1"))
        .await
        .expect("push");
    assert_eq!(pushed.manifest_digest, artifact.manifest_digest);
    let pulled = client
        .pull(&registry.reference(":v1"), &directory.path().join("pulled"))
        .await
        .expect("pull");
    assert_eq!(pulled.manifest_digest, artifact.manifest_digest);
    assert_eq!(
        verify_plugin_layout(&directory.path().join("pulled"), None)
            .expect("verified")
            .layer,
        artifact.layer
    );
    let wrong = registry.reference(&format!("@sha256:{}", "f".repeat(64)));
    assert!(
        client
            .pull(&wrong, &directory.path().join("wrong"))
            .await
            .expect_err("digest substitution")
            .to_string()
            .contains("requested immutable digest")
    );
    let before = registry.state.lock().expect("state").requests.len();
    assert!(
        client
            .push(&directory.path().join("layout"), &wrong)
            .await
            .is_err()
    );
    assert_eq!(
        before,
        registry.state.lock().expect("state").requests.len(),
        "invalid push must open no connection"
    );
    let state = registry.state.lock().expect("state");
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|(method, path, _)| method == "GET" && path.ends_with("/manifests/v1"))
            .count(),
        1,
        "resolve mutable tag once"
    );
}

#[tokio::test]
async fn bearer_challenges_use_only_explicit_token_origins_and_rbac_never_releases_secrets() {
    let directory = tempfile::tempdir().expect("temporary");
    layout(directory.path());
    let registry = RegistryFixture::start().await;
    let token = RegistryFixture::start().await;
    registry.state.lock().expect("state").challenge = Some(format!("{}/token", token.origin));
    assert!(
        registry
            .client()
            .push(&directory.path().join("layout"), &registry.reference(":v1"))
            .await
            .expect_err("unconfigured token origin")
            .to_string()
            .contains("not permitted")
    );
    assert!(token.state.lock().expect("token state").requests.is_empty());
    let mut profile = registry.profile();
    profile.token_origins.push(token.origin.clone());
    let secret = RegistryCredential::Basic {
        username: "fixture-user".into(),
        password: "fixture-password".into(),
    };
    assert!(!format!("{secret:?}").contains("fixture-password"));
    let client = PluginRegistryClient::new(profile, secret).expect("client");
    client
        .push(&directory.path().join("layout"), &registry.reference(":v1"))
        .await
        .expect("authenticated push");
    assert!(
        token
            .state
            .lock()
            .expect("state")
            .requests
            .iter()
            .all(|(_, path, _)| path.contains("scope=repository%3Ateam%2Fdemo%3A"))
    );
    registry.state.lock().expect("state").deny = true;
    let denial = client
        .pull(&registry.reference(":v1"), &directory.path().join("denied"))
        .await
        .expect_err("RBAC denied")
        .to_string();
    assert!(denial.contains("RBAC denied"));
    assert!(!denial.contains("fixture-password"));
}

#[tokio::test]
async fn interrupted_upload_queries_committed_range_before_resuming() {
    let directory = tempfile::tempdir().expect("temporary");
    let artifact = layout(directory.path());
    let registry = RegistryFixture::start().await;
    registry.state.lock().expect("state").interrupt_next_patch = true;
    registry
        .client()
        .push(&directory.path().join("layout"), &registry.reference(":v1"))
        .await
        .expect("resumed push");
    let state = registry.state.lock().expect("state");
    assert!(
        state
            .requests
            .iter()
            .any(|(method, path, _)| method == "GET" && path.starts_with("/upload/"))
    );
    assert_eq!(
        state.blobs[&artifact.parsed_manifest.config.digest],
        artifact.config
    );
    assert_eq!(
        state.blobs[&artifact.parsed_manifest.layers[0].digest],
        artifact.layer
    );
}

#[tokio::test]
async fn interrupted_empty_upload_restarts_instead_of_skipping_the_first_byte() {
    let directory = tempfile::tempdir().expect("temporary");
    let artifact = layout(directory.path());
    let registry = RegistryFixture::start().await;
    registry.state.lock().expect("state").interrupt_empty_patch = true;
    registry
        .client()
        .push(&directory.path().join("layout"), &registry.reference(":v1"))
        .await
        .expect("restarted push");
    let state = registry.state.lock().expect("state");
    assert!(
        state
            .requests
            .iter()
            .any(|(method, _, _)| method == "DELETE")
    );
    assert_eq!(
        state.blobs[&artifact.parsed_manifest.config.digest],
        artifact.config
    );
}

#[tokio::test]
async fn blob_redirects_require_an_exact_origin_and_strip_registry_authorization() {
    let directory = tempfile::tempdir().expect("temporary");
    let artifact = layout(directory.path());
    let registry = RegistryFixture::start().await;
    let objects = RegistryFixture::start().await;
    registry
        .client()
        .push(&directory.path().join("layout"), &registry.reference(":v1"))
        .await
        .expect("seed registry");
    objects.state.lock().expect("objects").blobs =
        registry.state.lock().expect("registry").blobs.clone();
    registry.state.lock().expect("registry").blob_redirect = Some(objects.origin.clone());
    assert!(
        registry
            .client()
            .pull(&registry.reference(":v1"), &directory.path().join("denied"))
            .await
            .expect_err("unconfigured redirect")
            .to_string()
            .contains("not permitted")
    );
    assert!(objects.state.lock().expect("objects").requests.is_empty());
    let mut profile = registry.profile();
    profile.blob_redirect_origins.push(objects.origin.clone());
    let client = PluginRegistryClient::new(
        profile,
        RegistryCredential::Bearer("fixture-registry-secret".into()),
    )
    .expect("client");
    let pulled = client
        .pull(
            &registry.reference(":v1"),
            &directory.path().join("allowed"),
        )
        .await
        .expect("permitted redirect");
    assert_eq!(pulled.manifest_digest, artifact.manifest_digest);
    let state = objects.state.lock().expect("objects");
    assert_eq!(state.requests.len(), 2);
    assert!(
        state
            .requests
            .iter()
            .all(|(_, _, authorization)| authorization.is_none())
    );
}
