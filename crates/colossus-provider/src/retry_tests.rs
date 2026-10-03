use super::*;
use std::sync::Mutex;

enum Reply {
    Status(u16, Option<&'static str>),
    Json(Value),
    Stream(&'static str),
    Disconnect,
}

struct RecordedRequest {
    received: tokio::time::Instant,
    body: Value,
}

struct SequenceServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

async fn sequence_server(replies: Vec<Reply>) -> SequenceServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        for reply in replies {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(70), listener.accept())
                .await
                .expect("request timeout")
                .expect("accept");
            let mut request = Vec::new();
            let mut scratch = [0_u8; 4096];
            let body = loop {
                let read = stream.read(&mut scratch).await.expect("read request");
                assert_ne!(read, 0, "request ended before its body");
                request.extend_from_slice(&scratch[..read]);
                assert!(request.len() <= MAX_PROVIDER_REQUEST_BYTES);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
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
                    break serde_json::from_slice(&request[header_end + 4..])
                        .expect("request JSON");
                }
            };
            recorded.lock().expect("requests").push(RecordedRequest {
                received: tokio::time::Instant::now(),
                body,
            });
            let (status, content_type, retry_after, body) = match reply {
                Reply::Status(status, retry_after) => (
                    status,
                    "application/json",
                    retry_after,
                    format!(r#"{{"error":"upstream HTTP {status}"}}"#),
                ),
                Reply::Json(body) => (200, "application/json", None, body.to_string()),
                Reply::Stream(body) => (200, "text/event-stream", None, body.into()),
                Reply::Disconnect => continue,
            };
            let retry_after = retry_after
                .map(|value| format!("retry-after: {value}\r\n"))
                .unwrap_or_default();
            let headers = format!(
                "HTTP/1.1 {status} Response\r\ncontent-type: {content_type}\r\n{retry_after}content-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).await.expect("headers");
            stream.write_all(body.as_bytes()).await.expect("body");
        }
    });
    SequenceServer {
        base_url: format!("http://{address}/v1"),
        requests,
        task,
    }
}

fn retry_gateway(profile: &ProviderProfile) -> EffectGateway {
    let origin = profile.network_origin().expect("origin").expect("network");
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
                .with_network_destination(origin),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [24_u8; 32],
    )
}

fn retry_profile(base_url: String, kind: ProviderKind) -> ProviderProfile {
    ProviderProfile::new("retry-provider", kind, Some(base_url), None, 60_000).expect("profile")
}

#[test]
fn retry_policy_is_bounded_and_respects_server_backoff() {
    for status in 502..=504 {
        let delays = (0..5)
            .map(|attempt| super::super::retry::retry_delay(status, attempt, None).expect("retry"))
            .collect::<Vec<_>>();
        assert_eq!(delays, [1, 2, 4, 8, 16].map(Duration::from_secs));
        assert!(super::super::retry::retry_delay(status, 5, None).is_none());
    }
    for status in [400, 401, 403, 408, 429, 500] {
        assert!(super::super::retry::retry_delay(status, 0, None).is_none());
    }
    assert_eq!(
        super::super::retry::retry_delay(503, 0, Some(7_000)),
        Some(Duration::from_secs(7))
    );
    assert_eq!(
        super::super::retry::retry_delay(503, 2, Some(1_000)),
        Some(Duration::from_secs(4))
    );
}

#[tokio::test]
async fn streaming_generation_retries_mixed_gateway_errors_with_incrementing_backoff() {
    let server = sequence_server(vec![
        Reply::Status(502, None),
        Reply::Status(503, None),
        Reply::Status(504, None),
        Reply::Stream(
            "data: {\"id\":\"chat-retry\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"retry-ready\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        ),
    ])
    .await;
    let profile = retry_profile(server.base_url, ProviderKind::OpenAiCompatible);
    let executor = ProviderExecutor::new(profile.clone());
    let mut released = ReleasedItems::default();
    retry_gateway(&profile)
        .execute_stream(provider_request(&profile), &executor, &mut released)
        .await
        .expect("recovered stream");
    server.task.await.expect("server");
    let requests = server.requests.lock().expect("requests");
    assert_eq!(requests.len(), 4);
    for (pair, seconds) in requests.windows(2).zip([1, 2, 4]) {
        assert!(pair[1].received.duration_since(pair[0].received) >= Duration::from_secs(seconds));
        assert_eq!(pair[0].body, pair[1].body);
    }
    let output = released
        .0
        .iter()
        .filter_map(|item| match item {
            ProviderStreamItem::Event {
                event: ProviderEvent::ModelDelta { text },
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(output, "retry-ready");
    let progress = released
        .0
        .iter()
        .filter_map(|item| match item {
            ProviderStreamItem::Retry { retry } => Some(retry),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(progress.len(), 7);
    for (pair, attempt) in progress[..6].chunks(2).zip(1..=3) {
        assert_eq!(pair[0].attempt, attempt);
        assert_eq!(
            pair[0].state,
            colossus_contracts::ProviderRetryState::Backoff
        );
        assert!(pair[0].retry_at.is_some());
        assert_eq!(
            pair[1].state,
            colossus_contracts::ProviderRetryState::Retrying
        );
        assert!(pair[1].retry_at.is_none());
    }
    assert_eq!(
        progress[6].state,
        colossus_contracts::ProviderRetryState::Recovered
    );
}

#[tokio::test]
async fn non_streaming_generation_retries_both_provider_protocols() {
    for (kind, response) in [
        (
            ProviderKind::OpenAiCompatible,
            json!({"id": "chat-retry", "choices": [{"message": {"role": "assistant", "content": "retry-ready"}}]}),
        ),
        (
            ProviderKind::OpenAiResponses,
            json!({"id": "response-retry", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "retry-ready"}]}]}),
        ),
    ] {
        let server =
            sequence_server(vec![Reply::Status(502, Some("2")), Reply::Json(response)]).await;
        let profile = retry_profile(server.base_url, kind);
        let executor = ProviderExecutor::new(profile.clone());
        let result = retry_gateway(&profile)
            .execute(provider_request(&profile), &executor)
            .await
            .expect("recovered generation");
        let turn: ProviderTurn = serde_json::from_slice(&result.bytes).expect("turn");
        assert!(turn.events.iter().any(|event| matches!(
            event, ProviderEvent::FinalOutput { text } if text == "retry-ready"
        )));
        server.task.await.expect("server");
        let requests = server.requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body, requests[1].body);
        assert!(requests[1].received - requests[0].received >= Duration::from_secs(2));
    }
}

