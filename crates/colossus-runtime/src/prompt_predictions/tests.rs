//! Auxiliary transport conformance without a live provider or user sign-in.
use super::*;
use colossus_contracts::ProviderContinuation;
use colossus_ports::ProviderContinuationRepository;
use colossus_testkit::InMemoryEventJournal;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[derive(Default)]
struct ForbiddenContinuations(AtomicUsize);

impl ForbiddenContinuations {
    fn unexpected<T>(&self) -> Result<T, StoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(StoreError::Adapter(
            "auxiliary request touched canonical provider state".into(),
        ))
    }
}

impl ProviderContinuationRepository for ForbiddenContinuations {
    fn load(&self, _: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        self.unexpected()
    }
    fn stage(&self, _: ProviderContinuation) -> Result<(), StoreError> {
        self.unexpected()
    }
    fn take_staged(&self, _: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        self.unexpected()
    }
    fn candidate(&self, _: &str, _: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        self.unexpected()
    }
    fn admission_events(
        &self,
        _: &str,
        _: &str,
        _: &ExecutionContext,
    ) -> Result<Vec<NewEvent>, StoreError> {
        self.unexpected()
    }
    fn settlement_events(
        &self,
        _: &ProviderContinuation,
        _: &ExecutionContext,
    ) -> Result<Vec<NewEvent>, StoreError> {
        self.unexpected()
    }
    fn retired_replay(&self, _: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        self.unexpected()
    }
    fn save(&self, _: ProviderContinuation, _: &ExecutionContext) -> Result<(), StoreError> {
        self.unexpected()
    }
    fn clear(&self, _: &str, _: &ExecutionContext) -> Result<(), StoreError> {
        self.unexpected()
    }
}

fn executor(kind: ProviderKind, base: &str, auth_root: &Path) -> ProviderExecutor {
    let codex = kind == ProviderKind::OpenAiCodex;
    let mut profile = ProviderProfile::new(
        "fixture",
        kind,
        (!codex).then(|| base.into()),
        codex.then(|| "codex:default".into()),
        5_000,
    )
    .unwrap();
    if !codex {
        return ProviderExecutor::new(profile);
    }
    // Production forbids custom Codex origins. Only this synthetic adapter gets
    // loopback and disposable, non-secret tokens; no user's auth is consulted.
    profile.base_url = Some(base.into());
    let token = |claims: Value| {
        format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    };
    let bytes = json!({"auth_mode":"chatgpt","tokens":{
        "id_token":token(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"prediction-fixture"}})),
        "access_token":token(json!({"exp":4_102_444_800_i64})),"refresh_token":"fixture-only"
    }}).to_string();
    let path = auth_root.join("auth.json");
    #[cfg(windows)]
    colossus_windows_native::create_private_file(&path, bytes.as_bytes()).unwrap();
    #[cfg(not(windows))]
    {
        fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    ProviderExecutor::new(profile).with_codex_auth_store(CodexAuthStore::at_path(path))
}

async fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 65_536);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

fn response(kind: ProviderKind) -> (&'static str, String) {
    const TEXT: &str = "Review the signed browser certificate path";
    match kind {
        ProviderKind::OpenAiCodex | ProviderKind::OpenAiResponses => {
            let codex = kind == ProviderKind::OpenAiCodex;
            let body = json!({"id":"prediction-response","status":"completed","output":[
                {"type":"compaction","id":"unused-private-state","encrypted_content":"PRIVATE_AUXILIARY_STATE"},
                {"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":TEXT}]}
            ],"usage":{"input_tokens":if codex {31}else{18},"output_tokens":if codex {111}else{7},"total_tokens":if codex {142}else{25}}});
            if codex {
                ("text/event-stream", format!("data: {}\n\ndata: {}\n\n",
                    json!({"type":"response.output_text.delta","delta":TEXT}),
                    json!({"type":"response.completed","response":body})))
            } else { ("application/json", body.to_string()) }
        }
        ProviderKind::OpenAiCompatible => ("application/json", json!({
            "id":"prediction-response","choices":[{"index":0,"message":{"role":"assistant","content":TEXT},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":18,"completion_tokens":7,"total_tokens":25}
        }).to_string()),
        ProviderKind::AnthropicMessages => ("application/json", json!({
            "id":"prediction-response","type":"message","role":"assistant","content":[
                {"type":"thinking","thinking":"PRIVATE_AUXILIARY_STATE","signature":"fixture-signature"},
                {"type":"text","text":TEXT}],"stop_reason":"end_turn","usage":{"input_tokens":18,"output_tokens":7}
        }).to_string()),
        ProviderKind::Echo => unreachable!("Echo has no HTTP transport"),
    }
}

