use super::*;

/// Cancelling an RPC must not release admission while its native work is queued.
#[tokio::test]
async fn cancelled_blocking_requests_retain_admission_until_work_finishes() {
    let workspace = tempfile::tempdir().unwrap();
    let service = HumanTerminalService::new(workspace.path()).unwrap();
    let (ready_sender, ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let locked_service = service.clone();
    let thread = std::thread::spawn(move || {
        let _sessions = locked_service.0.sessions.lock().unwrap();
        ready_sender.send(()).unwrap();
        let _ = released.recv();
    });
    struct Release {
        signal: Option<mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(signal) = self.signal.take() {
                let _ = signal.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    let release = Release {
        signal: Some(release),
        thread: Some(thread),
    };
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let context = || HumanTerminalRequest {
        owner: "human-a".into(),
        operation: Op::Context,
    };
    let mut requests = Vec::new();
    for _ in 0..8 {
        let service = service.clone();
        let request = context();
        requests.push(tokio::spawn(async move {
            service.execute("app:a", request).await
        }));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.0.controls.available_permits() != 0 {
        assert!(
            Instant::now() < deadline,
            "native work did not acquire admission"
        );
        tokio::task::yield_now().await;
    }
    for request in requests {
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
    }
    let result =
        tokio::time::timeout(Duration::from_secs(1), service.execute("app:a", context())).await;
    // Always release native work before asserting, including on regression.
    drop(release);
    assert!(
        matches!(result, Ok(Err(ref error)) if error.reason == ApiErrorReason::CapacityExceeded),
        "cancelled native work must retain all eight control slots: {result:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.0.controls.available_permits() != 8 {
        assert!(
            Instant::now() < deadline,
            "completed work did not release admission"
        );
        tokio::task::yield_now().await;
    }
    assert!(service.execute("app:a", context()).await.unwrap().available);
}