#[tokio::test]
async fn exhausted_retries_preserve_the_last_recoverable_http_status() {
    let server = sequence_server(
        [502, 503, 504, 502, 503, 504]
            .map(|status| Reply::Status(status, Some("0")))
            .into(),
    )
    .await;
    let profile = retry_profile(server.base_url, ProviderKind::OpenAiCompatible);
    let executor = ProviderExecutor::new(profile.clone());
    let error = retry_gateway(&profile)
        .execute(provider_request(&profile), &executor)
        .await
        .expect_err("retry limit");
    assert!(matches!(
        error,
        GatewayError::RecoverableExecution {
            http_status: Some(504),
            retry_after_ms: Some(0),
            ..
        }
    ));
    server.task.await.expect("server");
    assert_eq!(server.requests.lock().expect("requests").len(), 6);
}

#[tokio::test]
async fn non_retryable_status_and_uncertain_transport_failures_are_not_replayed() {
    for (reply, uncertain) in [(Reply::Status(401, None), false), (Reply::Disconnect, true)] {
        let server = sequence_server(vec![reply]).await;
        let profile = retry_profile(server.base_url, ProviderKind::OpenAiCompatible);
        let executor = ProviderExecutor::new(profile.clone());
        let error = retry_gateway(&profile)
            .execute(provider_request(&profile), &executor)
            .await
            .expect_err("terminal error");
        if uncertain {
            assert!(matches!(error, GatewayError::OutcomeUnknown(_)));
        } else {
            assert!(matches!(
                error,
                GatewayError::HttpStatus { status: 401, .. }
            ));
        }
        server.task.await.expect("server");
        assert_eq!(server.requests.lock().expect("requests").len(), 1);
    }
}

#[tokio::test]
async fn retry_backoff_stops_at_the_existing_generation_deadline() {
    let server = sequence_server(vec![Reply::Status(503, None)]).await;
    let profile = ProviderProfile::new(
        "short-deadline",
        ProviderKind::OpenAiCompatible,
        Some(server.base_url),
        None,
        200,
    )
    .expect("profile");
    let executor = ProviderExecutor::new(profile.clone());
    let mut released = ReleasedItems::default();
    let error = retry_gateway(&profile)
        .execute_stream(provider_request(&profile), &executor, &mut released)
        .await
        .expect_err("backoff cannot fit");
    assert!(matches!(
        error,
        GatewayError::RecoverableExecution {
            http_status: Some(503),
            ..
        }
    ));
    server.task.await.expect("server");
    assert_eq!(server.requests.lock().expect("requests").len(), 1);
    assert!(released.0.is_empty());
}

#[tokio::test]
async fn cancellation_during_backoff_never_sends_the_next_request() {
    let server = sequence_server(vec![Reply::Status(502, None), Reply::Status(502, None)]).await;
    let requests = Arc::clone(&server.requests);
    let profile = retry_profile(server.base_url, ProviderKind::OpenAiCompatible);
    let operation = tokio::spawn(async move {
        retry_gateway(&profile)
            .execute(
                provider_request(&profile),
                &ProviderExecutor::new(profile.clone()),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while requests.lock().expect("requests").is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first request");
    tokio::time::sleep(Duration::from_millis(50)).await;
    operation.abort();
    assert!(operation.await.expect_err("cancelled").is_cancelled());
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert_eq!(requests.lock().expect("requests").len(), 1);
    server.task.abort();
}

#[tokio::test]
async fn non_streaming_routes_release_live_recovery_without_requesting_sse() {
    for (kind, response) in [
        (
            ProviderKind::OpenAiCompatible,
            json!({"choices": [{"message": {"role": "assistant", "content": "json-ready"}}]}),
        ),
        (
            ProviderKind::OpenAiResponses,
            json!({"output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "json-ready"}]}]}),
        ),
    ] {
        let server = sequence_server(vec![Reply::Status(503, None), Reply::Json(response)]).await;
        let profile = retry_profile(server.base_url, kind);
        let mut effect = provider_request(&profile);
        effect.content["stream_response"] = json!(false);
        let mut released = ReleasedItems::default();
        retry_gateway(&profile)
            .execute_stream(effect, &ProviderExecutor::new(profile), &mut released)
            .await
            .expect("recovered non-streaming route");
        server.task.await.expect("server");
        let requests = server.requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body["stream"], false);
        assert_eq!(requests[1].body["stream"], false);
        assert!(
            matches!(&released.0[0], ProviderStreamItem::Retry { retry } if retry.state == colossus_contracts::ProviderRetryState::Backoff)
        );
        assert!(released.0.iter().any(|item| matches!(item, ProviderStreamItem::Event { event: ProviderEvent::FinalOutput { text } } if text == "json-ready")));
        assert!(matches!(
            released.0.last(),
            Some(ProviderStreamItem::Completed { .. })
        ));
    }
}
