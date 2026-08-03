use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_contracts::{DecisionOutcome, EffectPhase, PolicyDecision, ProviderEvent};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, ExecutionError, GatewayError,
    ReleasedEffectObserver, ReleasedEffectResult, SafetyKernel, effect_request, system_actor,
};
use colossus_ports::{EventJournal, PolicyDecisionPoint};
use colossus_testkit::InMemoryEventJournal;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
};
use rustls::{
    ServerConfig,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

struct CountingCredentialResolver {
    calls: AtomicUsize,
}

struct CountingHostCredentialResolver {
    calls: AtomicUsize,
    resolver: HostCredentialResolver,
}

struct ProviderPostDenyPolicy(BuiltInPolicy);

fn model_request_with_tools(names: &[&str]) -> ModelRequest {
    ModelRequest {
        instructions: "test".into(),
        messages: vec![ModelMessage {
            role: ModelMessageRole::User,
            content: "use a tool".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        tools: names
            .iter()
            .map(|name| ModelToolDefinition {
                name: (*name).into(),
                description: "Test tool.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            })
            .collect(),
        max_output_tokens: None,
    }
}

#[async_trait]
impl PolicyDecisionPoint for ProviderPostDenyPolicy {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<PolicyDecision, colossus_ports::PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "provider content denied by post-effect policy".into();
        }
        Ok(decision)
    }

    async fn doctor(&self) -> Result<Value, colossus_ports::PolicyError> {
        self.0.doctor().await
    }
}

impl CountingCredentialResolver {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl CredentialResolver for CountingCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, ProviderError> {
        assert_eq!(reference, "env:UNIT_PROVIDER_KEY");
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok("unit-secret".into())
    }
}

impl CountingHostCredentialResolver {
    fn new(secret: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            resolver: HostCredentialResolver::new([("provider-main".into(), secret.to_owned())])
                .expect("host credentials"),
        }
    }
}

impl CredentialResolver for CountingHostCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, ProviderError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.resolver.resolve(reference)
    }
}

#[test]
fn host_credentials_are_strict_bounded_and_debug_redacted() {
    let resolver =
        HostCredentialResolver::new([("provider-main".into(), "must-not-appear".into())])
            .expect("host credentials");
    assert_eq!(
        resolver.resolve("host:provider-main").expect("resolved"),
        "must-not-appear"
    );
    assert!(!format!("{resolver:?}").contains("must-not-appear"));

    let missing = resolver
        .resolve("host:provider-other")
        .expect_err("unknown credential");
    assert!(!missing.to_string().contains("must-not-appear"));
    assert!(HostCredentialResolver::new([("bad/id".into(), "secret".into())]).is_err());
    assert!(
        HostCredentialResolver::new([
            ("duplicate".into(), "first".into()),
            ("duplicate".into(), "second".into()),
        ])
        .is_err()
    );
    assert!(HostCredentialResolver::new([("empty".into(), String::new())]).is_err());
    assert!(
        HostCredentialResolver::new(
            (0..=64).map(|index| (format!("provider-{index}"), "secret".into()))
        )
        .is_err()
    );
}

#[test]
fn provider_profiles_accept_only_valid_credential_references() {
    for reference in ["env:OPENAI_API_KEY", "host:provider-main"] {
        ProviderProfile::new(
            "remote",
            ProviderKind::OpenAiResponses,
            Some("https://api.example.com/v1".into()),
            Some(reference.into()),
            1_000,
        )
        .expect("valid credential reference");
    }
    let codex = ProviderProfile::new(
        "codex",
        ProviderKind::OpenAiCodex,
        None,
        Some("codex:default".into()),
        1_000,
    )
    .expect("valid Codex profile");
    assert_eq!(codex.base_url.as_deref(), Some(CODEX_API_BASE_URL));
    assert_eq!(
        codex.generation_endpoint().expect("Codex endpoint"),
        "https://chatgpt.com/backend-api/codex/responses"
    );
    assert_eq!(
        codex
            .models_endpoint()
            .expect("Codex models endpoint")
            .expect("Codex has models endpoint"),
        format!(
            "https://chatgpt.com/backend-api/codex/models?client_version={CODEX_PROTOCOL_VERSION}"
        )
    );
    assert!(
        ProviderProfile::new(
            "codex",
            ProviderKind::OpenAiCodex,
            Some("https://example.com".into()),
            Some("codex:default".into()),
            1_000,
        )
        .is_err()
    );
    assert!(
        ProviderProfile::new(
            "codex",
            ProviderKind::OpenAiCodex,
            None,
            Some("env:OPENAI_API_KEY".into()),
            1_000,
        )
        .is_err()
    );
    for reference in [
        "host:",
        "host:provider/main",
        "host:provider:main",
        "value",
        CODEX_CREDENTIAL_REFERENCE,
    ] {
        for kind in [
            ProviderKind::OpenAiResponses,
            ProviderKind::OpenAiCompatible,
        ] {
            assert!(
                ProviderProfile::new(
                    "remote",
                    kind,
                    Some("https://api.example.com/v1".into()),
                    Some(reference.into()),
                    1_000,
                )
                .is_err(),
                "invalid reference was accepted: {reference} ({})",
                kind.as_str()
            );
        }
    }
}

fn test_jwt(claims: Value) -> String {
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims serialize"))
    )
}

fn write_test_codex_auth(path: &Path, expires_at: i64) {
    let auth = json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": test_jwt(json!({"https://api.openai.com/auth": {
                "chatgpt_account_id": "account-secret"
            }})),
            "access_token": test_jwt(json!({"exp": expires_at})),
            "refresh_token": "refresh-secret"
        }
    });
    fs::write(path, serde_json::to_vec(&auth).expect("auth serializes")).expect("auth writes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("permissions update");
    }
}

