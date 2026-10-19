use super::fixture::*;
use crate::prelude::*;
use colossus_contracts::{
    ComputerLimits, EffectPhase, ImageInputBudget, ModelImageDetail, PolicyDecision,
};
use colossus_ports::{
    ComputerArtifactError, ComputerArtifactPublication, ComputerArtifactPublisher, ToolExecutor,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Default)]
struct Publisher(AtomicUsize);
#[async_trait]
impl ComputerArtifactPublisher for Publisher {
    async fn publish(
        &self,
        _: Arc<dyn EventJournal>,
        request: ComputerArtifactPublication,
    ) -> Result<ModelImageReference, ComputerArtifactError> {
        assert_eq!(request.bytes.as_slice(), BASE64.decode(PNG).unwrap());
        assert_eq!(request.binding.application_id, "app:fixture");
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ModelImageReference {
            artifact_id: format!("artifact-{}", "a".repeat(64)),
            file_name: "computer-screenshot.png".into(),
            media_type: "image/png".into(),
            size_bytes: u64::from(request.descriptor.size_bytes),
            sha256: request.descriptor.sha256,
            width_pixels: 1,
            height_pixels: 1,
            detail: ModelImageDetail::Auto,
        })
    }
}

struct PostDeny(BuiltInPolicy);
#[async_trait]
impl PolicyDecisionPoint for PostDeny {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
        }
        Ok(decision)
    }
    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.0.doctor().await
    }
}

#[tokio::test]
async fn computer_png_is_validated_quarantined_published_inline_and_observation_released_last() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver);
    let publisher = Arc::new(Publisher::default());
    runtime.install_computer_artifact_publisher(publisher.clone());
    let context = begin(&runtime, "capture-run");
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let result = executor
        .execute(
            ToolCall {
                call_id: "capture-call".into(),
                name: "computer.screenshot".into(),
                arguments: session(&opened),
            },
            context.clone(),
        )
        .await
        .unwrap();
    let metadata: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(result.images.len(), 1);
    assert_eq!(
        metadata["artifact"]["artifact_id"],
        result.images[0].artifact_id
    );
    assert!(!result.output.contains(PNG));
    assert!(result.output.len() < 4096);
    assert_eq!(publisher.0.load(Ordering::SeqCst), 1);
    let arguments = json!({"session_id":metadata["session_id"],"control_generation":metadata["control_generation"],
        "observation_id":metadata["observation_id"],"x":0,"y":0});
    call(&executor, &context, "computer.move", arguments)
        .await
        .unwrap();
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn denied_capture_or_hash_mismatch_never_publishes_pixels_or_input_authority() {
    for mismatch in [false, true] {
        let driver = Arc::new(Driver::default());
        let (runtime, _workspace) = runtime(driver.clone());
        let publisher = Arc::new(Publisher::default());
        runtime.install_computer_artifact_publisher(publisher.clone());
        let context = begin(&runtime, "denied-capture");
        let mut executor = executor(&runtime);
        let opened = open(&executor, &context).await;
        driver.bad_png.store(mismatch, Ordering::SeqCst);
        if !mismatch {
            executor.gateway = Arc::new(EffectGateway::new(
                runtime.journal(),
                Arc::new(PostDeny(
                    BuiltInPolicy::offline_default()
                        .with_action("computer.screenshot", DecisionOutcome::Allow),
                )),
                Arc::new(DenyApproval),
                SafetyKernel::new(["computer.screenshot".into()]),
                [9; 32],
            ));
        }
        assert!(
            call(&executor, &context, "computer.screenshot", session(&opened))
                .await
                .is_err()
        );
        assert_eq!(publisher.0.load(Ordering::SeqCst), 0);
        assert!(driver.revoked.lock().unwrap().contains("denied-capture"));
        runtime.drain_computer_sessions().await.unwrap();
    }
}

