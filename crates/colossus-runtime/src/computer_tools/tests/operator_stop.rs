use super::fixture::*;
use crate::prelude::*;
use colossus_contracts::{ComputerControlState, ComputerStopReason};
use colossus_ports::{AgentRunLifecycle, ComputerDriverError};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn native_user_stop_cancels_only_owning_run_and_prevents_reopen() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "operator-stop");
    let sibling = begin(&runtime, "still-running");
    let computer = runtime.computer.as_ref().unwrap();
    let run = computer.run(&context).unwrap();
    let sibling_run = computer.run(&sibling).unwrap();
    let executor = executor(&runtime);
    let opened = open(&executor, &context).await;
    let arguments = with(
        observe(&executor, &context, &opened).await,
        json!({"x":1,"y":1}),
    );
    *driver.input_error.lock().unwrap() = Some(ComputerDriverError::UserStopped);
    assert!(matches!(
        call(&executor, &context, "computer.move", arguments.clone()).await,
        Err(ToolError::Failed(message)) if message.contains(&ComputerDriverError::UserStopped.to_string())
    ));
    assert!(run.control.is_cancelled());
    assert!(!sibling_run.control.is_cancelled());
    assert!(driver.revoked.lock().unwrap().contains("operator-stop"));
    assert!(!driver.revoked.lock().unwrap().contains("still-running"));
    assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
    *driver.input_error.lock().unwrap() = None;
    let native_requests = driver.requests.lock().unwrap().len();
    assert!(
        call(&executor, &context, "computer.move", arguments)
            .await
            .is_err()
    );
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
    assert_eq!(driver.requests.lock().unwrap().len(), native_requests);
    computer.finish_run("operator-stop").await;
    assert!(!computer.runs.lock().unwrap().contains_key("operator-stop"));
    assert!(driver.closed.load(Ordering::SeqCst) > 0);
    call(&executor, &sibling, "computer.windows", json!({}))
        .await
        .unwrap();
    runtime.drain_computer_sessions().await.unwrap();
}

#[tokio::test]
async fn released_user_stopped_status_is_committed_before_run_cancellation() {
    let driver = Arc::new(Driver::default());
    let (runtime, _workspace) = runtime(driver.clone());
    let context = begin(&runtime, "stopped-status");
    let computer = runtime.computer.as_ref().unwrap();
    let run = computer.run(&context).unwrap();
    let executor = executor(&runtime);
    open(&executor, &context).await;
    {
        let mut sessions = driver.sessions.lock().unwrap();
        let native = sessions.get_mut("stopped-status").unwrap();
        native.control_generation = 2;
        native.state = ComputerControlState::Stopped;
        native.reason = Some(ComputerStopReason::UserStopped);
    }
    let status = call(&executor, &context, "computer.status", json!({}))
        .await
        .unwrap();
    assert_eq!(status["session"]["state"], "stopped");
    assert_eq!(status["session"]["reason"], "user_stopped");
    assert_eq!(status["session"]["control_generation"], 2);
    assert!(run.control.is_cancelled());
    {
        let state = run.state.lock().unwrap();
        assert!(state.stopped);
        assert_eq!(
            serde_json::to_value(state.session.as_ref().unwrap()).unwrap(),
            status["session"]
        );
    }
    computer.finish_run("stopped-status").await;
    assert!(!runtime.computer_active_work());
}

#[tokio::test]
async fn stale_or_denied_input_does_not_cancel_the_agent_run() {
    for error in [ComputerDriverError::Stale, ComputerDriverError::Denied] {
        let driver = Arc::new(Driver::default());
        let (runtime, _workspace) = runtime(driver.clone());
        let context = begin(&runtime, "recoverable-input");
        let run = runtime.computer.as_ref().unwrap().run(&context).unwrap();
        let executor = executor(&runtime);
        let opened = open(&executor, &context).await;
        let arguments = with(
            observe(&executor, &context, &opened).await,
            json!({"x":1,"y":1}),
        );
        *driver.input_error.lock().unwrap() = Some(error);
        assert!(matches!(
            call(&executor, &context, "computer.move", arguments).await,
            Err(ToolError::Failed(_))
        ));
        assert!(!run.control.is_cancelled());
        assert!(driver.revoked.lock().unwrap().is_empty());
        assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
        *driver.input_error.lock().unwrap() = None;
        let fresh = with(
            observe(&executor, &context, &opened).await,
            json!({"x":1,"y":1}),
        );
        call(&executor, &context, "computer.move", fresh)
            .await
            .unwrap();
        assert_eq!(driver.inputs.load(Ordering::SeqCst), 1);
        runtime.drain_computer_sessions().await.unwrap();
    }
}
