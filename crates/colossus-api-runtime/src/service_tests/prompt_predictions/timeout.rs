use super::*;

async fn delayed_provider(
    listener: TcpListener,
    started: tokio::sync::oneshot::Sender<()>,
) -> usize {
    let mut notify = Some(started);
    for index in 0..2 {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() < 1_048_576);
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        if index == 1 {
            notify.take().unwrap().send(()).unwrap();
            tokio::time::sleep(Duration::from_secs(14)).await;
        }
        let response = json!({"id":"timeout-fixture","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Completed the requested work."},"finish_reason":"stop"}]}).to_string();
        // The bounded prediction adapter has already closed its socket on timeout.
        let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await;
    }
    2
}

#[tokio::test]
async fn cancelled_prediction_observer_still_settles_timed_out_effect_and_cannot_repeat() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (started, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(20), delayed_provider(listener, started))
            .await
            .unwrap()
    });
    let fixture = fixture(origin, 30_000);
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller("app:prediction-timeout", "prediction-timeout");
    let created = api
        .create_run(&owner, request("timeout-source", "Finish the work."))
        .await
        .unwrap();
    let source = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(source.status, RunStatus::Completed);
    let journal = fixture.runtime.journal();
    let before = journal.head().unwrap().0;
    let observer = {
        let api = api.clone();
        let owner = owner.clone();
        let source_run_id = source.id.clone();
        tokio::spawn(async move {
            api.predict_next_prompt(&owner, PredictNextPromptRequest { source_run_id })
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    observer.abort();
    assert!(observer.await.unwrap_err().is_cancelled());
    let settled = tokio::time::timeout(Duration::from_secs(16), async {
        loop {
            let events = journal.read_global(before + 1, 128).unwrap();
            if let Some(event) = events
                .iter()
                .find(|event| event.event_type == "effect.outcome_unknown.v1")
            {
                let payload = journal.decrypt_payload(event).unwrap();
                assert!(
                    payload
                        .to_string()
                        .contains("auxiliary prompt prediction exceeded its provider deadline")
                );
                let effects = journal.read_stream_from(&event.stream_id, 0, 16).unwrap();
                assert!(
                    effects
                        .iter()
                        .any(|event| event.event_type == "effect.started.v1")
                );
                let receipts = journal
                    .list_stream_ids("runtime.prompt_prediction:", None, 2)
                    .unwrap();
                if receipts.len() == 1
                    && journal.read_stream_from(&receipts[0], 0, 3).unwrap().len() == 2
                {
                    break event.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(settled.context.run_id.as_deref(), Some(source.id.as_str()));
    let head = journal.head().unwrap().0;
    let result = api
        .predict_next_prompt(
            &owner,
            PredictNextPromptRequest {
                source_run_id: source.id,
            },
        )
        .await
        .unwrap();
    assert!(result.text.is_none());
    assert_eq!(
        journal.head().unwrap().0,
        head,
        "replay must not start another effect"
    );
    assert_eq!(server.await.unwrap(), 2);
}
