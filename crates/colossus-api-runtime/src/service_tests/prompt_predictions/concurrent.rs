//! Reconnected observers share the durable paid result, including its usage.
use super::*;

#[tokio::test]
async fn concurrent_prediction_observers_recover_one_final_receipt_without_another_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(20), async move {
            let mut started = Some(started);
            let mut release = Some(release_rx);
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    assert!(bytes.len() < 1_048_576);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length = headers.lines().find_map(|line|line.strip_prefix("content-length:"))
                            .unwrap().trim().parse::<usize>().unwrap();
                        if bytes.len() >= end + 4 + length { break; }
                    }
                }
                if index == 1 {
                    started.take().unwrap().send(()).unwrap();
                    release.take().unwrap().await.unwrap();
                }
                let text = if index == 0 {"The requested implementation is complete."} else {"Review the certificate fixture"};
                let response = json!({"id":"coalesced-prediction","choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":18,"completion_tokens":7,"total_tokens":25}}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            }
            2
        }).await.unwrap()
    });
    let fixture = fixture(origin, 5_000);
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller("app:prediction-concurrent", "prediction-concurrent");
    let created = api
        .create_run(
            &owner,
            request("concurrent-source", "Finish the certificate fixture."),
        )
        .await
        .unwrap();
    let source = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(source.status, RunStatus::Completed);
    let prediction_request = PredictNextPromptRequest {
        source_run_id: source.id.clone(),
    };
    let observer = |api: Arc<RuntimeAgentRunApi>| {
        let owner = owner.clone();
        let request = prediction_request.clone();
        tokio::spawn(async move { api.predict_next_prompt(&owner, request).await.unwrap() })
    };
    let first = observer(api.clone());
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    let second = observer(service(
        fixture.runtime.clone(),
        RunAdmissionConfig::default(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !second.is_finished(),
        "a pending durable attempt must not race a final receipt with an empty reply"
    );
    release.send(()).unwrap();
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.text.as_deref(),
        Some("Review the certificate fixture")
    );
    assert_eq!(first.usage.as_ref().unwrap().total_tokens, 25);
    let head = fixture.runtime.journal().head().unwrap().0;
    let reconnect = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    assert_eq!(
        reconnect
            .predict_next_prompt(&owner, prediction_request)
            .await
            .unwrap(),
        first
    );
    assert_eq!(fixture.runtime.journal().head().unwrap().0, head);
    assert_eq!(
        server.await.unwrap(),
        2,
        "one source call plus exactly one prediction"
    );
}
