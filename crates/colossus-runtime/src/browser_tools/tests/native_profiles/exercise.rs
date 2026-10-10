use super::*;

pub(super) async fn finish(
    runtime: &Runtime,
    supervisor: &OciBrowserSupervisor,
    acknowledged: &AtomicUsize,
    run: &str,
) {
    runtime.browser.as_ref().unwrap().finish_run(run).await;
    assert!(!runtime.browser_active_work());
    // The supervisor retires closed ledger entries before the next admission;
    // capture the positive receipt now rather than inferring history from its final map.
    let receipts = supervisor.acceptance_launch_stages().await;
    let actual: Vec<_> = receipts
        .iter()
        .filter(|(_, _, receipt)| *receipt != OciBrowserShutdownReceipt::NoProcess)
        .collect();
    assert_eq!(actual.len(), 1);
    assert!(actual[0].1);
    assert_eq!(actual[0].2, OciBrowserShutdownReceipt::GracefulCefShutdown);
    acknowledged.fetch_add(1, Ordering::SeqCst);
}
pub(super) async fn page(
    runtime: &Runtime,
    context: &ExecutionContext,
    origin: &str,
    path: &str,
    profile: &BrowserProfileSelection,
    expected: &str,
) {
    let tools = executor(runtime);
    let opened = call(&tools, context, "browser.open", json!({
        "mode":"headless", "allowed_origins":[origin], "initial_url":format!("{origin}{path}"), "profile":profile,
    })).await.unwrap();
    let snapshot = call(
        &tools,
        context,
        "browser.snapshot",
        snapshot_target(&opened),
    )
    .await
    .unwrap();
    assert!(serde_json::to_string(&snapshot).unwrap().contains(expected));
}
