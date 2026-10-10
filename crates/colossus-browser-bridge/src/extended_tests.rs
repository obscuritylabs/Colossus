//! Adversarial screenshot ownership and irreversible native document adoption.
use crate::{tests::*, *};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_contracts::*;
use colossus_ports::*;
use sha2::{Digest as _, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

mod recovery;

#[derive(Default)]
struct Native {
    captures: AtomicUsize,
    executions: AtomicUsize,
    reads: AtomicUsize,
    confirmations: AtomicUsize,
    cancels: AtomicUsize,
    closes: AtomicUsize,
    bad_descriptor: AtomicBool,
    bad_chunk: AtomicBool,
    block_capture: AtomicBool,
    recover_document: AtomicBool,
}
#[async_trait]
impl BrowserDriver for Native {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        Ok(BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            title: String::new(),
            origin: None,
        })
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let recovered = self.recover_document.load(Ordering::SeqCst)
            && matches!(command.action, BrowserAction::Snapshot { .. });
        let document = if recovered {
            command.next_document_id
        } else {
            command.target.document_id
        };
        let snapshot = if recovered {
            Some(BrowserSnapshot {
                snapshot_id: command.snapshot_id.ok_or(BrowserDriverError::Denied)?,
                document_id: document.clone(),
                nodes: vec![],
                truncated: false,
            })
        } else {
            None
        };
        Ok(BrowserObservation {
            session_id: command.session_id,
            tab: BrowserTabSummary {
                tab_id: command.target.tab_id,
                document_id: document,
                title: String::new(),
                origin: None,
            },
            snapshot,
            truncated: false,
        })
    }
    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        while self.block_capture.load(Ordering::SeqCst) {
            if control.is_cancelled() {
                return Err(BrowserDriverError::Cancelled);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        Ok(BrowserScreenshotDescriptor {
            session_id: command.session_id,
            target: command.target,
            control_generation: command.control_generation,
            transfer_id: "5".repeat(32),
            size_bytes: if self.bad_descriptor.load(Ordering::SeqCst) {
                MAX_BROWSER_SCREENSHOT_BYTES + 1
            } else {
                70_000
            },
            sha256: format!("{:x}", Sha256::digest(vec![9; 70_000])),
            width: 800,
            height: 600,
        })
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserScreenshotChunk {
            offset: request.offset,
            data_base64: if self.bad_chunk.load(Ordering::SeqCst) {
                "!".into()
            } else {
                STANDARD.encode(vec![
                    9;
                    (70_000 - request.offset as usize)
                        .min(BROWSER_SCREENSHOT_CHUNK_BYTES)
                ])
            },
        })
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.confirmations.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserTabSummary {
            tab_id: request.confirmed_target.tab_id,
            document_id: request.confirmed_target.document_id,
            title: String::new(),
            origin: None,
        })
    }
    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn close_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
async fn fixture(
    human: bool,
) -> (
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
        .push(BrowserActionKind::Screenshot);
    let native = Arc::new(Native::default());
    let driver: Arc<dyn BrowserDriver> = native.clone();
    let host_enrollment = admitted.clone();
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_ctl,
        host_enrollment,
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
        .expect("ready"),
    );
    let mut open = open_request();
    if human {
        open.run_id = None;
    }
    browser.open_session(open, &control()).await.expect("open");
    (browser, native, host)
}
fn screenshot() -> BrowserDriverCommand {
    let mut value = command(1);
    value.action = BrowserAction::Screenshot { max_bytes: 100_000 };
    value
}
fn read(descriptor: &BrowserScreenshotDescriptor) -> BrowserScreenshotReadRequest {
    BrowserScreenshotReadRequest {
        binding: enrollment().binding,
        run_id: "run".into(),
        session_id: descriptor.session_id.clone(),
        target: descriptor.target.clone(),
        control_generation: descriptor.control_generation,
        transfer_id: descriptor.transfer_id.clone(),
        offset: 0,
    }
}
async fn finish(
    browser: Arc<BrowserBridgeDriver>,
    host: tokio::task::JoinHandle<Result<(), BrowserDriverError>>,
) {
    browser
        .close_session(&open_request().session_id)
        .await
        .expect("close");
    browser.disconnect_for_shutdown();
    host.await.expect("owner").expect("drain");
}