#[tokio::test]
async fn capture_count_budget_is_checked_before_native_dispatch() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    runtime.install_computer_artifact_publisher(Arc::new(Publisher::default()));
    let mut context = begin(&runtime, "budget-run");
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    context.image_input_budget = Some(ImageInputBudget {
        remaining_count: 0,
        remaining_bytes: u64::from(ComputerLimits::default().max_screenshot_bytes),
    });
    let before = driver.requests.lock().unwrap().len();
    assert!(
        call(&executor, &context, "computer.screenshot", session(&opened))
            .await
            .is_err()
    );
    assert_eq!(driver.requests.lock().unwrap().len(), before);
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn capture_byte_budget_narrows_native_request_and_accepts_small_png() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    runtime.install_computer_artifact_publisher(Arc::new(Publisher::default()));
    let mut context = begin(&runtime, "small-budget");
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let bytes = BASE64.decode(PNG).unwrap().len() as u64;
    context.image_input_budget = Some(ImageInputBudget {
        remaining_count: 1,
        remaining_bytes: bytes,
    });
    call(&executor, &context, "computer.screenshot", session(&opened))
        .await
        .unwrap();
    assert!(
        matches!(driver.requests.lock().unwrap().last().unwrap().command,
        colossus_ports::ComputerCommand::Screenshot { max_bytes,.. } if u64::from(max_bytes) == bytes)
    );
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn post_denied_window_list_does_not_release_a_native_choice() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "denied-windows");
    let mut executor = executor(&runtime);
    executor.gateway = Arc::new(EffectGateway::new(
        runtime.journal(),
        Arc::new(PostDeny(
            BuiltInPolicy::offline_default()
                .with_action("computer.windows", DecisionOutcome::Allow),
        )),
        Arc::new(DenyApproval),
        SafetyKernel::new(["computer.windows".into()]),
        [9; 32],
    ));
    assert!(
        call(&executor, &context, "computer.windows", json!({}))
            .await
            .is_err()
    );
    assert!(driver.revoked.lock().unwrap().contains("denied-windows"));
    assert!(
        call(
            &executor,
            &context,
            "computer.open",
            json!({"window_id":window_id()})
        )
        .await
        .is_err()
    );
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn screenshot_of_new_owned_modal_requires_private_root_proof_and_policy_release() {
    for (foreign_root, deny_release) in [(false, false), (true, false), (false, true)] {
        let driver = Arc::new(Driver::default());
        let (runtime, _workspace) = runtime(driver.clone());
        let publisher = Arc::new(Publisher::default());
        runtime.install_computer_artifact_publisher(publisher.clone());
        let context = begin(&runtime, "modal-capture");
        let mut executor = executor(&runtime);
        let opened = open(&executor, &context).await;
        let envelope = observe(&executor, &context, &opened).await;
        driver.open_modal_on_input.store(true, Ordering::SeqCst);
        call(
            &executor,
            &context,
            "computer.click",
            with(
                envelope,
                json!({
            "target":{"kind":"point","x":1,"y":1},"button":"left","count":1}),
            ),
        )
        .await
        .unwrap();
        driver.foreign_root.store(foreign_root, Ordering::SeqCst);
        let run = runtime.computer.as_ref().unwrap().run(&context).unwrap();
        let modal = colossus_contracts::ComputerWindowId::parse(format!("cu_w_{}", "b".repeat(32)))
            .unwrap();
        assert!(!run.state.lock().unwrap().observed_windows.contains(&modal));
        if deny_release {
            executor.gateway = Arc::new(EffectGateway::new(
                runtime.journal(),
                Arc::new(PostDeny(
                    BuiltInPolicy::offline_default()
                        .with_action("computer.screenshot", DecisionOutcome::Allow),
                )),
                Arc::new(DenyApproval),
                SafetyKernel::new(["computer.screenshot".into()]),
                [9; 32],
            ));
        }
        let result = call(&executor, &context, "computer.screenshot", session(&opened)).await;
        let released = !foreign_root && !deny_release;
        assert_eq!(result.is_ok(), released);
        assert_eq!(publisher.0.load(Ordering::SeqCst), usize::from(released));
        assert_eq!(
            run.state.lock().unwrap().observed_windows.contains(&modal),
            released
        );
        if let Ok(capture) = result {
            assert_eq!(capture["window_id"], modal.as_str());
            let envelope = json!({"session_id":capture["session_id"],"control_generation":capture["control_generation"],
                "observation_id":capture["observation_id"],"window_id":modal});
            call(&executor, &context, "computer.focus", envelope)
                .await
                .unwrap();
        }
        runtime.drain_computer_sessions().await.unwrap();
    }
}