#[test]
fn model_profiles_derive_effective_input_budget_and_reject_exhausted_windows() {
    let profile = ModelProfile::new(
        "primary",
        "provider",
        "model",
        10_001,
        2_000,
        ModelCapabilities {
            tool_calls: false,
            streaming: false,
        },
        Some(ReasoningEffort::XHigh),
    )
    .expect("model profile");
    assert_eq!(profile.limits.safety_margin_tokens, 1_001);
    assert_eq!(profile.limits.input_budget_tokens, 7_000);
    assert!(!profile.capabilities.tool_calls);
    assert!(!profile.capabilities.streaming);
    assert_eq!(profile.reasoning_effort, Some(ReasoningEffort::XHigh));

    assert!(
        ModelProfile::new(
            "exhausted",
            "provider",
            "model",
            4_096,
            3_584,
            ModelCapabilities {
                tool_calls: true,
                streaming: true,
            },
            None,
        )
        .is_err()
    );
}

#[test]
fn service_unavailable_is_recoverable_without_reclassifying_bad_requests() {
    assert!(matches!(
        crate::executor::provider_execution_error(ProviderError::Status {
            status: 503,
            retry_after_ms: Some(7_000),
        }),
        ExecutionError::Recoverable {
            ref code,
            ref message,
            http_status: Some(503),
            retry_after_ms: Some(7_000),
        }
            if code == "provider.temporarily_unavailable"
                && message.contains("retry after the endpoint reports ready")
    ));
    assert!(matches!(
        crate::executor::provider_execution_error(ProviderError::Status {
            status: 400,
            retry_after_ms: None,
        }),
        ExecutionError::HttpStatus {
            status: 400,
            ref message,
        }
            if message == "provider endpoint returned HTTP 400"
    ));
}

