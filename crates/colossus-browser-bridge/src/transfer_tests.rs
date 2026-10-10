//! Authenticated file transfers reject wrong owner, stale tickets and replay before native effects.
use crate::{tests::*, *};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_contracts::*;
use colossus_ports::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
mod native;
mod unknown;
use native::*;
fn upload() -> BrowserUploadPrepareRequest {
    let mut command = command(1);
    command.action = BrowserAction::Upload {
        element: element(),
        artifact_id: "artifact".into(),
        max_bytes: 100_000,
    };
    BrowserUploadPrepareRequest {
        command,
        descriptor: BrowserUploadDescriptor {
            artifact_id: "artifact".into(),
            size_bytes: 70_000,
            sha256: "7".repeat(64),
            file_name: "input.txt".into(),
        },
    }
}
fn element() -> BrowserElementRef {
    BrowserElementRef {
        document_id: command(1).target.document_id,
        snapshot_id: BrowserSnapshotId::parse(format!("bn_{}", "8".repeat(32))).unwrap(),
        element_id: BrowserElementId::parse(format!("be_{}", "9".repeat(32))).unwrap(),
    }
}
fn transfer(token: &str, offset: u32) -> BrowserScreenshotReadRequest {
    let command = command(1);
    BrowserScreenshotReadRequest {
        binding: command.binding,
        run_id: command.run_id,
        session_id: command.session_id,
        target: command.target,
        control_generation: 1,
        transfer_id: token.into(),
        offset,
    }
}
fn write(token: &str, offset: u32, size: usize) -> BrowserUploadWriteRequest {
    BrowserUploadWriteRequest {
        transfer: transfer(token, offset),
        data_base64: STANDARD.encode(vec![1; size]),
    }
}
async fn fixture() -> (
    Arc<BrowserBridgeDriver>,
    Arc<Native>,
    tokio::task::JoinHandle<Result<(), BrowserDriverError>>,
) {
    let (data, host_data) = channel_pair();
    let (ctl, host_ctl) = channel_pair();
    let mut admitted = enrollment();
    admitted
        .capabilities
        .actions
        .extend([BrowserActionKind::Upload, BrowserActionKind::Download]);
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_ctl,
        admitted.clone(),
        BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new([7; 32])),
        driver,
    ));
    let browser = Arc::new(
        BrowserBridgeDriver::connect(
            data,
            ctl,
            admitted,
            BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new([7; 32])),
        )
        .await
        .unwrap(),
    );
    browser
        .open_session(open_request(), &control())
        .await
        .unwrap();
    (browser, native, host)
}
async fn finish(
    browser: Arc<BrowserBridgeDriver>,
    host: tokio::task::JoinHandle<Result<(), BrowserDriverError>>,
) {
    browser
        .close_session(&open_request().session_id)
        .await
        .unwrap();
    browser.disconnect_for_shutdown();
    drop(browser);
    tokio::time::timeout(Duration::from_secs(2), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn upload_is_ordered_complete_one_shot_and_commits_document_ledger() {
    let (browser, native, host) = fixture().await;
    let receipt = browser.prepare_upload(upload(), &control()).await.unwrap();
    assert_eq!(
        browser
            .commit_upload(transfer(&receipt.transfer_id, 0), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.commits.load(Ordering::SeqCst), 0);
    let receipt = browser
        .write_upload_chunk(write(&receipt.transfer_id, 0, 65_536), &control())
        .await
        .unwrap();
    assert_eq!(receipt.next_offset, 65_536);
    assert_eq!(
        browser
            .write_upload_chunk(write(&receipt.transfer_id, 0, 4_464), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    let receipt = browser
        .write_upload_chunk(write(&receipt.transfer_id, 65_536, 4_464), &control())
        .await
        .unwrap();
    let request = transfer(&receipt.transfer_id, 70_000);
    let observed = browser
        .commit_upload(request.clone(), &control())
        .await
        .unwrap();
    assert_eq!(observed.tab.document_id, command(1).next_document_id);
    assert_eq!(
        browser.commit_upload(request, &control()).await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(
        browser.execute(command(1), &control()).await,
        Err(BrowserDriverError::Stale)
    );
    let mut fresh = command(1);
    fresh.target.document_id = observed.tab.document_id;
    browser.execute(fresh, &control()).await.unwrap();
    assert_eq!(native.writes.load(Ordering::SeqCst), 2);
    assert_eq!(native.commits.load(Ordering::SeqCst), 1);
    finish(browser, host).await;
}
#[tokio::test]
async fn wrong_upload_owner_document_generation_token_and_bytes_never_dispatch() {
    let (browser, native, host) = fixture().await;
    let receipt = browser.prepare_upload(upload(), &control()).await.unwrap();
    for field in 0..7 {
        let mut request = write(&receipt.transfer_id, 0, 1);
        match field {
            0 => request.transfer.binding.application_id = "other".into(),
            1 => request.transfer.run_id = "other".into(),
            2 => request.transfer.control_generation = 2,
            3 => request.transfer.target.document_id = command(1).next_document_id,
            4 => request.transfer.transfer_id = "a".repeat(32),
            5 => request.transfer.offset = 1,
            _ => request.data_base64 = "!".into(),
        }
        assert!(
            browser
                .write_upload_chunk(request, &control())
                .await
                .is_err()
        );
    }
    assert_eq!(native.writes.load(Ordering::SeqCst), 0);
    finish(browser, host).await;
}
#[tokio::test]
async fn staging_retires_on_following_action_and_metadata_paths_are_denied() {
    let (browser, native, host) = fixture().await;
    for name in ["../input.txt", "C:\\input.txt", "a/b", ".", ".."] {
        let mut request = upload();
        request.descriptor.file_name = name.into();
        assert_eq!(
            browser.prepare_upload(request, &control()).await,
            Err(BrowserDriverError::Denied)
        );
    }
    assert_eq!(native.prepares.load(Ordering::SeqCst), 0);
    let receipt = browser.prepare_upload(upload(), &control()).await.unwrap();
    browser.execute(command(1), &control()).await.unwrap();
    assert_eq!(
        browser
            .write_upload_chunk(write(&receipt.transfer_id, 0, 1), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    finish(browser, host).await;
}
#[tokio::test]
async fn malformed_native_progress_is_unknown_and_fences_writer() {
    let (browser, native, host) = fixture().await;
    let receipt = browser.prepare_upload(upload(), &control()).await.unwrap();
    native.bad_receipt.store(true, Ordering::SeqCst);
    assert_eq!(
        browser
            .write_upload_chunk(write(&receipt.transfer_id, 0, 1), &control())
            .await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert_eq!(
        browser.execute(command(1), &control()).await,
        Err(BrowserDriverError::Unavailable)
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while native.cancels.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(native.cancels.load(Ordering::SeqCst) > 0);
    finish(browser, host).await;
}
fn download() -> BrowserDriverCommand {
    let mut command = command(1);
    command.action = BrowserAction::Download {
        element: element(),
        max_bytes: 100_000,
    };
    command
}
#[tokio::test]
async fn download_is_bound_ordered_and_retires_after_complete_read() {
    let (browser, native, host) = fixture().await;
    let descriptor = browser.download(download(), &control()).await.unwrap();
    for field in 0..5 {
        let mut request = transfer(&descriptor.transfer_id, 0);
        match field {
            0 => request.run_id = "other".into(),
            1 => request.control_generation = 2,
            2 => request.target.document_id = command(1).next_document_id,
            3 => request.offset = 1,
            _ => request.transfer_id = "a".repeat(32),
        }
        assert!(
            browser
                .read_download_chunk(request, &control())
                .await
                .is_err()
        );
    }
    assert_eq!(native.reads.load(Ordering::SeqCst), 0);
    let first = browser
        .read_download_chunk(transfer(&descriptor.transfer_id, 0), &control())
        .await
        .unwrap();
    assert_eq!(STANDARD.decode(&first.data_base64).unwrap().len(), 65_536);
    let last = browser
        .read_download_chunk(transfer(&descriptor.transfer_id, 65_536), &control())
        .await
        .unwrap();
    assert_eq!(STANDARD.decode(&last.data_base64).unwrap().len(), 4_464);
    assert_eq!(
        browser
            .read_download_chunk(transfer(&descriptor.transfer_id, 70_000), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    finish(browser, host).await;
}
#[tokio::test]
async fn unadmitted_download_origin_and_malformed_chunk_are_unknown() {
    for malformed_chunk in [false, true] {
        let (browser, native, host) = fixture().await;
        if malformed_chunk {
            let descriptor = browser.download(download(), &control()).await.unwrap();
            native.bad_chunk.store(true, Ordering::SeqCst);
            assert_eq!(
                browser
                    .read_download_chunk(transfer(&descriptor.transfer_id, 0), &control())
                    .await,
                Err(BrowserDriverError::OutcomeUnknown)
            );
        } else {
            native.outside_download.store(true, Ordering::SeqCst);
            assert_eq!(
                browser.download(download(), &control()).await,
                Err(BrowserDriverError::OutcomeUnknown)
            );
        }
        finish(browser, host).await;
    }
}
#[tokio::test(start_paused = true)]
async fn elapsed_upload_and_download_custody_do_not_dispatch_native() {
    let (browser, native, host) = fixture().await;
    let receipt = browser.prepare_upload(upload(), &control()).await.unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert_eq!(
        browser
            .write_upload_chunk(write(&receipt.transfer_id, 0, 1), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    let descriptor = browser.download(download(), &control()).await.unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert_eq!(
        browser
            .read_download_chunk(transfer(&descriptor.transfer_id, 0), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.writes.load(Ordering::SeqCst), 0);
    assert_eq!(native.reads.load(Ordering::SeqCst), 0);
    finish(browser, host).await;
}
#[tokio::test]
async fn dropped_upload_write_keeps_independent_cancel_and_cleanup() {
    let (browser, native, host) = fixture().await;
    let receipt = browser.prepare_upload(upload(), &control()).await.unwrap();
    native.block_write.store(true, Ordering::SeqCst);
    let retained = browser.clone();
    let writer = tokio::spawn(async move {
        retained
            .write_upload_chunk(write(&receipt.transfer_id, 0, 1), &control())
            .await
    });
    while native.writes.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    writer.abort();
    let _ = writer.await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while native.cancels.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    finish(browser, host).await;
}
#[tokio::test]
async fn supervised_pool_checks_receipts_before_disarming_cleanup() {
    let native = Arc::new(Native::default());
    let pool = BrowserHostPool::new(Arc::new(Factory(native.clone()))).unwrap();
    pool.open_session(open_request(), &control()).await.unwrap();
    let receipt = pool.prepare_upload(upload(), &control()).await.unwrap();
    native.bad_receipt.store(true, Ordering::SeqCst);
    assert_eq!(
        pool.write_upload_chunk(write(&receipt.transfer_id, 0, 1), &control())
            .await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    pool.close_session(&open_request().session_id)
        .await
        .unwrap();
    assert_eq!(pool.owned_session_count(), Ok(0));
    assert!(native.cancels.load(Ordering::SeqCst) > 0);
    assert!(native.closes.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn empty_download_requires_one_exact_empty_read_then_retires() {
    let (browser, native, host) = fixture().await;
    native.empty_download.store(true, Ordering::SeqCst);
    let descriptor = browser.download(download(), &control()).await.unwrap();
    assert_eq!(descriptor.size_bytes, 0);
    let chunk = browser
        .read_download_chunk(transfer(&descriptor.transfer_id, 0), &control())
        .await
        .unwrap();
    assert!(chunk.data_base64.is_empty());
    assert_eq!(
        browser
            .read_download_chunk(transfer(&descriptor.transfer_id, 0), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.reads.load(Ordering::SeqCst), 1);
    finish(browser, host).await;
}
