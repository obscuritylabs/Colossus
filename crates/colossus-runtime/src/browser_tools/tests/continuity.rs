use super::*;

#[tokio::test]
async fn next_turn_requires_a_new_open_permit_before_resuming_the_same_browser() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(driver.clone()),
    );
    let first = begin(&runtime, "first");
    let executor = executor(&runtime);
    let opened = call(&executor, &first, "browser.open", open_arguments())
        .await
        .unwrap();
    runtime
        .browser
        .as_ref()
        .unwrap()
        .complete_run("first")
        .await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 0);
    assert!(
        call(
            &executor,
            &first,
            "browser.snapshot",
            snapshot_target(&opened)
        )
        .await
        .is_err()
    );
    let second = begin(&runtime, "second");
    assert!(
        call(
            &executor,
            &second,
            "browser.snapshot",
            snapshot_target(&opened)
        )
        .await
        .is_err()
    );
    let mut unoffered = second.clone();
    unoffered
        .offered_tools
        .retain(|name| name != "browser.open");
    assert!(
        call(&executor, &unoffered, "browser.open", open_arguments())
            .await
            .is_err()
    );
    assert_eq!(driver.resumed.load(Ordering::SeqCst), 0);
    let resumed = call(&executor, &second, "browser.open", open_arguments())
        .await
        .unwrap();
    assert_eq!(
        resumed["session"]["session_id"],
        opened["session"]["session_id"]
    );
    assert!(
        resumed["control_generation"].as_u64().unwrap()
            > opened["control_generation"].as_u64().unwrap()
    );
    assert_eq!(driver.opened.load(Ordering::SeqCst), 1);
    assert_eq!(driver.resumed.load(Ordering::SeqCst), 1);
    call(
        &executor,
        &second,
        "browser.snapshot",
        snapshot_target(&resumed),
    )
    .await
    .unwrap();
    runtime.browser.as_ref().unwrap().finish_run("second").await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shared_cancellation_during_suspension_keeps_terminal_cleanup_ownership() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let barrier = Arc::new(tokio::sync::Notify::new());
    *driver.suspend_barrier.lock().unwrap() = Some(barrier.clone());
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(driver.clone()),
    );
    let first = begin(&runtime, "first");
    call(
        &executor(&runtime),
        &first,
        "browser.open",
        open_arguments(),
    )
    .await
    .unwrap();
    let browser = runtime.browser.as_ref().unwrap().clone();
    let owned = browser.run(&first).unwrap();
    let binding = owned.actor.binding;
    let control = owned.control;
    let finishing = {
        let browser = browser.clone();
        tokio::spawn(async move { browser.complete_run("first").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while driver.suspend_started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // API cancellation uses this shared signal without calling lifecycle.cancel_run.
    control.cancel();
    barrier.notify_one();
    finishing.await.unwrap();
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
    assert!(!browser.runs.lock().unwrap().contains_key("first"));
    assert!(browser.coordinator.list(&binding).unwrap().is_empty());
}

#[tokio::test]
async fn shutdown_reaps_suspended_contexts_even_after_their_runs_have_been_removed() {
    let workspace = private_tempdir();
    let driver = Arc::new(Driver::verified());
    driver.retain_completed.store(true, Ordering::SeqCst);
    let runtime = runtime(
        workspace.path(),
        &config(workspace.path()),
        Some(driver.clone()),
    );
    let first = begin(&runtime, "first");
    call(
        &executor(&runtime),
        &first,
        "browser.open",
        open_arguments(),
    )
    .await
    .unwrap();
    runtime
        .browser
        .as_ref()
        .unwrap()
        .complete_run("first")
        .await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 0);
    runtime.browser.as_ref().unwrap().drain().await;
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);
}
