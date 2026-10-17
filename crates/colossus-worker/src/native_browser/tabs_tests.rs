//! Public agent tools select Core-owned tabs across two authenticated presenters.
use super::*;

#[derive(Debug)]
pub(super) struct Plan {
    pub(super) session: BrowserSessionId,
    pub(super) control: u64,
    pub(super) original: BrowserTabId,
}

#[tokio::test]
async fn public_agent_tab_open_select_and_close_retarget_once_and_reject_foreign_views() {
    tokio::time::timeout(Duration::from_secs(15), exercise(true))
        .await
        .unwrap();
}

pub(super) async fn provider(
    listener: tokio::net::TcpListener,
    seen: tokio::sync::oneshot::Sender<()>,
    proceed: tokio::sync::oneshot::Receiver<Plan>,
    completed: tokio::sync::mpsc::Sender<usize>,
    mut permissions: tokio::sync::mpsc::Receiver<()>,
) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = read_request(&mut stream).await;
    seen.send(()).unwrap();
    let plan = proceed.await.unwrap();
    for (phase, operation, extra) in [
        (
            0,
            "browser.tab.open",
            json!({"url":"https://example.org/page"}),
        ),
        (1, "browser.tab.select", json!({"tab_id":plan.original})),
        (2, "browser.tab.close", json!({"tab_id":plan.original})),
    ] {
        let alias = request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .find(|name| *name == operation.replace('.', "_"))
            .expect("the authenticated application offered this tool");
        let mut arguments = json!({"session_id":plan.session,"control_generation":plan.control});
        arguments
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let call_id = format!("tabs-{phase}");
        response(
            &mut stream,
            json!({"tool_calls":[{
                "index":0,"id":call_id,"type":"function",
                "function":{"name":alias,"arguments":arguments.to_string()}
            }]}),
            "tool_calls",
        )
        .await;
        drop(stream);
        let (mut next, _) = listener.accept().await.unwrap();
        request = read_request(&mut next).await;
        let result = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
            .expect("public agent run must return the actual browser tool result");
        let result: Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
        assert!(
            result.get("tab").is_some(),
            "the authorized tab action must succeed: {result}"
        );
        completed.send(phase).await.unwrap();
        permissions.recv().await.unwrap();
        stream = next;
    }
    response(&mut stream, json!({"content":"done"}), "stop").await;
}

pub(super) async fn retarget(
    client: &PresentationClient,
    session: &colossus_runtime::RuntimeNativeBrowserSession,
    driver: &Driver,
    completions: &mut tokio::sync::mpsc::Receiver<usize>,
    permit: tokio::sync::mpsc::Sender<()>,
    human: Lease,
    human_ciphertext: Vec<u8>,
) {
    let mut lease = session.presentation().observe(human).await.err();
    assert!(
        lease.take().is_some(),
        "the old human epoch is permanently stale"
    );
    let mut prior = driver.0.data.lock().unwrap().lease.unwrap();
    for expected in 0..3 {
        assert_eq!(completions.recv().await, Some(expected));
        let unselected = session.configure().unwrap().target;
        let selected = Host::page(&driver.0.data.lock().unwrap()).target;
        *driver.0.claimed_target.lock().unwrap() = Some(unselected.clone());
        assert_eq!(
            client.observe(prior).await.err(),
            Some(PresentationError::Stale),
            "an owned but unselected or closed tab cannot nominate the viewer target"
        );
        driver.0.stale_observation.store(true, Ordering::SeqCst);
        assert_eq!(
            client.observe(prior).await.err(),
            Some(PresentationError::Stale)
        );
        let page = client.observe(prior).await.unwrap();
        assert_eq!(page.target, selected);
        let mut configure = session.configure().unwrap();
        assert_eq!(configure.target, selected);
        configure.viewport_generation += 1;
        let count = driver.0.configurations.load(Ordering::SeqCst);
        let mut wrong = configure.clone();
        wrong.target = unselected;
        assert_eq!(
            client.configure(wrong).await.err(),
            Some(PresentationError::Stale)
        );
        let mut foreign = configure.clone();
        foreign.target.tab_id = BrowserTabId::parse(format!("bt_{}", "a".repeat(32))).unwrap();
        assert_eq!(
            client.configure(foreign).await.err(),
            Some(PresentationError::Stale)
        );
        assert_eq!(driver.0.configurations.load(Ordering::SeqCst), count);
        let next = client.configure(configure.clone()).await.unwrap();
        assert_ne!(next.tab, prior.tab);
        if matches!(expected, 0 | 2) {
            assert!(
                next.document_generation < prior.document_generation,
                "a different exact Core-owned tab has its independent document generation"
            );
        }
        assert_eq!(next.control_generation, prior.control_generation);
        assert!(next.viewport_generation > prior.viewport_generation);
        assert_eq!(
            client.configure(configure).await.err(),
            Some(PresentationError::Stale)
        );
        assert_eq!(driver.0.configurations.load(Ordering::SeqCst), count + 1);
        assert_eq!(
            client.next_frame(prior).await.err(),
            Some(PresentationError::Stale)
        );
        assert!(
            client
                .command(PresentationCommand::Input {
                    lease: human,
                    input: Input::Key {
                        code: 0,
                        pressed: true
                    },
                    modifiers: 0,
                })
                .await
                .is_err()
        );
        let mut codec =
            FrameCodec::new(client.surface_key(), session.enrollment_digest(), next).unwrap();
        assert!(codec.decode(human_ciphertext.clone()).is_err());
        let frame = codec
            .decode(client.next_frame(next).await.unwrap().unwrap())
            .unwrap();
        assert_eq!(frame.lease, next);
        prior = next;
        permit.send(()).await.unwrap();
    }
    assert_eq!(driver.0.data.lock().unwrap().tabs.len(), 1);
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 1_048_576);
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let size: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + size {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + size]).unwrap();
            }
        }
    }
}
async fn response(stream: &mut tokio::net::TcpStream, delta: Value, finish: &str) {
    let body = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id":"fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]
        })
    );
    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}