#[tokio::test]
async fn predictions_use_adapter_transport_and_never_touch_canonical_continuations() {
    for (kind, omit_limit) in [
        (ProviderKind::OpenAiCodex, false),
        (ProviderKind::OpenAiResponses, false),
        (ProviderKind::OpenAiCompatible, false),
        (ProviderKind::OpenAiCompatible, true),
        (ProviderKind::AnthropicMessages, false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(15), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                // Codex subscriptions reject stream=false. Do not let a fixture
                // silently accept the regression before returning its SSE body.
                assert_eq!(request["stream"], kind == ProviderKind::OpenAiCodex);
                let (mime, body) = response(kind);
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                request
            }).await.unwrap()
        });
        let auth = crate::test_support::private_tempdir();
        let mut adapter = executor(kind, &format!("{origin}/v1"), auth.path());
        if omit_limit {
            let profile = adapter
                .profile()
                .clone()
                .with_chat_completions_output_token_parameter(
                    ChatCompletionsOutputTokenParameter::Omit,
                )
                .unwrap();
            adapter = ProviderExecutor::new(profile);
        }
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(journal.clone()));
        let continuations = Arc::new(ForbiddenContinuations::default());
        let model = ModelProfile::new(
            "fixture",
            "fixture",
            "fixture-model",
            32_768,
            4096,
            ModelCapabilities {
                tool_calls: false,
                streaming: false,
                image_inputs: false,
                document_inputs: false,
            },
            None,
        )
        .unwrap();
        let providers = Arc::new(
            ProviderRegistry::new(
                vec![adapter],
                vec![model],
                BTreeMap::from([("primary".into(), "fixture".into())]),
            )
            .unwrap()
            .with_continuations(continuations.clone())
            .unwrap(),
        );
        let policy = BuiltInPolicy::offline_default()
            .with_action(kind.generation_action(), DecisionOutcome::Allow)
            .with_network_destination(origin);
        let service = PredictionService {
            gateway: Arc::new(EffectGateway::new(
                journal.clone(),
                Arc::new(policy),
                Arc::new(DenyApproval),
                SafetyKernel::new(["provider.call".into()]),
                [29; 32],
            )),
            providers,
            sessions: sessions.clone(),
            journal,
        };
        let actor = system_actor("prediction-fixture");
        sessions
            .create_session("prediction-session", None, actor.clone())
            .unwrap();
        for (role, text) in [
            (
                ModelMessageRole::User,
                "Inspect the browser certificate path",
            ),
            (
                ModelMessageRole::Assistant,
                "The signed browser fixture is ready for review",
            ),
        ] {
            sessions
                .append_message(
                    "prediction-session",
                    "completed-source",
                    ModelMessage {
                        agent_message_origin: None,
                        role,
                        content: text.into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    actor.clone(),
                )
                .unwrap();
        }
        let before = sessions.list_messages("prediction-session").unwrap();
        let summary_before =
            serde_json::to_value(sessions.get_session("prediction-session").unwrap()).unwrap();
        let prediction = service
            .predict(
                "prediction-session",
                "completed-source",
                "primary",
                "fixture",
                actor.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            prediction.text.as_deref(),
            Some("Review the signed browser certificate path"),
            "{kind:?}"
        );
        assert_eq!(
            prediction.usage.as_ref().unwrap().total_tokens,
            if kind == ProviderKind::OpenAiCodex {
                142
            } else {
                25
            }
        );
        let replay = service
            .predict(
                "prediction-session",
                "completed-source",
                "primary",
                "fixture",
                actor,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            serde_json::to_value(prediction).unwrap()
        );
        assert_eq!(
            continuations.0.load(Ordering::SeqCst),
            0,
            "auxiliary calls must not read, stage, or settle canonical provider state"
        );
        assert_eq!(
            sessions.list_messages("prediction-session").unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(sessions.get_session("prediction-session").unwrap()).unwrap(),
            summary_before
        );
        let wire = server.await.unwrap();
        assert!(wire.get("tools").is_none());
        assert!(wire.get("previous_response_id").is_none());
        assert!(wire.get("context_management").is_none());
        let limit = wire
            .get("max_output_tokens")
            .or_else(|| wire.get("max_tokens"));
        if kind == ProviderKind::OpenAiCodex || omit_limit {
            assert!(limit.is_none());
        } else {
            assert_eq!(limit.and_then(Value::as_u64), Some(96));
        }
    }
}
