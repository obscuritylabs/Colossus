use super::*;
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
use std::sync::atomic::{AtomicUsize, Ordering};
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
fn provider_profiles_accept_only_valid_environment_or_host_references() {
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
    for reference in ["host:", "host:provider/main", "host:provider:main", "value"] {
        assert!(
            ProviderProfile::new(
                "remote",
                ProviderKind::OpenAiResponses,
                Some("https://api.example.com/v1".into()),
                Some(reference.into()),
                1_000,
            )
            .is_err(),
            "invalid reference was accepted: {reference}"
        );
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
    )
    .expect("model profile");
    assert_eq!(profile.limits.safety_margin_tokens, 1_001);
    assert_eq!(profile.limits.input_budget_tokens, 7_000);
    assert!(!profile.capabilities.tool_calls);
    assert!(!profile.capabilities.streaming);

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
            request: Some(model_request()),
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
            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request).into_owned();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
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
            {"type": "function_call", "call_id": "call-1", "name": "lookup",
             "arguments": "{\"query\":\"rust\"}"}
        ]
    });
    let turn = normalize_responses(
        &profile,
        "unit-profile",
        "unit-model",
        &serde_json::to_vec(&response).expect("JSON"),
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
            if call_id == "call-1" && name == "lookup" && arguments["query"] == "rust"
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
                    name: "lookup".into(),
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
    let responses =
        responses_payload(&request, "unit-model", 4_096, false).expect("Responses payload");
    assert_eq!(responses["model"], "unit-model");
    assert_eq!(responses["max_output_tokens"], 4_096);
    assert_eq!(responses["input"][0]["type"], "function_call");
    assert_eq!(responses["input"][0]["call_id"], "call-1");
    assert_eq!(responses["input"][1]["type"], "function_call_output");
    assert_eq!(responses["input"][1]["call_id"], "call-1");

    let chat = chat_payload(&request, "unit-model", 4_096, false).expect("chat payload");
    assert_eq!(chat["model"], "unit-model");
    assert_eq!(chat["max_tokens"], 4_096);
    assert_eq!(chat["messages"][1]["tool_calls"][0]["id"], "call-1");
    assert_eq!(chat["messages"][2]["tool_call_id"], "call-1");
}

#[test]
fn chat_tool_projection_omits_max_length_but_keeps_the_canonical_schema_strict() {
    let tool = ModelToolDefinition {
        name: "workspace.inspect".into(),
        description: "Inspect bounded workspace paths.".into(),
        input_schema: json!({
            "type": "object",
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

    let chat = chat_payload(&request, "unit-model", 4_096, false).expect("chat payload");
    let projected = &chat["tools"][0]["function"]["parameters"];
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
        tool.input_schema["properties"]["paths"]["items"]["maxLength"],
        4096
    );
    let responses =
        responses_payload(&request, "unit-model", 4_096, false).expect("Responses payload");
    assert_eq!(
        responses["tools"][0]["parameters"]["properties"]["paths"]["items"]["maxLength"],
        4096
    );
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
