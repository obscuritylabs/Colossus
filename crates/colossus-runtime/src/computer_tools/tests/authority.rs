use super::fixture::*;
use crate::{
    Runtime, RuntimeOpenOptions, computer_tools::RuntimeComputerHost, prelude::*,
    test_support::private_tempdir,
};
use colossus_ports::ComputerDriverError;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn computer_input_consumes_only_released_observation_and_cannot_cross_runs_or_workspaces() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "first-run");
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let envelope = observe(&executor, &context, &opened).await;
    // A same-epoch read preserves the observation without renewing its native age.
    call(&executor, &context, "computer.status", json!({}))
        .await
        .unwrap();
    let arguments = with(envelope, json!({"x":10,"y":10}));
    let other = begin(&runtime, "second-run");
    assert!(
        call(&executor, &other, "computer.move", arguments.clone())
            .await
            .is_err()
    );
    let mut foreign = context.clone();
    foreign.session_id = Some("different-conversation".into());
    assert!(
        call(&executor, &foreign, "computer.move", arguments.clone())
            .await
            .is_err()
    );
    call(&executor, &context, "computer.move", arguments.clone())
        .await
        .unwrap();
    assert!(
        call(&executor, &context, "computer.move", arguments)
            .await
            .is_err()
    );
    assert_eq!(driver.inputs.load(Ordering::SeqCst), 1);
    runtime.drain_computer_sessions().await.unwrap();
    assert!(driver.revoked.lock().unwrap().contains("first-run"));
    assert!(!runtime.computer_active_work());
}

#[tokio::test]
async fn paused_native_epoch_cannot_be_bypassed_with_close_open_or_stale_input() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "paused-run");
    let run = runtime.computer.as_ref().unwrap().run(&context).unwrap();
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let envelope = observe(&executor, &context, &opened).await;
    driver.paused.store(true, Ordering::SeqCst);
    let status = call(&executor, &context, "computer.status", json!({}))
        .await
        .unwrap();
    assert_eq!(status["session"]["control_generation"], 2);
    assert!(!run.control.is_cancelled());
    assert!(driver.revoked.lock().unwrap().is_empty());
    assert!(matches!(
        call(
            &executor,
            &context,
            "computer.move",
            with(envelope, json!({"x":1,"y":1}))
        )
        .await,
        Err(ToolError::Failed(message)) if message.contains(&ComputerDriverError::Stale.to_string())
    ));
    assert!(matches!(
        call(&executor, &context, "computer.snapshot", session(&status)).await,
        Err(ToolError::Failed(message)) if message.contains(&ComputerDriverError::Paused.to_string())
    ));
    call(&executor, &context, "computer.close", session(&status))
        .await
        .unwrap();
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
    assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn input_unknown_is_never_retried_and_enumeration_only_helpers_are_cleaned() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "unknown-run");
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let arguments = with(
        observe(&executor, &context, &opened).await,
        json!({"keys":["Enter"]}),
    );
    driver.unknown.store(true, Ordering::SeqCst);
    assert!(matches!(
        call(&executor, &context, "computer.press", arguments.clone()).await,
        Err(ToolError::OutcomeUnknown(_))
    ));
    assert!(
        call(&executor, &context, "computer.press", arguments)
            .await
            .is_err()
    );
    assert_eq!(driver.inputs.load(Ordering::SeqCst), 1);
    let enumeration = begin(&runtime, "enumeration-run");
    call(&executor, &enumeration, "computer.windows", json!({}))
        .await
        .unwrap();
    runtime.drain_computer_sessions().await.unwrap();
    assert!(driver.revoked.lock().unwrap().contains("enumeration-run"));
    assert!(driver.closed.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn cleanup_uncertainty_stays_visible_until_positive_acknowledgement() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "cleanup-run");
    let executor = executor(&runtime);
    call(&executor, &context, "computer.windows", json!({}))
        .await
        .unwrap();
    driver.cleanup_unknown.store(true, Ordering::SeqCst);
    assert!(runtime.drain_computer_sessions().await.is_err());
    assert!(runtime.computer_active_work());
    assert!(driver.revoked.lock().unwrap().contains("cleanup-run"));
    driver.cleanup_unknown.store(false, Ordering::SeqCst);
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn dropping_runtime_revokes_enumeration_owner_synchronously() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "dropped-enumeration");
    let executor = executor(&runtime);
    call(&executor, &context, "computer.windows", json!({}))
        .await
        .unwrap();
    drop(executor);
    drop(runtime);
    assert!(
        driver
            .revoked
            .lock()
            .unwrap()
            .contains("dropped-enumeration")
    );
    tokio::task::yield_now().await;
    assert!(driver.closed.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn expired_or_foreign_observation_is_consumed_without_native_input() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "expired-observation");
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let run = runtime.computer.as_ref().unwrap().run(&context).unwrap();
    for expired in [true, false] {
        let mut envelope = observe(&executor, &context, &opened).await;
        if expired {
            run.state.lock().unwrap().observation.as_mut().unwrap().1 =
                Instant::now() - Duration::from_secs(60);
        } else {
            envelope["observation_id"] = json!(observation_id());
        }
        assert!(matches!(
            call(
                &executor,
                &context,
                "computer.move",
                with(envelope, json!({"x":0,"y":0}))
            ).await,
            Err(ToolError::Failed(message)) if message.contains(&ComputerDriverError::Stale.to_string())
        ));
        assert!(run.state.lock().unwrap().observation.is_none());
        assert!(!run.control.is_cancelled());
        assert!(driver.revoked.lock().unwrap().is_empty());
    }
    assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
    runtime.drain_computer_sessions().await.unwrap();
}

#[test]
fn computer_requires_enabled_host_and_bundled_skill_tracks_same_availability() {
    for (enabled, host) in [(false, true), (true, false), (true, true)] {
        let workspace = private_tempdir();
        let home = private_tempdir();
        let mut config = configuration(workspace.path());
        config.computer.enabled = enabled;
        let mut options = RuntimeOpenOptions::for_workspace(workspace.path())
            .unwrap()
            .with_colossus_home(home.path())
            .unwrap();
        if host {
            options =
                options.with_computer_host(RuntimeComputerHost::new(Arc::new(Driver::default())));
        }
        let runtime =
            Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
        let available = enabled && host;
        assert_eq!(runtime.computer_capabilities().available, available);
        assert_eq!(
            runtime
                .tools
                .list_specs()
                .iter()
                .any(|spec| spec.name == "computer.open"),
            available
        );
        let catalog = runtime.plugin_catalog.capture().unwrap();
        let has = |records: &[AgentPluginRecord]| {
            records
                .iter()
                .flat_map(|record| &record.skills)
                .any(|skill| skill.id == "colossus/computer-use")
        };
        assert_eq!(has(&catalog.records), available);
        assert_eq!(
            has(&runtime
                .plugin_catalog
                .restore(&catalog.digests())
                .unwrap()
                .records),
            available
        );
        assert_eq!(
            runtime
                .plugin_catalog
                .live_inventory()
                .unwrap()
                .iter()
                .flat_map(|entry| &entry.skills)
                .any(|skill| skill.id == "colossus/computer-use"),
            available
        );
    }
}