fn model_request() -> ModelRequest {
    ModelRequest {
        instructions: "Be exact.".into(),
        messages: vec![ModelMessage {
            role: ModelMessageRole::User,
            content: "hello".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        tools: Vec::new(),
        max_output_tokens: None,
    }
}

#[test]
fn request_output_ceiling_must_match_the_resolved_model_limit() {
    let mut request = model_request();
    assert!(validate_model_request(&request, 4_096).is_ok());

    request.max_output_tokens = Some(4_096);
    assert!(validate_model_request(&request, 4_096).is_ok());
    assert!(validate_model_request(&request, 2_048).is_err());

    request.max_output_tokens = Some(0);
    assert!(validate_model_request(&request, 4_096).is_err());
    request.max_output_tokens = None;
    assert!(validate_model_request(&request, 0).is_err());
}

#[test]
fn provider_tool_schemas_require_an_explicit_object_root() {
    let mut request = model_request_with_tools(&["workspace.inspect"]);
    assert!(validate_model_request(&request, 4_096).is_ok());

    for schema in [
        json!(null),
        json!([]),
        json!({}),
        json!({"type": "array", "items": {"type": "string"}}),
        json!({"type": ["object"]}),
    ] {
        request.tools[0].input_schema = schema;
        assert!(matches!(
            validate_model_request(&request, 4_096),
            Err(ProviderError::Configuration(message))
                if message.contains("schema root must declare type object")
        ));
    }
}

fn provider_request(profile: &ProviderProfile) -> EffectRequest {
    let mut request = effect_request(
        system_actor("provider-test"),
        profile.kind.generation_action(),
        profile.generation_endpoint().expect("generation endpoint"),
        serde_json::to_value(ProviderEffectInput {
            provider_profile: profile.name.clone(),
            model_profile: Some("unit-profile".into()),
            model: Some("unit-model".into()),
            max_output_tokens: Some(4_096),
            reasoning_effort: None,
            request: Some(model_request()),
            include_response_diagnostics: false,
        })
        .expect("effect input"),
    );
    request.capabilities = vec!["provider.call".into()];
    request.credential_references = profile
        .credential_reference
        .as_ref()
        .map(|reference| CredentialReference {
            reference: reference.clone(),
            value_hash: None,
        })
        .into_iter()
        .collect();
    request
}

async fn one_response_server(body: Value) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut scratch = [0_u8; 4096];
        loop {
            let read = stream.read(&mut scratch).await.expect("read request");
            assert_ne!(read, 0, "client closed before completing request");
            request.extend_from_slice(&scratch[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request).into_owned();
        let response_body = serde_json::to_vec(&body).expect("response JSON");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            response_body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write headers");
        stream.write_all(&response_body).await.expect("write body");
        request_text
    });
    (format!("http://{address}/v1"), task)
}

async fn one_status_server(
    status: u16,
    reason: &'static str,
    retry_after: Option<&'static str>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut scratch = [0_u8; 4096];
        loop {
            let read = stream.read(&mut scratch).await.expect("read request");
            assert_ne!(read, 0, "client closed before completing request");
            request.extend_from_slice(&scratch[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let retry_after_header = retry_after
            .map(|value| format!("retry-after: {value}\r\n"))
            .unwrap_or_default();
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\n{retry_after_header}content-length: 0\r\nconnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });
    (format!("http://{address}/v1"), task)
}

async fn one_status_body_server(
    status: u16,
    reason: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut scratch = [0_u8; 4096];
        loop {
            let read = stream.read(&mut scratch).await.expect("read request");
            assert_ne!(read, 0, "client closed before completing request");
            request.extend_from_slice(&scratch[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        String::from_utf8_lossy(&request).into_owned()
    });
    (format!("http://{address}/v1"), task)
}

async fn one_tls_response_server(
    body: Value,
) -> (
    String,
    AdditionalRootCertificates,
    tokio::task::JoinHandle<String>,
) {
    let mut ca_params = CertificateParams::new(vec!["Colossus Test CA".into()]).expect("CA params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().expect("CA key"))
        .expect("CA certificate");
    let mut server_params =
        CertificateParams::new(vec!["127.0.0.1".into()]).expect("server params");
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_key = KeyPair::generate().expect("server key");
    let server_certificate = server_params
        .signed_by(&server_key, &ca)
        .expect("server certificate");
    let roots =
        AdditionalRootCertificates::from_pem_bundle(ca.pem().as_bytes()).expect("test CA bundle");
    let server_config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![server_certificate.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
            )
            .expect("TLS server config");
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut stream = acceptor.accept(stream).await.expect("TLS handshake");
        let mut request = Vec::new();
        let mut scratch = [0_u8; 4096];
        loop {
            let read = stream.read(&mut scratch).await.expect("read request");
            assert_ne!(read, 0, "client closed before completing request");
            request.extend_from_slice(&scratch[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request).into_owned();
        let response_body = serde_json::to_vec(&body).expect("response JSON");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            response_body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write headers");
        stream.write_all(&response_body).await.expect("write body");
        request_text
    });
    (format!("https://{address}/v1"), roots, task)
}

async fn one_sse_server(body: String) -> (String, tokio::task::JoinHandle<String>) {
    one_sse_server_with_content_type(body, Some("text/event-stream")).await
}

async fn one_sse_server_with_content_type(
    body: String,
    content_type: Option<&'static str>,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut scratch = [0_u8; 4096];
        loop {
            let read = stream.read(&mut scratch).await.expect("read request");
            assert_ne!(read, 0, "client closed before completing request");
            request.extend_from_slice(&scratch[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request).into_owned();
        let content_type_header = content_type
            .map(|value| format!("content-type: {value}\r\n"))
            .unwrap_or_default();
        let response = format!(
            "HTTP/1.1 200 OK\r\n{content_type_header}content-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write headers");
        for chunk in body.as_bytes().chunks(17) {
            stream.write_all(chunk).await.expect("write SSE chunk");
            tokio::task::yield_now().await;
        }
        request_text
    });
    (format!("http://{address}/v1"), task)
}

#[derive(Default)]
struct ReleasedItems(Vec<ProviderStreamItem>);

#[async_trait]
impl ReleasedEffectObserver for ReleasedItems {
    async fn observe(&mut self, result: ReleasedEffectResult) -> Result<(), ExecutionError> {
        let item = serde_json::from_slice(&result.bytes)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        self.0.push(item);
        Ok(())
    }
}

#[test]
fn malformed_tool_arguments_fail_closed() {
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some("http://127.0.0.1:9000/v1".into()),
        None,
        1_000,
    )
    .expect("profile");
    let malformed = json!({
        "id": "response-1",
        "choices": [{"message": {
            "role": "assistant",
            "tool_calls": [{
                "id": "call-1",
                "function": {"name": "lookup", "arguments": "[]"}
            }]
        }}]
    });
    let error = normalize_chat(
        &profile,
        "unit-profile",
        "unit-model",
        &serde_json::to_vec(&malformed).expect("JSON"),
        &ProviderToolNames::default(),
    )
    .expect_err("non-object arguments must fail");
    assert!(matches!(error, ProviderError::Malformed(_)));
}

#[test]
fn responses_output_normalizes_visible_text_and_strict_tool_calls() {
    let profile = ProviderProfile::new(
        "openai",
        ProviderKind::OpenAiResponses,
        Some("https://api.openai.com/v1".into()),
        Some("env:UNIT_PROVIDER_KEY".into()),
        1_000,
    )
    .expect("profile");
    let response = json!({
        "id": "response-1",
        "output": [
            {"type": "reasoning", "summary": [
                {"type": "summary_text", "text": "safe plan"}
            ], "content": "hidden reasoning"},
            {"type": "message", "content": [
                {"type": "output_text", "text": "working"}
            ]},
            {"type": "function_call", "call_id": "call-1", "name": "workspace_inspect",
             "arguments": "{\"query\":\"rust\"}"}
        ]
    });
    let request = model_request_with_tools(&["workspace.inspect"]);
    let tool_names = ProviderToolNames::from_request(&request).expect("provider tool names");
    let turn = normalize_responses(
        &profile,
        "unit-profile",
        "unit-model",
        &serde_json::to_vec(&response).expect("JSON"),
        &tool_names,
    )
    .expect("normalized response");
    assert!(matches!(
        &turn.events[0],
        ProviderEvent::ReasoningSummary { summary } if summary == "safe plan"
    ));
    assert!(matches!(
        &turn.events[1],
        ProviderEvent::ModelDelta { text } if text == "working"
    ));
    assert!(matches!(
        &turn.events[2],
        ProviderEvent::ToolCallRequested { call_id, name, arguments }
            if call_id == "call-1"
                && name == "workspace.inspect"
                && arguments["query"] == "rust"
    ));
    assert!(
        !serde_json::to_string(&turn)
            .expect("turn JSON")
            .contains("hidden reasoning")
    );
    assert!(
        !turn
            .events
            .iter()
            .any(|event| matches!(event, ProviderEvent::FinalOutput { .. })),
        "a turn requesting a tool must not be marked final"
    );
}

#[test]
fn model_catalog_normalizes_openai_and_codex_manifest_shapes() {
    let openai = normalize_models(
        &serde_json::to_vec(&json!({
            "data": [{"id": "gpt-openai", "object": "model", "owned_by": "openai"}]
        }))
        .expect("OpenAI models serialize"),
    )
    .expect("OpenAI models normalize");
    assert_eq!(openai[0].id, "gpt-openai");
    assert_eq!(openai[0].object.as_deref(), Some("model"));

    let codex = normalize_models(
        &serde_json::to_vec(&json!({
            "models": [{"slug": "gpt-codex", "display_name": "GPT Codex"}]
        }))
        .expect("Codex models serialize"),
    )
    .expect("Codex models normalize");
    assert_eq!(codex[0].id, "gpt-codex");
    assert_eq!(codex[0].object, None);
}

#[test]
fn responses_stream_normalizes_deltas_completion_and_usage() {
    let mut state = ResponsesStreamState::default();
    assert!(
        state
            .ingest(json!({
                "type": "response.created",
                "response": {"id": "resp-stream"}
            }))
            .expect("created")
            .is_empty()
    );
    let events = state
        .ingest(json!({
            "type": "response.output_text.delta",
            "delta": "hello"
        }))
        .expect("delta");
    assert!(matches!(
        &events[0],
        ProviderEvent::ModelDelta { text } if text == "hello"
    ));
    let events = state
        .ingest(json!({
            "type": "response.completed",
            "response": {
                "id": "resp-stream",
                "status": "completed",
                "output": [],
                "usage": {
                    "input_tokens": 5,
                    "output_tokens": 2,
                    "total_tokens": 7,
                    "input_tokens_details": {"cached_tokens": 1},
                    "output_tokens_details": {"reasoning_tokens": 1}
                }
            }
        }))
        .expect("completed");
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::FinalOutput { text } if text == "hello"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::Usage { usage }
            if usage.input_tokens == 5
                && usage.output_tokens == 2
                && usage.total_tokens == 7
                && usage.cached_input_tokens == Some(1)
                && usage.reasoning_tokens == Some(1)
    )));
    assert!(state.finish().expect("finished").is_empty());
}

