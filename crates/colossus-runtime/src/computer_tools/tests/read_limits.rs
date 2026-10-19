use super::fixture::*;
use crate::{
    Runtime, RuntimeOpenOptions, computer_tools::RuntimeComputerHost, prelude::*,
    test_support::private_tempdir,
};
use colossus_contracts::{EffectPhase, PolicyDecision};
use colossus_ports::ComputerCommand;

struct ReadPolicy {
    inner: BuiltInPolicy,
    requests: StdMutex<Vec<(String, Value)>>,
}

#[async_trait]
impl PolicyDecisionPoint for ReadPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        if request.phase == EffectPhase::PreEffect {
            self.requests
                .lock()
                .unwrap()
                .push((request.action.clone(), request.content.clone()));
        }
        self.inner.decide(request).await
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.inner.doctor().await
    }
}

#[tokio::test]
async fn read_counts_are_narrowed_before_policy_and_native_dispatch() {
    let driver = Arc::new(Driver::default());
    let workspace = private_tempdir();
    let mut config = configuration(workspace.path());
    config.computer.max_windows = 2;
    config.computer.max_snapshot_nodes = 4;
    let options = RuntimeOpenOptions::for_workspace(workspace.path())
        .unwrap()
        .with_computer_host(RuntimeComputerHost::new(driver.clone()));
    let runtime =
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
    let context = begin(&runtime, "bounded-reads");
    let mut executor = executor(&runtime);
    let policy = Arc::new(ReadPolicy {
        inner: BuiltInPolicy::offline_default()
            .with_action("computer.windows", DecisionOutcome::Allow)
            .with_action("computer.open", DecisionOutcome::Allow)
            .with_action("computer.snapshot", DecisionOutcome::Allow),
        requests: StdMutex::new(Vec::new()),
    });
    executor.gateway = Arc::new(EffectGateway::new(
        runtime.journal(),
        policy.clone(),
        Arc::new(DenyApproval),
        SafetyKernel::new([
            "computer.windows".into(),
            "computer.open".into(),
            "computer.snapshot".into(),
        ]),
        [7; 32],
    ));
    for (arguments, effective) in [
        (json!({}), 2),
        (json!({"max_windows":1}), 1),
        (json!({"max_windows":128}), 2),
    ] {
        call(&executor, &context, "computer.windows", arguments)
            .await
            .unwrap();
        assert!(matches!(
            driver.requests.lock().unwrap().last().unwrap().command,
            ComputerCommand::Windows { max_windows } if max_windows == effective
        ));
        let requests = policy.requests.lock().unwrap();
        let (action, content) = requests.last().unwrap();
        assert_eq!(action, "computer.windows");
        assert_eq!(content["command"]["max_windows"], effective);
    }
    let opened = call(
        &executor,
        &context,
        "computer.open",
        json!({"window_id":window_id()}),
    )
    .await
    .unwrap();
    for (requested, effective) in [(None, 4), (Some(1), 1), (Some(1024), 4)] {
        let mut arguments = session(&opened);
        if let Some(requested) = requested {
            arguments["max_nodes"] = json!(requested);
        }
        call(&executor, &context, "computer.snapshot", arguments)
            .await
            .unwrap();
        assert!(matches!(
            driver.requests.lock().unwrap().last().unwrap().command,
            ComputerCommand::Snapshot { max_nodes, .. } if max_nodes == effective
        ));
        let requests = policy.requests.lock().unwrap();
        let (action, content) = requests.last().unwrap();
        assert_eq!(action, "computer.snapshot");
        assert_eq!(content["command"]["max_nodes"], effective);
    }
    assert_eq!(driver.inputs.load(std::sync::atomic::Ordering::SeqCst), 0);
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn invalid_read_counts_are_rejected_instead_of_narrowed() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "invalid-read-counts");
    let executor = executor(&runtime);
    for invalid in [json!(0), json!(129), json!("128")] {
        assert!(matches!(
            call(
                &executor,
                &context,
                "computer.windows",
                json!({"max_windows":invalid})
            )
            .await,
            Err(ToolError::InvalidArguments { .. })
        ));
    }
    assert!(driver.requests.lock().unwrap().is_empty());
    let opened = open(&executor, &context).await;
    let before = driver.requests.lock().unwrap().len();
    for invalid in [json!(0), json!(1025), json!("1024")] {
        let mut arguments = session(&opened);
        arguments["max_nodes"] = invalid;
        assert!(matches!(
            call(&executor, &context, "computer.snapshot", arguments).await,
            Err(ToolError::InvalidArguments { .. })
        ));
    }
    assert_eq!(driver.requests.lock().unwrap().len(), before);
    runtime.drain_computer_sessions().await.unwrap();
}