#[tokio::test]
async fn screenshot_chunks_are_exact_ordered_owned_bounded_and_retired() {
    let (browser, native, host) = fixture(false).await;
    let descriptor = browser
        .capture(screenshot(), &control())
        .await
        .expect("capture");
    let first = browser
        .read_screenshot_chunk(read(&descriptor), &control())
        .await
        .expect("first");
    assert_eq!(
        STANDARD.decode(&first.data_base64).expect("base64").len(),
        65_536
    );
    assert_eq!(
        browser
            .read_screenshot_chunk(read(&descriptor), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    let mut request = read(&descriptor);
    request.offset = 65_536;
    let last = browser
        .read_screenshot_chunk(request.clone(), &control())
        .await
        .expect("last");
    assert_eq!(
        STANDARD.decode(&last.data_base64).expect("base64").len(),
        4_464
    );
    assert_eq!(
        browser.read_screenshot_chunk(request, &control()).await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.reads.load(Ordering::SeqCst), 2);
    finish(browser, host).await;
}

#[tokio::test]
async fn screenshot_wrong_authority_document_transfer_and_offset_never_reach_native() {
    let (browser, native, host) = fixture(false).await;
    let descriptor = browser
        .capture(screenshot(), &control())
        .await
        .expect("capture");
    for field in 0..6 {
        let mut request = read(&descriptor);
        match field {
            0 => request.binding.application_id = "other".into(),
            1 => request.run_id = "other-run".into(),
            2 => request.control_generation = 2,
            3 => request.target.document_id = command(1).next_document_id,
            4 => request.transfer_id = "6".repeat(32),
            _ => request.offset = 1,
        }
        assert!(
            browser
                .read_screenshot_chunk(request, &control())
                .await
                .is_err()
        );
    }
    assert_eq!(native.reads.load(Ordering::SeqCst), 0);
    finish(browser, host).await;
}

#[tokio::test]
async fn any_following_action_retires_the_previous_capture() {
    let (browser, native, host) = fixture(false).await;
    let descriptor = browser
        .capture(screenshot(), &control())
        .await
        .expect("capture");
    browser
        .execute(command(1), &control())
        .await
        .expect("action");
    assert_eq!(
        browser
            .read_screenshot_chunk(read(&descriptor), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.reads.load(Ordering::SeqCst), 0);
    finish(browser, host).await;
}

#[tokio::test]
async fn malformed_native_receipt_or_chunk_is_unknown_and_cancels_without_replay() {
    for bad_descriptor in [true, false] {
        let (browser, native, host) = fixture(false).await;
        native
            .bad_descriptor
            .store(bad_descriptor, Ordering::SeqCst);
        let result = browser.capture(screenshot(), &control()).await;
        if bad_descriptor {
            assert_eq!(result, Err(BrowserDriverError::OutcomeUnknown));
        } else {
            native.bad_chunk.store(true, Ordering::SeqCst);
            assert_eq!(
                browser
                    .read_screenshot_chunk(read(&result.expect("capture")), &control())
                    .await,
                Err(BrowserDriverError::OutcomeUnknown)
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(native.cancels.load(Ordering::SeqCst) > 0);
        assert_eq!(native.captures.load(Ordering::SeqCst), 1);
        finish(browser, host).await;
    }
}

#[tokio::test]
async fn dropped_capture_retains_authenticated_native_cleanup() {
    let (browser, native, host) = fixture(false).await;
    native.block_capture.store(true, Ordering::SeqCst);
    let owned = browser.clone();
    let caller = tokio::spawn(async move { owned.capture(screenshot(), &control()).await });
    while native.captures.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    caller.abort();
    let _ = caller.await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(native.cancels.load(Ordering::SeqCst) > 0);
    assert_eq!(browser.owned_session_count().expect("owned"), 1);
    finish(browser, host).await;
}

fn handoff() -> BrowserNativeHandoffRequest {
    BrowserNativeHandoffRequest {
        binding: enrollment().binding,
        session_id: open_request().session_id,
        expected_target: command(1).target,
        confirmed_target: BrowserTarget {
            tab_id: open_request().tab_id,
            document_id: command(1).next_document_id,
        },
        native_document_generation: 2,
    }
}
#[tokio::test]
async fn native_handoff_adopts_confirmed_document_once_before_first_agent_dispatch() {
    let (browser, native, host) = fixture(true).await;
    let request = handoff();
    let tab = browser
        .confirm_native_handoff(request.clone(), &control())
        .await
        .expect("native confirmation");
    assert_eq!(tab.document_id, request.confirmed_target.document_id);
    assert_eq!(
        browser
            .confirm_native_handoff(request.clone(), &control())
            .await,
        Err(BrowserDriverError::Stale)
    );
    assert_eq!(native.confirmations.load(Ordering::SeqCst), 1);
    assert_eq!(
        browser.execute(command(1), &control()).await,
        Err(BrowserDriverError::Stale)
    );
    let mut current = command(1);
    current.target = request.confirmed_target;
    browser
        .execute(current, &control())
        .await
        .expect("fresh agent document");
    finish(browser, host).await;
}
#[tokio::test]
async fn native_handoff_rejects_agent_open_and_changed_original_admission() {
    for human in [false, true] {
        let (browser, native, host) = fixture(human).await;
        let mut request = handoff();
        if human {
            request.expected_target.document_id = command(1).next_document_id;
        }
        assert!(
            browser
                .confirm_native_handoff(request, &control())
                .await
                .is_err()
        );
        assert_eq!(native.confirmations.load(Ordering::SeqCst), 0);
        finish(browser, host).await;
    }
}

struct Factory(Arc<Native>);
#[async_trait]
impl BrowserHostFactory for Factory {
    fn capabilities(&self) -> BrowserCapabilities {
        let mut caps = enrollment().capabilities;
        caps.actions.push(BrowserActionKind::Screenshot);
        caps
    }
    async fn launch(
        &self,
        _: &BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        Ok(self.0.clone())
    }
    async fn reap_failed_launch(
        &self,
        _: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        Ok(())
    }
}
async fn pool_settled(pool: &BrowserHostPool) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while pool.owned_session_count().expect("owned") != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("finite physical cleanup");
}
#[tokio::test]
async fn pool_malformed_capture_retains_cleanup_before_delivery() {
    let native = Arc::new(Native::default());
    let pool = BrowserHostPool::new(Arc::new(Factory(native.clone()))).expect("pool");
    pool.open_session(open_request(), &control())
        .await
        .expect("open");
    native.bad_descriptor.store(true, Ordering::SeqCst);
    assert_eq!(
        pool.capture(screenshot(), &control()).await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    pool_settled(&pool).await;
    assert_eq!(native.captures.load(Ordering::SeqCst), 1);
    assert_eq!(native.closes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn pool_capture_drop_keeps_native_owner_until_cleanup_acknowledges() {
    let native = Arc::new(Native::default());
    let pool = Arc::new(BrowserHostPool::new(Arc::new(Factory(native.clone()))).expect("pool"));
    pool.open_session(open_request(), &control())
        .await
        .expect("open");
    native.block_capture.store(true, Ordering::SeqCst);
    let owned = pool.clone();
    let caller = tokio::spawn(async move { owned.capture(screenshot(), &control()).await });
    while native.captures.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    caller.abort();
    let _ = caller.await;
    pool_settled(&pool).await;
    assert_eq!(native.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(native.closes.load(Ordering::SeqCst), 1);
    assert_eq!(
        pool.capture(screenshot(), &control()).await,
        Err(BrowserDriverError::Stale)
    );
}