#[test]
fn provider_tool_names_alias_dots_and_reject_ambiguous_or_nonportable_names() {
    let request = model_request_with_tools(&["workspace.inspect"]);
    let names = ProviderToolNames::from_request(&request).expect("portable alias");
    assert_eq!(
        names
            .provider_name("workspace.inspect")
            .expect("provider name"),
        "workspace_inspect"
    );
    assert_eq!(
        names.canonical_name("workspace_inspect"),
        "workspace.inspect"
    );

    let collision = model_request_with_tools(&["workspace.inspect", "workspace_inspect"]);
    assert!(matches!(
        ProviderToolNames::from_request(&collision),
        Err(ProviderError::Configuration(_))
    ));

    let nonportable = model_request_with_tools(&["workspace/inspect"]);
    assert!(matches!(
        ProviderToolNames::from_request(&nonportable),
        Err(ProviderError::Configuration(_))
    ));
}

#[test]
fn streamed_chat_tool_aliases_are_restored_to_canonical_names() {
    let request = model_request_with_tools(&["filesystem.write"]);
    let names = ProviderToolNames::from_request(&request).expect("provider tool names");
    let mut state = ProviderStreamState::new(ProviderKind::OpenAiCompatible, names);
    assert!(
        state
            .ingest(json!({
                "id": "chat-tool-alias",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call-1",
                            "type": "function",
                            "function": {
                                "name": "filesystem_write",
                                "arguments": "{\"path\":\"README.md\"}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }))
            .expect("stream chunk")
            .is_empty()
    );
    let events = state.finish().expect("stream completion");
    assert!(matches!(
        &events[0],
        ProviderEvent::ToolCallRequested { call_id, name, arguments }
            if call_id == "call-1"
                && name == "filesystem.write"
                && arguments["path"] == "README.md"
    ));
}

#[test]
fn incomplete_sse_and_unterminated_chat_streams_fail_closed() {
    let mut decoder = SseDecoder::default();
    assert!(
        decoder
            .feed(br#"data: {"id":"chat-1"}"#)
            .expect("buffered partial frame")
            .is_empty()
    );
    assert!(matches!(decoder.finish(), Err(ProviderError::Transport(_))));

    let mut state = ChatStreamState::default();
    let events = state
        .ingest(json!({
            "id": "chat-1",
            "choices": [{
                "index": 0,
                "delta": {"content": "partial"},
                "finish_reason": null
            }]
        }))
        .expect("partial chat chunk");
    assert!(matches!(
        &events[0],
        ProviderEvent::ModelDelta { text } if text == "partial"
    ));
    assert!(matches!(state.finish(), Err(ProviderError::Transport(_))));
}

#[test]
fn continuation_payloads_preserve_assistant_call_and_tool_result_ids() {
    let request = ModelRequest {
        instructions: "test".into(),
        messages: vec![
            ModelMessage {
                role: ModelMessageRole::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call-1".into(),
                    name: "workspace.inspect".into(),
                    arguments: json!({"query": "rust"}),
                }],
            },
            ModelMessage {
                role: ModelMessageRole::Tool,
                content: "result".into(),
                tool_call_id: Some("call-1".into()),
                tool_calls: Vec::new(),
            },
        ],
        tools: Vec::new(),
        max_output_tokens: None,
    };
    let tool_names = ProviderToolNames::from_request(&request).expect("provider tool names");
    let responses = responses_payload(
        &request,
        ProviderKind::OpenAiResponses,
        "unit-model",
        4_096,
        None,
        false,
        &tool_names,
    )
    .expect("Responses payload");
    assert_eq!(responses["model"], "unit-model");
    assert_eq!(responses["max_output_tokens"], 4_096);
    assert_eq!(responses["input"][0]["type"], "function_call");
    assert_eq!(responses["input"][0]["call_id"], "call-1");
    assert_eq!(responses["input"][0]["name"], "workspace_inspect");
    assert_eq!(responses["input"][1]["type"], "function_call_output");
    assert_eq!(responses["input"][1]["call_id"], "call-1");

    let chat = chat_payload(&request, "unit-model", 4_096, None, false, &tool_names)
        .expect("chat payload");
    assert_eq!(chat["model"], "unit-model");
    assert_eq!(chat["max_tokens"], 4_096);
    assert_eq!(chat["messages"][1]["tool_calls"][0]["id"], "call-1");
    assert_eq!(
        chat["messages"][1]["tool_calls"][0]["function"]["name"],
        "workspace_inspect"
    );
    assert_eq!(chat["messages"][2]["tool_call_id"], "call-1");
}

#[test]
fn codex_responses_payload_omits_unsupported_output_token_parameter() {
    let request = model_request_with_tools(&[]);
    let tool_names = ProviderToolNames::from_request(&request).expect("provider tool names");
    let payload = responses_payload(
        &request,
        ProviderKind::OpenAiCodex,
        "unit-model",
        4_096,
        None,
        true,
        &tool_names,
    )
    .expect("Codex Responses payload");

    assert!(payload.get("max_output_tokens").is_none());
    assert_eq!(payload["stream"], true);
}

#[test]
fn reasoning_effort_uses_each_provider_protocol_shape_and_is_optional() {
    let request = model_request_with_tools(&[]);
    let tool_names = ProviderToolNames::from_request(&request).expect("provider tool names");
    let responses = responses_payload(
        &request,
        ProviderKind::OpenAiResponses,
        "unit-model",
        4_096,
        Some(ReasoningEffort::XHigh),
        false,
        &tool_names,
    )
    .expect("Responses payload");
    assert_eq!(responses["reasoning"]["effort"], "xhigh");

    let chat = chat_payload(
        &request,
        "unit-model",
        4_096,
        Some(ReasoningEffort::Ultra),
        false,
        &tool_names,
    )
    .expect("Chat Completions payload");
    assert_eq!(chat["reasoning_effort"], "ultra");

    let provider_default = responses_payload(
        &request,
        ProviderKind::OpenAiCodex,
        "unit-model",
        4_096,
        None,
        false,
        &tool_names,
    )
    .expect("Codex payload using provider default");
    assert!(provider_default.get("reasoning").is_none());
}

#[test]
fn openai_tool_projection_is_compatible_without_mutating_the_canonical_schema() {
    let tool = ModelToolDefinition {
        name: "workspace.inspect".into(),
        description: "Inspect bounded workspace paths.".into(),
        input_schema: json!({
            "type": "object",
            "oneOf": [{"required": ["paths"]}, {"required": ["environment"]}],
            "anyOf": [{"required": ["paths"]}],
            "allOf": [{"properties": {"mode": {"type": "string"}}}],
            "enum": [{}],
            "const": {},
            "properties": {
                "paths": {
                    "type": "array",
                    "maxItems": 128,
                    "items": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 4096
                    }
                },
                "selector": {
                    "oneOf": [{"type": "string"}, {"type": "integer"}]
                },
                "mode": {"type": "string", "const": "safe"},
                "environment": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "string",
                        "maxLength": 65536
                    }
                }
            },
            "required": ["paths"],
            "additionalProperties": false
        }),
    };
    let canonical_schema = tool.input_schema.clone();
    let request = ModelRequest {
        instructions: "test".into(),
        messages: vec![ModelMessage {
            role: ModelMessageRole::User,
            content: "inspect".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        tools: vec![tool.clone()],
        max_output_tokens: Some(4_096),
    };

    let tool_names = ProviderToolNames::from_request(&request).expect("provider tool names");
    let chat = chat_payload(&request, "unit-model", 4_096, None, false, &tool_names)
        .expect("chat payload");
    assert_eq!(chat["tools"][0]["function"]["name"], "workspace_inspect");
    let projected = &chat["tools"][0]["function"]["parameters"];
    for keyword in ["oneOf", "anyOf", "allOf", "enum", "const"] {
        assert!(projected.get(keyword).is_none(), "root {keyword} remained");
    }
    assert!(
        !serde_json::to_string(projected)
            .expect("projected schema")
            .contains("\"maxLength\"")
    );
    assert_eq!(projected["properties"]["paths"]["items"]["minLength"], 1);
    assert_eq!(projected["properties"]["paths"]["maxItems"], 128);
    assert_eq!(
        projected["properties"]["environment"]["additionalProperties"]["type"],
        "string"
    );
    assert_eq!(
        projected["properties"]["selector"]["oneOf"][0]["type"],
        "string"
    );
    assert_eq!(projected["properties"]["mode"]["const"], "safe");
    assert!(chat["tools"][0]["function"].get("strict").is_none());

    let responses = responses_payload(
        &request,
        ProviderKind::OpenAiResponses,
        "unit-model",
        4_096,
        None,
        false,
        &tool_names,
    )
    .expect("Responses payload");
    assert_eq!(responses["tools"][0]["name"], "workspace_inspect");
    assert_eq!(responses["tools"][0]["strict"], false);
    let response_schema = &responses["tools"][0]["parameters"];
    for keyword in ["oneOf", "anyOf", "allOf", "enum", "const"] {
        assert!(
            response_schema.get(keyword).is_none(),
            "root {keyword} remained"
        );
    }
    assert_eq!(
        response_schema["properties"]["paths"]["items"]["maxLength"],
        4096
    );
    assert_eq!(
        response_schema["properties"]["selector"]["oneOf"][1]["type"],
        "integer"
    );
    assert_eq!(response_schema["properties"]["mode"]["const"], "safe");
    assert_eq!(tool.input_schema, canonical_schema);
}

#[test]
fn representative_builtin_schemas_project_to_openai_compatible_roots() {
    let specs = colossus_tools::builtin_specs();
    for (name, canonical_root_keyword, description_fragment) in [
        ("shell.run", "oneOf", "exactly one of command or argv"),
        ("skill.validate", "oneOf", "exactly one of"),
        ("user.ask", "allOf", "allow_free_form is false"),
    ] {
        let spec = specs
            .iter()
            .find(|spec| spec.name == name)
            .unwrap_or_else(|| panic!("missing {name} spec"));
        assert_eq!(spec.input_schema["type"], "object");
        assert!(spec.input_schema.get(canonical_root_keyword).is_some());
        assert!(spec.description.contains(description_fragment));
        let canonical_schema = spec.input_schema.clone();
        let request = ModelRequest {
            instructions: "test".into(),
            messages: vec![ModelMessage {
                role: ModelMessageRole::User,
                content: "use the tool".into(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            tools: vec![ModelToolDefinition {
                name: spec.name.clone(),
                description: spec.description.clone(),
                input_schema: spec.input_schema.clone(),
            }],
            max_output_tokens: Some(4_096),
        };
        validate_model_request(&request, 4_096).expect("valid canonical tool root");
        let tool_names = ProviderToolNames::from_request(&request).expect("provider tool names");
        let responses = responses_payload(
            &request,
            ProviderKind::OpenAiResponses,
            "unit-model",
            4_096,
            None,
            false,
            &tool_names,
        )
        .expect("Responses payload");
        let chat = chat_payload(&request, "unit-model", 4_096, None, false, &tool_names)
            .expect("Chat Completions payload");

        for projected in [
            &responses["tools"][0]["parameters"],
            &chat["tools"][0]["function"]["parameters"],
        ] {
            assert_eq!(projected["type"], "object");
            for keyword in ["oneOf", "anyOf", "allOf", "enum", "const"] {
                assert!(
                    projected.get(keyword).is_none(),
                    "{name} retained root {keyword}"
                );
            }
        }
        assert_eq!(responses["tools"][0]["strict"], false);
        assert!(chat["tools"][0]["function"].get("strict").is_none());
        assert_eq!(request.tools[0].input_schema, canonical_schema);
    }
}

#[test]
fn hidden_reasoning_is_not_released_but_safe_summary_is() {
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some("http://127.0.0.1:9000/v1".into()),
        None,
        1_000,
    )
    .expect("profile");
    let response = json!({
        "id": "response-1",
        "choices": [{"message": {
            "role": "assistant",
            "content": "visible",
            "reasoning": "private chain of thought",
            "reasoning_details": [
                {"type": "reasoning.encrypted", "text": "ciphertext"},
                {"type": "reasoning.summary", "summary": "safe summary"}
            ]
        }}]
    });
    let turn = normalize_chat(
        &profile,
        "unit-profile",
        "unit-model",
        &serde_json::to_vec(&response).expect("JSON"),
        &ProviderToolNames::default(),
    )
    .expect("normalized turn");
    assert!(turn.events.iter().any(|event| matches!(
        event,
        ProviderEvent::ReasoningSummary { summary } if summary == "safe summary"
    )));
    let released = serde_json::to_string(&turn).expect("turn JSON");
    assert!(!released.contains("private chain of thought"));
    assert!(!released.contains("ciphertext"));
}

#[tokio::test]
async fn denial_happens_before_credential_resolution() {
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some("http://127.0.0.1:9/v1".into()),
        Some("host:provider-main".into()),
        1_000,
    )
    .expect("profile");
    let credentials = Arc::new(CountingHostCredentialResolver::new("unit-secret"));
    let executor = ProviderExecutor::with_credentials(
        profile.clone(),
        Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
    );
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        journal,
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [7_u8; 32],
    );
    let error = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect_err("policy must deny provider call");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert_eq!(credentials.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn allowed_provider_call_is_permit_bound_and_post_released() {
    let (base_url, server) = one_response_server(json!({
        "id": "response-1",
        "choices": [{"message": {"role": "assistant", "content": "hello back"}}]
    }))
    .await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        Some("env:UNIT_PROVIDER_KEY".into()),
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network provider origin");
    let credentials = Arc::new(CountingCredentialResolver::new());
    let executor = ProviderExecutor::with_credentials(
        profile.clone(),
        Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
    );
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [8_u8; 32],
    );
    let released = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect("allowed provider call");
    let turn: ProviderTurn = serde_json::from_slice(&released.bytes).expect("provider turn");
    assert!(matches!(
        turn.events.last(),
        Some(ProviderEvent::FinalOutput { text }) if text == "hello back"
    ));
    assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
    let raw_request = server.await.expect("server task");
    assert!(raw_request.contains("POST /v1/chat/completions HTTP/1.1"));
    assert!(
        raw_request
            .to_ascii_lowercase()
            .contains("authorization: bearer unit-secret")
    );
    let event_types = journal
        .read_global(1, 50)
        .expect("journal events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"effect.release_requested.v1".into()));
    assert!(event_types.contains(&"effect.completed.v1".into()));
}

#[tokio::test]
async fn codex_provider_uses_chatgpt_account_headers_and_responses_shape() {
    let body = [
        r#"data: {"type":"response.created","response":{"id":"response-codex"}}

"#,
        r#"data: {"type":"response.output_text.delta","delta":"subscription response"}

"#,
        r#"data: {"type":"response.completed","response":{"id":"response-codex","status":"completed","output":[]}}

"#,
    ]
    .concat();
    let (base_url, server) = one_sse_server_with_content_type(body, None).await;
    let mut profile = ProviderProfile::new(
        "codex",
        ProviderKind::OpenAiCodex,
        None,
        Some(CODEX_CREDENTIAL_REFERENCE.into()),
        5_000,
    )
    .expect("Codex profile");
    profile.base_url = Some(base_url);
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network provider origin");
    let directory = tempfile::tempdir().expect("tempdir");
    let auth_path = directory.path().join("auth.json");
    let expires_at = 4_102_444_800;
    write_test_codex_auth(&auth_path, expires_at);
    let expected_access_token = test_jwt(json!({"exp": expires_at}));
    let executor = ProviderExecutor::new(profile.clone())
        .with_codex_auth_store(CodexAuthStore::at_path(auth_path));
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [18_u8; 32],
    );
    let mut released = ReleasedItems::default();
    let mut effect = provider_request(&profile);
    effect.content["reasoning_effort"] = json!("xhigh");
    gateway
        .execute_stream(effect, &executor, &mut released)
        .await
        .expect("Codex request succeeds");
    assert!(released.0.iter().any(|item| matches!(
        item,
        ProviderStreamItem::Event {
            event: ProviderEvent::FinalOutput { text }
        } if text == "subscription response"
    )));
    let request = server.await.expect("server task");
    let request_lower = request.to_ascii_lowercase();
    assert!(request.contains("POST /v1/responses HTTP/1.1"));
    assert!(request_lower.contains(&format!(
        "authorization: bearer {}",
        expected_access_token.to_ascii_lowercase()
    )));
    assert!(request_lower.contains("chatgpt-account-id: account-secret"));
    assert!(request_lower.contains("originator: codex colossus"));
    assert!(request_lower.contains(&format!("version: {CODEX_PROTOCOL_VERSION}")));
    assert!(request_lower.contains("accept: text/event-stream"));
    assert!(request_lower.contains(concat!("user-agent: colossus/", env!("CARGO_PKG_VERSION"))));
    assert!(
        !request.contains("\"max_output_tokens\""),
        "Codex subscription requests must match the official Codex wire shape"
    );
    assert!(request.contains("\"reasoning\":{\"effort\":\"xhigh\"}"));
}

#[tokio::test]
async fn codex_refresh_requires_the_openai_auth_origin_in_the_permit() {
    let mut profile = ProviderProfile::new(
        "codex",
        ProviderKind::OpenAiCodex,
        None,
        Some(CODEX_CREDENTIAL_REFERENCE.into()),
        5_000,
    )
    .expect("Codex profile");
    profile.base_url = Some("http://127.0.0.1:9/v1".into());
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network provider origin");
    let directory = tempfile::tempdir().expect("tempdir");
    let auth_path = directory.path().join("auth.json");
    write_test_codex_auth(&auth_path, 1);
    let executor = ProviderExecutor::new(profile.clone())
        .with_codex_auth_store(CodexAuthStore::at_path(auth_path));
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [19_u8; 32],
    );
    let error = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect_err("refresh must not use an unauthorized origin");
    assert!(
        format!("{error:?}").contains("provider origin is absent from permit obligations"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn codex_request_secrets_redact_access_token_and_account_identifier() {
    let mut secrets = RequestSecrets::default();
    secrets.retain("access-secret");
    secrets.retain("account-secret");
    let mut bytes = b"access-secret account-secret safe".to_vec();
    secrets.redact_bytes(&mut bytes);
    assert_eq!(
        String::from_utf8(bytes).expect("UTF-8"),
        "[REDACTED] [REDACTED] safe"
    );
}

#[tokio::test]
async fn provider_retry_after_header_is_preserved_as_safe_recoverable_metadata() {
    let (base_url, server) = one_status_server(503, "Service Unavailable", Some("7")).await;
    let profile = ProviderProfile::new(
        "temporarily-unavailable",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network provider origin");
    let executor = ProviderExecutor::new(profile.clone());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [14_u8; 32],
    );
    let error = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect_err("503 must remain a recoverable provider failure");
    assert!(matches!(
        error,
        GatewayError::RecoverableExecution {
            ref code,
            http_status: Some(503),
            retry_after_ms: Some(7_000),
            ..
        } if code == "provider.temporarily_unavailable"
    ));
    server.await.expect("server task");
}

#[tokio::test]
async fn provider_https_accepts_the_runtime_ca_bundle() {
    let (base_url, tls_roots, server) = one_tls_response_server(json!({
        "id": "response-private-ca",
        "choices": [{"message": {"role": "assistant", "content": "trusted"}}]
    }))
    .await;
    let profile = ProviderProfile::new(
        "private-ca",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network provider origin");
    let executor = ProviderExecutor::new(profile.clone()).with_tls_roots(tls_roots);
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [13_u8; 32],
    );
    let released = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect("provider call through private CA");
    let turn: ProviderTurn = serde_json::from_slice(&released.bytes).expect("provider turn");
    assert!(matches!(
        turn.events.last(),
        Some(ProviderEvent::FinalOutput { text }) if text == "trusted"
    ));
    let request = server.await.expect("server task");
    assert!(request.contains("POST /v1/chat/completions HTTP/1.1"));
}

#[tokio::test]
async fn actual_provider_content_denied_post_effect_never_reaches_the_caller() {
    let secret = "provider-private-content";
    let (base_url, server) = one_response_server(json!({
        "id": "response-private",
        "choices": [{"message": {"role": "assistant", "content": secret}}]
    }))
    .await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network provider origin");
    let executor = ProviderExecutor::new(profile.clone());
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(ProviderPostDenyPolicy(policy)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [12_u8; 32],
    );
    let error = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect_err("post-effect provider denial");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert!(!error.to_string().contains(secret));
    let request = server.await.expect("server task");
    assert!(request.contains("POST /v1/chat/completions HTTP/1.1"));

    let events = journal.read_global(1, 50).expect("journal events");
    let event_types = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"effect.started.v1"));
    assert!(event_types.contains(&"effect.release_requested.v1"));
    assert!(event_types.contains(&"effect.release_denied.v1"));
    assert!(!event_types.contains(&"effect.completed.v1"));
    assert!(
        !serde_json::to_string(&events)
            .expect("event evidence")
            .contains(secret)
    );
}

#[tokio::test]
async fn compatible_sse_stream_releases_ordered_deltas_usage_and_completion() {
    let body = [
            r#"data: {"id":"chat-1","choices":[{"index":0,"delta":{"content":"con"},"finish_reason":null}]}

"#,
            r#"data: {"id":"chat-1","choices":[{"index":0,"delta":{"content":"nected"},"finish_reason":"stop"}]}

"#,
            r#"data: {"id":"chat-1","choices":[],"usage":{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9,"prompt_tokens_details":{"cached_tokens":3},"completion_tokens_details":{"reasoning_tokens":1}}}

"#,
            "data: [DONE]\n\n",
        ]
        .concat();
    let (base_url, server) = one_sse_server(body).await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network origin");
    let executor = ProviderExecutor::new(profile.clone());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [4_u8; 32],
    );
    let mut released = ReleasedItems::default();
    let terminal = gateway
        .execute_stream(provider_request(&profile), &executor, &mut released)
        .await
        .expect("streamed provider call");
    let terminal: ProviderStreamItem =
        serde_json::from_slice(&terminal.bytes).expect("terminal item");
    assert!(matches!(
        terminal,
        ProviderStreamItem::Completed {
            ref response_id, ..
        }
            if response_id.as_deref() == Some("chat-1")
    ));
    assert!(matches!(
        &released.0[0],
        ProviderStreamItem::Event { event: ProviderEvent::ModelDelta { text } }
            if text == "con"
    ));
    assert!(matches!(
        &released.0[1],
        ProviderStreamItem::Event { event: ProviderEvent::ModelDelta { text } }
            if text == "nected"
    ));
    assert!(released.0.iter().any(|item| matches!(
        item,
        ProviderStreamItem::Event { event: ProviderEvent::Usage { usage } }
            if usage.input_tokens == 7
                && usage.output_tokens == 2
                && usage.total_tokens == 9
                && usage.cached_input_tokens == Some(3)
                && usage.reasoning_tokens == Some(1)
    )));
    assert!(released.0.iter().any(|item| matches!(
        item,
        ProviderStreamItem::Event { event: ProviderEvent::FinalOutput { text } }
            if text == "connected"
    )));
    assert_eq!(released.0.last(), Some(&terminal));
    let raw_request = server.await.expect("server task");
    assert!(raw_request.contains("\"stream\":true"));
    assert!(raw_request.contains("\"include_usage\":true"));
    assert!(
        journal
            .read_global(1, 50)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "effect.chunk_released.v1")
    );
}

#[tokio::test]
async fn streamed_bad_request_releases_explicit_request_and_response_diagnostics() {
    let response_body = r#"{"error":{"message":"invalid dotted tool name"}}"#;
    let (base_url, server) = one_status_body_server(400, "Bad Request", response_body).await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network origin");
    let executor = ProviderExecutor::new(profile.clone());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [15_u8; 32],
    );
    let mut request = provider_request(&profile);
    request.content["include_response_diagnostics"] = Value::Bool(true);
    request.content["request"]["tools"] = json!([{
        "name": "tool.with.dots",
        "description": "A dotted test tool.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }
    }]);
    let mut released = ReleasedItems::default();
    let terminal = gateway
        .execute_stream(request, &executor, &mut released)
        .await
        .expect("explicit diagnostic must pass the release boundary");
    let terminal: ProviderStreamItem =
        serde_json::from_slice(&terminal.bytes).expect("provider diagnostic item");
    assert_eq!(released.0.last(), Some(&terminal));
    let ProviderStreamItem::Diagnostic { diagnostic } = terminal else {
        panic!("expected terminal provider diagnostic");
    };
    assert_eq!(diagnostic.status, 400);
    assert_eq!(diagnostic.body, response_body);
    assert_eq!(
        diagnostic.request_body.as_ref().and_then(|body| {
            body.pointer("/tools/0/function/name")
                .and_then(Value::as_str)
        }),
        Some("tool_with_dots")
    );
    let raw_request = server.await.expect("server task");
    assert!(raw_request.contains("\"name\":\"tool_with_dots\""));
    assert!(!raw_request.contains("\"name\":\"tool.with.dots\""));
    assert!(raw_request.contains("\"stream\":true"));
    let events = journal.read_global(1, 50).expect("events");
    assert!(
        !serde_json::to_string(&events)
            .expect("event evidence")
            .contains(response_body)
    );
}

