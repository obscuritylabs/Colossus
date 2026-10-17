use super::*;

#[tokio::test]
async fn read_snapshot_commits_fresh_native_document_and_fences_previous_mutations() {
    let (browser, native, host) = fixture(false).await;
    native.recover_document.store(true, Ordering::SeqCst);
    let mut request = command(1);
    request.action = BrowserAction::Snapshot { max_nodes: 32 };
    request.snapshot_id =
        Some(BrowserSnapshotId::parse(format!("bn_{}", "8".repeat(32))).expect("snapshot"));
    let next = request.next_document_id.clone();
    let result = browser
        .execute(request, &control())
        .await
        .expect("native read recovery");
    assert_eq!(result.tab.document_id, next);
    assert_eq!(result.snapshot.expect("fresh snapshot").document_id, next);
    assert_eq!(
        browser.execute(command(1), &control()).await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.executions.load(Ordering::SeqCst), 1);
    let mut fresh = command(1);
    fresh.target.document_id = next;
    browser
        .execute(fresh, &control())
        .await
        .expect("current owned document");
    finish(browser, host).await;
}
