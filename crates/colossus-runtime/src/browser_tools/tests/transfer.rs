use super::*;
use colossus_ports::*;
mod fixture;
pub(super) use fixture::Pending;
const CONTENT: &[u8] = b"Actual owned fixture file content";

#[derive(Debug, Default)]
struct Publisher {
    resolved: AtomicUsize,
    published: AtomicUsize,
}
#[async_trait]
impl BrowserArtifactPublisher for Publisher {
    async fn publish(
        &self,
        _: Arc<dyn EventJournal>,
        _: BrowserArtifactPublication,
    ) -> Result<ModelImageReference, BrowserArtifactError> {
        Err(BrowserArtifactError::Invalid)
    }
    async fn resolve_upload(
        &self,
        _: Arc<dyn EventJournal>,
        request: BrowserUploadArtifactRequest,
    ) -> Result<BrowserUploadArtifact, BrowserArtifactError> {
        assert_eq!(request.binding.application_id, "authenticated-app");
        assert_eq!(request.run_id, "transfer-run");
        self.resolved.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserUploadArtifact {
            descriptor: BrowserUploadDescriptor {
                artifact_id: request.artifact_id,
                size_bytes: CONTENT.len() as u32,
                sha256: hash(CONTENT),
                file_name: "upload.txt".into(),
            },
            bytes: zeroize::Zeroizing::new(CONTENT.to_vec()),
        })
    }
    async fn publish_download(
        &self,
        _: Arc<dyn EventJournal>,
        request: BrowserDownloadPublication,
    ) -> Result<BrowserReleasedArtifact, BrowserArtifactError> {
        assert_eq!(request.bytes.as_slice(), CONTENT);
        assert_eq!(request.binding.application_id, "authenticated-app");
        assert_eq!(request.run_id, "transfer-run");
        self.published.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserReleasedArtifact {
            artifact_id: format!("artifact-{}", "c".repeat(64)),
            file_name: "browser-download.bin".into(),
            media_type: "application/octet-stream".into(),
            size_bytes: CONTENT.len() as u64,
            sha256: hash(CONTENT),
        })
    }
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(<sha2::Sha256 as sha2::Digest>::digest(bytes))
}
fn verified() -> Arc<Driver> {
    let mut driver = Driver::verified();
    driver
        .capabilities
        .actions
        .extend([BrowserActionKind::Upload, BrowserActionKind::Download]);
    Arc::new(driver)
}
async fn reference(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
) -> Value {
    let snapshot = call(
        executor,
        context,
        "browser.snapshot",
        snapshot_target(opened),
    )
    .await
    .unwrap();
    let mut arguments = target(opened);
    arguments["document_id"] = snapshot["tab"]["document_id"].clone();
    arguments["snapshot_id"] = snapshot["snapshot"]["snapshot_id"].clone();
    arguments["element_id"] = snapshot["snapshot"]["nodes"][0]["element"]["element_id"].clone();
    arguments
}

#[tokio::test]
async fn transfer_bytes_cross_gateway_and_only_released_owner_metadata_returns() {
    let workspace = private_tempdir();
    let driver = verified();
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(driver.clone()),
    );
    let publisher = Arc::new(Publisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let context = begin(&runtime, "transfer-run");
    let executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    let mut upload = reference(&executor, &context, &opened).await;
    upload["artifact_id"] = json!(format!("artifact-{}", "a".repeat(64)));
    call(&executor, &context, "browser.upload", upload.clone())
        .await
        .unwrap();
    assert!(
        call(&executor, &context, "browser.upload", upload)
            .await
            .is_err(),
        "mutated input reference cannot replay"
    );
    let download = reference(&executor, &context, &opened).await;
    let released = call(&executor, &context, "browser.download", download)
        .await
        .unwrap();
    assert_eq!(released["artifact"]["size_bytes"], CONTENT.len());
    let public = serde_json::to_string(&released).unwrap();
    assert!(!public.contains("transfer_id") && !public.contains(&BASE64.encode(CONTENT)));
    assert_eq!(driver.transfers.load(Ordering::SeqCst), 2);
    assert_eq!(publisher.resolved.load(Ordering::SeqCst), 2);
    assert_eq!(publisher.published.load(Ordering::SeqCst), 1);
    runtime.drain_browser_sessions().await;
}

struct InspectUpload {
    actual: Arc<AtomicBool>,
    inner: BuiltInPolicy,
}
#[async_trait]
impl PolicyDecisionPoint for InspectUpload {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut decision = self.inner.decide(request).await?;
        if request.action == "browser.upload" && request.phase == EffectPhase::PreEffect {
            let actual = BASE64
                .decode(request.content["content_base64"].as_str().unwrap())
                .unwrap();
            assert_eq!(actual, CONTENT);
            self.actual.store(true, Ordering::SeqCst);
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "actual upload bytes denied".into();
        }
        Ok(decision)
    }
    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.inner.doctor().await
    }
}
#[tokio::test]
async fn pre_policy_inspects_actual_upload_before_any_native_file_effect() {
    let workspace = private_tempdir();
    let driver = verified();
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(driver.clone()),
    );
    runtime.install_browser_artifact_publisher(Arc::new(Publisher::default()));
    let context = begin(&runtime, "transfer-run");
    let mut executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    let mut upload = reference(&executor, &context, &opened).await;
    upload["artifact_id"] = json!(format!("artifact-{}", "a".repeat(64)));
    let actual = Arc::new(AtomicBool::new(false));
    executor.gateway = Arc::new(EffectGateway::new(
        runtime.journal(),
        Arc::new(InspectUpload {
            actual: actual.clone(),
            inner: BuiltInPolicy::offline_default()
                .with_action("browser.upload", DecisionOutcome::Allow)
                .with_network_destination("https://example.org"),
        }),
        Arc::new(DenyApproval),
        SafetyKernel::new(["browser.upload".into()]),
        [8; 32],
    ));
    assert!(matches!(
        call(&executor, &context, "browser.upload", upload).await,
        Err(ToolError::Denied(_))
    ));
    assert!(actual.load(Ordering::SeqCst));
    assert_eq!(driver.transfers.load(Ordering::SeqCst), 0);
    runtime.drain_browser_sessions().await;
}
#[tokio::test]
async fn post_denial_keeps_complete_download_out_of_artifact_storage() {
    let workspace = private_tempdir();
    let driver = verified();
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(driver.clone()),
    );
    let publisher = Arc::new(Publisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let context = begin(&runtime, "transfer-run");
    let mut executor = executor(&runtime);
    let opened = call(&executor, &context, "browser.open", open_arguments())
        .await
        .unwrap();
    let download = reference(&executor, &context, &opened).await;
    executor.gateway = Arc::new(EffectGateway::new(
        runtime.journal(),
        Arc::new(PostDeny(
            BuiltInPolicy::offline_default()
                .with_action("browser.download", DecisionOutcome::Allow)
                .with_network_destination("https://example.org"),
        )),
        Arc::new(DenyApproval),
        SafetyKernel::new(["browser.download".into()]),
        [8; 32],
    ));
    assert!(matches!(
        call(&executor, &context, "browser.download", download).await,
        Err(ToolError::Denied(_))
    ));
    assert_eq!(driver.transfers.load(Ordering::SeqCst), 1);
    assert_eq!(publisher.published.load(Ordering::SeqCst), 0);
    runtime.drain_browser_sessions().await;
}