#[tokio::test]
async fn streamed_non_sse_success_releases_explicit_response_diagnostics() {
    let response_body = json!({"detail": "stream negotiation failed"});
    let expected_body = serde_json::to_string(&response_body).expect("response JSON");
    let (base_url, server) = one_response_server(response_body).await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network origin");
    let executor = ProviderExecutor::new(profile.clone());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [25_u8; 32],
    );
    let mut request = provider_request(&profile);
    request.content["include_response_diagnostics"] = Value::Bool(true);
    let mut released = ReleasedItems::default();
    let terminal = gateway
        .execute_stream(request, &executor, &mut released)
        .await
        .expect("explicit diagnostic must pass the release boundary");
    let terminal: ProviderStreamItem =
        serde_json::from_slice(&terminal.bytes).expect("provider diagnostic item");
    let ProviderStreamItem::Diagnostic { diagnostic } = terminal else {
        panic!("expected terminal provider diagnostic");
    };
    assert_eq!(diagnostic.status, 200);
    assert_eq!(diagnostic.content_type.as_deref(), Some("application/json"));
    assert_eq!(diagnostic.body, expected_body);
    let raw_request = server.await.expect("server task");
    assert!(
        raw_request
            .to_ascii_lowercase()
            .contains("accept: text/event-stream")
    );
    assert!(raw_request.contains("\"stream\":true"));
}

