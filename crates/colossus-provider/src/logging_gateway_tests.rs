use super::*;
use std::{
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::Instant,
};

const CHILD_CASE: &str = "COLOSSUS_PROVIDER_LOGGING_TEST_CASE";
const CHILD_TEST: &str = "tests::logging_gateway_tests::provider_logging_child";
const PRIVATE_PROFILE: &str =
    "PRIVATE_PROFILE https://user:password@private.test/path?token=secret";
const PRIVATE_RUN: &str = "PRIVATE_RUN https://private.test/run?token=secret";
const PRIVATE_PROMPT: &str = "PRIVATE_PROMPT";

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLogs {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
    }
}

fn isolated_case(case: &str) {
    // Tracing callsite interest is process-global. A single global dispatcher in
    // an isolated process keeps the assertions independent of parallel provider
    // tests, including provider tasks that do not inherit a future's dispatcher.
    let mut child = Command::new(std::env::current_exe().expect("provider test executable"))
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CHILD_CASE, case)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn isolated provider log test");
    let deadline = Instant::now() + Duration::from_secs(20);
    while child
        .try_wait()
        .expect("provider log child status")
        .is_none()
    {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("provider logging child exceeded its shutdown deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("provider log child output");
    assert!(
        output.status.success(),
        "provider logging case {case} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn gateway(profile: &ProviderProfile) -> EffectGateway {
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
                .with_network_destination(profile.network_origin().unwrap().unwrap()),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [77; 32],
    )
}

fn request(profile: &ProviderProfile, run_id: &str) -> EffectRequest {
    let mut request = provider_request(profile);
    let mut model = model_request();
    model.messages[0].content = PRIVATE_PROMPT.into();
    request.content["request"] = serde_json::to_value(model).unwrap();
    request.context.run_id = Some(run_id.into());
    request
}

fn profile(name: &str, url: &str, kind: ProviderKind) -> ProviderProfile {
    ProviderProfile::new(
        name,
        kind,
        Some(url.into()),
        Some("env:UNIT_PROVIDER_KEY".into()),
        5_000,
    )
    .unwrap()
}

fn executor(profile: &ProviderProfile) -> ProviderExecutor {
    ProviderExecutor::with_credentials(profile.clone(), Arc::new(CountingCredentialResolver::new()))
}

fn assert_private_fields_absent(text: &str, url: &str) {
    for private in [
        PRIVATE_PROMPT,
        "PRIVATE_RESPONSE",
        "unit-secret",
        "UNIT_PROVIDER_KEY",
        url,
        "x-api-key",
        "PRIVATE_PROFILE",
        "PRIVATE_RUN",
        "private.test",
    ] {
        assert!(!text.contains(private), "private provider field leaked");
    }
}

fn event_fields(text: &str, event_name: &str) -> Value {
    let matches = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("provider log JSON"))
        .filter_map(|record| {
            let fields = record.get("fields")?;
            (fields["event_name"] == event_name).then(|| fields.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one {event_name}: {text}");
    matches.into_iter().next().unwrap()
}

#[test]
fn native_provider_logs_correlated_lifecycle_without_request_or_response_content() {
    isolated_case("completed");
}

#[test]
fn failed_provider_logs_typed_category_without_untrusted_error_text() {
    isolated_case("failed");
}

#[test]
fn cancelled_provider_logs_interruption_with_unknown_outcome() {
    isolated_case("interrupted");
}

#[tokio::test]
async fn provider_logging_child() {
    let Ok(case) = std::env::var(CHILD_CASE) else {
        return;
    };
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .json()
            .with_env_filter("colossus.provider=debug")
            .with_writer(move || writer.clone())
            .finish(),
    )
    .expect("one process-global provider log subscriber");
    match case.as_str() {
        "completed" => completed(&logs).await,
        "failed" => failed(&logs).await,
        "interrupted" => interrupted(&logs).await,
        _ => panic!("unknown provider logging test case"),
    }
}

async fn completed(logs: &CapturedLogs) {
    for profile_name in ["anthropic-main", PRIVATE_PROFILE] {
        let (url, server) = one_response_server(json!({
            "id":"msg-log-test", "type":"message", "role":"assistant",
            "content":[{"type":"text","text":"PRIVATE_RESPONSE"}],
            "stop_reason":"end_turn", "usage":{"input_tokens":4,"output_tokens":2}
        }))
        .await;
        let profile = profile(profile_name, &url, ProviderKind::AnthropicMessages);
        gateway(&profile)
            .execute(request(&profile, "run-log-test"), &executor(&profile))
            .await
            .unwrap();
        server.await.unwrap();
        let text = logs.take();
        for expected in [
            "provider.started",
            "provider.completed",
            "provider.connected",
            "anthropic_messages",
            crate::logging::safe_identifier(profile_name),
            "run-log-test",
            "elapsed_ms",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        let started = event_fields(&text, "provider.started");
        let finished = event_fields(&text, "provider.completed");
        assert_eq!(started["run_id"], "run-log-test");
        assert_eq!(finished["run_id"], started["run_id"]);
        assert!(!text.contains("provider.interrupted"));
        assert!(!text.contains("provider.failed"));
        assert_private_fields_absent(&text, &url);
    }
}

async fn failed(logs: &CapturedLogs) {
    for streaming in [false, true] {
        let (url, server) = one_status_body_server(
            400,
            "Bad Request",
            r#"{"error":{"code":"context_length_exceeded","message":"PRIVATE_RESPONSE https://private.test/?token=secret"}}"#,
        )
        .await;
        let profile = profile(PRIVATE_PROFILE, &url, ProviderKind::OpenAiCompatible);
        let gateway = gateway(&profile);
        let executor = executor(&profile);
        let effect = request(&profile, PRIVATE_RUN);
        let error = if streaming {
            gateway
                .execute_stream(effect, &executor, &mut ReleasedItems::default())
                .await
        } else {
            gateway.execute(effect, &executor).await
        }
        .expect_err("recognized provider context rejection");
        assert!(matches!(
            error,
            GatewayError::ProviderRejected(colossus_contracts::ProviderFailure {
                reason: colossus_contracts::ProviderFailureReason::ContextLimitExceeded,
                http_status: Some(400),
            })
        ));
        server.await.unwrap();
        let text = logs.take();
        let started = event_fields(&text, "provider.started");
        let failure = event_fields(&text, "provider.failed");
        assert_eq!(started["streaming"], streaming);
        assert_eq!(started["provider_profile"], "redacted");
        assert_eq!(started["run_id"], "redacted");
        assert_eq!(failure["provider_profile"], "redacted");
        assert_eq!(failure["run_id"], "redacted");
        assert_eq!(failure["error_type"], "provider.context_limit_exceeded");
        assert_eq!(failure["http_status"], 400);
        assert_eq!(failure["outcome_unknown"], false);
        assert!(failure["elapsed_ms"].is_u64());
        assert!(!text.contains("provider.completed"));
        assert!(!text.contains("provider.interrupted"));
        assert_private_fields_absent(&text, &url);
    }
}

async fn interrupted(logs: &CapturedLogs) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sent, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut bytes = [0_u8; 4_096];
        while !request
            .windows(PRIVATE_PROMPT.len())
            .any(|part| part == PRIVATE_PROMPT.as_bytes())
        {
            let read = stream.read(&mut bytes).await.unwrap();
            assert_ne!(read, 0, "request must start before interruption");
            request.extend_from_slice(&bytes[..read]);
            assert!(request.len() <= 64 * 1_024, "bounded synthetic request");
        }
        sent.send(()).unwrap();
        // Hold the response pending until the provider future is cancelled.
        while stream.read(&mut bytes).await.unwrap() != 0 {}
    });
    let profile = profile(PRIVATE_PROFILE, &url, ProviderKind::OpenAiCompatible);
    let operation = tokio::spawn(async move {
        gateway(&profile)
            .execute(request(&profile, PRIVATE_RUN), &executor(&profile))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .expect("provider request began")
        .expect("request-start signal");
    operation.abort();
    assert!(
        operation
            .await
            .expect_err("cancelled provider")
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("cancelled provider closed its response transport")
        .unwrap();
    let text = logs.take();
    let started = event_fields(&text, "provider.started");
    let interruption = event_fields(&text, "provider.interrupted");
    assert_eq!(started["provider_profile"], "redacted");
    assert_eq!(interruption["provider_profile"], "redacted");
    assert_eq!(interruption["run_id"], "redacted");
    assert_eq!(interruption["outcome_unknown"], true);
    assert!(!text.contains("provider.completed"));
    assert!(!text.contains("provider.failed"));
    assert_private_fields_absent(&text, &url);
}
