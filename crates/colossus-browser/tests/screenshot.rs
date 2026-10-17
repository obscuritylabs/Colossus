//! Exact-byte screenshot assembly and late-writer rejection at the coordinator boundary.
mod support;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_browser::{BrowserCoordinator, BrowserError};
use colossus_contracts::*;
use colossus_ports::*;
use sha2::{Digest as _, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use support::*;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQMgkDAAD4AJ3MaiF4AAAAAElFTkSuQmCC";

#[derive(Default)]
struct CaptureDriver {
    inner: TestDriver,
    captures: AtomicUsize,
    bad_offset: AtomicBool,
    bad_digest: AtomicBool,
    oversized_chunk: AtomicBool,
    block_read: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl BrowserDriver for CaptureDriver {
    fn capabilities(&self) -> BrowserCapabilities {
        let mut capabilities = self.inner.capabilities();
        capabilities.actions.push(BrowserActionKind::Screenshot);
        capabilities
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.inner.open_session(request, control).await
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.inner.execute(command, control).await
    }
    async fn capture(
        &self,
        command: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        let bytes = BASE64.decode(PNG).unwrap();
        Ok(BrowserScreenshotDescriptor {
            session_id: command.session_id,
            target: command.target,
            control_generation: command.control_generation,
            transfer_id: "a".repeat(32),
            size_bytes: bytes.len() as u32,
            sha256: if self.bad_digest.load(Ordering::SeqCst) {
                "0".repeat(64)
            } else {
                format!("{:x}", Sha256::digest(&bytes))
            },
            width: 1,
            height: 1,
        })
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        if self.block_read.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(BrowserScreenshotChunk {
            offset: request.offset + u32::from(self.bad_offset.load(Ordering::SeqCst)),
            data_base64: if self.oversized_chunk.load(Ordering::SeqCst) {
                "A".repeat(BROWSER_SCREENSHOT_CHUNK_BYTES.div_ceil(3) * 4 + 4)
            } else {
                PNG.into()
            },
        })
    }
    async fn cancel_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.inner.cancel_session(session).await
    }
    async fn close_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.inner.close_session(session).await
    }
}

async fn setup(
    driver: Arc<CaptureDriver>,
) -> (
    Arc<BrowserCoordinator>,
    BrowserActor,
    BrowserSessionSummary,
    BrowserControlLease,
) {
    let coordinator = Arc::new(BrowserCoordinator::new(driver, BrowserLimits::default()));
    let actor = actor();
    let session = coordinator
        .open_for_run(&actor, options(), &RunControl::default())
        .await
        .unwrap();
    let lease = coordinator
        .grant_control(&actor, &session.session_id, 30_000)
        .unwrap();
    (coordinator, actor, session, lease)
}

#[tokio::test]
async fn capture_without_snapshot_assembles_exact_png_under_owned_lease() {
    let driver = Arc::new(CaptureDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    assert_eq!(driver.inner.active_tabs(), 1);
    let capture = coordinator
        .screenshot(
            &actor,
            &lease,
            &target(&session),
            MAX_BROWSER_SCREENSHOT_BYTES,
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert_eq!(coordinator.list(&actor.binding).unwrap().len(), 1);
    assert_eq!(*capture.bytes, BASE64.decode(PNG).unwrap());
    assert_eq!(capture.descriptor.target, target(&session));
    assert_eq!(
        coordinator
            .get(&actor.binding, &session.session_id)
            .unwrap()
            .tabs,
        session.tabs
    );
}

#[tokio::test]
async fn malformed_private_chunk_or_digest_cannot_become_releasable_image() {
    for failure in 0..3 {
        let driver = Arc::new(CaptureDriver::default());
        match failure {
            0 => driver.bad_offset.store(true, Ordering::SeqCst),
            1 => driver.bad_digest.store(true, Ordering::SeqCst),
            _ => driver.oversized_chunk.store(true, Ordering::SeqCst),
        }
        let (coordinator, actor, session, lease) = setup(driver.clone()).await;
        assert!(matches!(
            coordinator
                .screenshot(
                    &actor,
                    &lease,
                    &target(&session),
                    MAX_BROWSER_SCREENSHOT_BYTES,
                    &RunControl::default()
                )
                .await,
            Err(BrowserError::OutcomeUnknown)
        ));
        assert!(driver.inner.cancellations.load(Ordering::SeqCst) >= 1);
    }
}

#[tokio::test]
async fn writer_takeover_while_reading_discards_even_valid_late_png() {
    let driver = Arc::new(CaptureDriver::default());
    driver.block_read.store(true, Ordering::SeqCst);
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let pending = {
        let coordinator = coordinator.clone();
        let actor = actor.clone();
        let target = target(&session);
        tokio::spawn(async move {
            coordinator
                .screenshot(
                    &actor,
                    &lease,
                    &target,
                    MAX_BROWSER_SCREENSHOT_BYTES,
                    &RunControl::default(),
                )
                .await
        })
    };
    driver.entered.notified().await;
    coordinator
        .takeover(&actor.binding, &session.session_id)
        .await
        .unwrap();
    driver.release.notify_one();
    assert!(matches!(
        pending.await.unwrap(),
        Err(BrowserError::OutcomeUnknown)
    ));
}
