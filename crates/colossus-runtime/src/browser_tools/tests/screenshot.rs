use super::*;
use colossus_ports::{
    BrowserArtifactError, BrowserArtifactPublication, BrowserArtifactPublisher, EventJournal,
};

pub(super) const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQMgkDAAD4AJ3MaiF4AAAAAElFTkSuQmCC";

#[derive(Debug, Default)]
struct Publisher {
    published: AtomicUsize,
}

#[async_trait]
impl BrowserArtifactPublisher for Publisher {
    async fn publish(
        &self,
        _: Arc<dyn EventJournal>,
        request: BrowserArtifactPublication,
    ) -> Result<ModelImageReference, BrowserArtifactError> {
        assert_eq!(request.bytes.as_slice(), BASE64.decode(PNG).unwrap());
        assert_eq!(request.binding.application_id, "authenticated-app");
        assert_eq!(request.run_id, "screenshot-run");
        self.published.fetch_add(1, Ordering::SeqCst);
        Ok(ModelImageReference {
            artifact_id: format!("artifact-{}", "a".repeat(64)),
            file_name: "browser-screenshot.png".into(),
            media_type: "image/png".into(),
            size_bytes: request.bytes.len() as u64,
            sha256: request.descriptor.sha256,
            width_pixels: 1,
            height_pixels: 1,
            detail: ModelImageDetail::Auto,
        })
    }
}

#[tokio::test]
async fn actual_png_is_quarantined_then_published_and_only_metadata_is_returned() {
    let workspace = private_tempdir();
    let mut fixture = Driver::verified();
    fixture
        .capabilities
        .actions
        .push(BrowserActionKind::Screenshot);
    let driver = Arc::new(fixture);
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let publisher = Arc::new(Publisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let context = begin(&runtime, "screenshot-run");
    let executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    let result = executor
        .execute(
            ToolCall {
                call_id: "capture-call".into(),
                name: "browser.screenshot".into(),
                arguments: target(&opened),
            },
            context,
        )
        .await
        .unwrap();
    let metadata: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(result.images.len(), 1);
    assert_eq!(
        metadata["artifact"]["artifact_id"],
        result.images[0].artifact_id
    );
    assert!(result.output.len() < 4096);
    assert!(!result.output.contains(PNG));
    assert!(!result.output.contains("transfer_id"));
    assert_eq!(driver.captured.load(Ordering::SeqCst), 1);
    assert_eq!(publisher.published.load(Ordering::SeqCst), 1);
    runtime.drain_browser_sessions().await;
}

#[tokio::test]
async fn screenshot_post_denial_never_publishes_actual_png() {
    let workspace = private_tempdir();
    let mut fixture = Driver::verified();
    fixture
        .capabilities
        .actions
        .push(BrowserActionKind::Screenshot);
    let driver = Arc::new(fixture);
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let publisher = Arc::new(Publisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let context = begin(&runtime, "screenshot-run");
    let mut executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    executor.gateway = Arc::new(EffectGateway::new(
        runtime.journal(),
        Arc::new(PostDeny(
            BuiltInPolicy::offline_default()
                .with_action("browser.screenshot", DecisionOutcome::Allow)
                .with_network_destination("https://example.org"),
        )),
        Arc::new(DenyApproval),
        SafetyKernel::new(["browser.screenshot".into()]),
        [9; 32],
    ));
    let error = call(&executor, &context, "browser.screenshot", target(&opened))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Denied(_)));
    assert!(!error.to_string().contains(PNG));
    assert_eq!(driver.captured.load(Ordering::SeqCst), 1);
    assert_eq!(publisher.published.load(Ordering::SeqCst), 0);
    runtime.drain_browser_sessions().await;
}

#[tokio::test]
async fn stale_document_and_absent_publisher_reject_capture_before_native_dispatch() {
    let workspace = private_tempdir();
    let mut fixture = Driver::verified();
    fixture
        .capabilities
        .actions
        .push(BrowserActionKind::Screenshot);
    let driver = Arc::new(fixture);
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(Arc::clone(&driver)),
    );
    let context = begin(&runtime, "screenshot-run");
    let executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    assert!(
        call(&executor, &context, "browser.screenshot", target(&opened))
            .await
            .is_err()
    );
    runtime.install_browser_artifact_publisher(Arc::new(Publisher::default()));
    let mut stale = target(&opened);
    stale["document_id"] = json!(format!("bd_{}", "e".repeat(32)));
    assert!(
        call(&executor, &context, "browser.screenshot", stale)
            .await
            .is_err()
    );
    assert_eq!(driver.captured.load(Ordering::SeqCst), 0);
    runtime.drain_browser_sessions().await;
}