#[tokio::test]
async fn streamed_credential_echo_is_redacted_before_release() {
    let body = [
            r#"data: {"id":"chat-secret","choices":[{"index":0,"delta":{"content":"unit-secret"},"finish_reason":"stop"}]}

"#,
            "data: [DONE]\n\n",
        ]
        .concat();
    let (base_url, server) = one_sse_server(body).await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        Some("host:provider-main".into()),
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network origin");
    let credentials = Arc::new(CountingHostCredentialResolver::new("unit-secret"));
    let executor = ProviderExecutor::with_credentials(
        profile.clone(),
        Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
    );
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [5_u8; 32],
    );
    let mut released = ReleasedItems::default();
    gateway
        .execute_stream(provider_request(&profile), &executor, &mut released)
        .await
        .expect("streamed provider call");
    let serialized = serde_json::to_string(&released.0).expect("released JSON");
    assert!(!serialized.contains("unit-secret"));
    assert!(serialized.contains("[REDACTED]"));
    assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
    server.await.expect("server task");
}

#[tokio::test]
async fn malformed_tool_arguments_cross_gateway_as_recoverable_failure() {
    let (base_url, server) = one_response_server(json!({
        "id": "response-1",
        "choices": [{"message": {
            "role": "assistant",
            "tool_calls": [{
                "id": "call-1",
                "function": {"name": "lookup", "arguments": "not-json"}
            }]
        }}]
    }))
    .await;
    let profile = ProviderProfile::new(
        "local",
        ProviderKind::OpenAiCompatible,
        Some(base_url),
        None,
        5_000,
    )
    .expect("profile");
    let origin = profile
        .network_origin()
        .expect("origin")
        .expect("network origin");
    let executor = ProviderExecutor::new(profile.clone());
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(origin);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [3_u8; 32],
    );
    let error = gateway
        .execute(provider_request(&profile), &executor)
        .await
        .expect_err("malformed arguments must not be released");
    assert!(matches!(
        error,
        GatewayError::RecoverableExecution { ref code, .. }
            if code == "provider.invalid_tool_arguments"
    ));
    server.await.expect("server task");
    assert!(
        journal
            .read_global(1, 20)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "effect.failed.v1")
    );
}
