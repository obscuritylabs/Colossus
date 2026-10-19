use super::{Installation, MacosBrowserShutdownReceipt};
use crate::BrowserEgressLease;
use async_trait::async_trait;
use colossus_browser_bridge::BrowserBridgeDriver;
use colossus_browser_presentation::PresentationClient;
use colossus_contracts::{
    BrowserCapabilities, BrowserObservation, BrowserSessionId, BrowserTabSummary,
};
use colossus_darwin_process::{DarwinAuditPeer, DarwinChild, DarwinDirectChildCleanup};
use colossus_mac_browser_process_broker::CoreProcessOwner;
use colossus_ports::{
    BrowserCancelDisposition, BrowserDownloadChunk, BrowserDownloadDescriptor,
    BrowserDownloadReadRequest, BrowserDriver, BrowserDriverCommand, BrowserDriverControl,
    BrowserDriverError, BrowserDriverOpenRequest, BrowserNativeHandoffRequest,
    BrowserScreenshotChunk, BrowserScreenshotDescriptor, BrowserScreenshotReadRequest,
    BrowserUploadCommitRequest, BrowserUploadPrepareRequest, BrowserUploadReceipt,
    BrowserUploadWriteRequest,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;

pub(super) struct Resources {
    pub request: BrowserDriverOpenRequest,
    pub installation: Arc<Installation>,
    pub child: Option<DarwinChild>,
    pub process_owner: Option<CoreProcessOwner>,
    #[cfg(feature = "macos-browser-tls-development")]
    pub tls: Option<super::tls::Owner>,
    pub spawn_attempted: bool,
    child_cleanup: Option<DarwinDirectChildCleanup>,
    pub peer: Option<DarwinAuditPeer>,
    pub egress: Option<BrowserEgressLease>,
    pub bridge: Option<Arc<BrowserBridgeDriver>>,
    pub presentation: Option<(PresentationClient, [u8; 32])>,
    pub drains: Vec<tokio::task::JoinHandle<std::io::Result<u64>>>,
    drain_failed: bool,
    pub fenced: Arc<AtomicBool>,
    pub closed: bool,
    pub cef_acknowledged: bool,
    pub receipt: Option<MacosBrowserShutdownReceipt>,
}

impl Resources {
    pub fn new(
        request: BrowserDriverOpenRequest,
        installation: Arc<Installation>,
        fenced: Arc<AtomicBool>,
    ) -> Self {
        Self {
            request,
            installation,
            child: None,
            process_owner: None,
            #[cfg(feature = "macos-browser-tls-development")]
            tls: None,
            spawn_attempted: false,
            child_cleanup: None,
            peer: None,
            egress: None,
            bridge: None,
            presentation: None,
            drains: Vec::new(),
            drain_failed: false,
            fenced,
            closed: false,
            cef_acknowledged: false,
            receipt: None,
        }
    }

    pub async fn cleanup(&mut self) -> Result<(), BrowserDriverError> {
        self.fenced.store(true, Ordering::Release);
        if self.closed {
            return Ok(());
        }
        if let Some(bridge) = &self.bridge {
            bridge.disconnect_for_shutdown();
        }
        if let Some((presenter, _)) = &self.presentation {
            presenter.disconnect();
        }
        let mut unknown = false;
        #[cfg(feature = "macos-browser-tls-development")]
        if let Some(tls) = &mut self.tls
            && tls.close().await.is_err()
        {
            unknown = true;
        }
        if let Some(tls_owner) = &self.installation.tls_owner
            && !matches!(
                tokio::time::timeout(Duration::from_secs(7), tls_owner.revoke()).await,
                Ok(Ok(()))
            )
        {
            // TLS lifetime failure cannot suppress independent process/proxy
            // cleanup, and the exact owner remains held by the installation.
            unknown = true;
        }
        if let Some(egress) = &mut self.egress
            && !matches!(
                tokio::time::timeout(Duration::from_secs(2), egress.revoke()).await,
                Ok(Ok(()))
            )
        {
            // A proxy failure cannot suppress the independent child and pipe
            // cleanup duties. Its lease retains the unresolved revocation.
            unknown = true;
        }
        let owned_process = self.process_owner.is_some();
        if let Some(owner) = self.process_owner.as_mut()
            && owner.close().is_err()
        {
            unknown = true;
        }
        if self.cef_acknowledged
            && let Some(child) = self.child.as_mut()
        {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {}
                    Err(_) => {
                        unknown = true;
                        break;
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        if let Some(child) = self.child.as_mut() {
            match child.terminate_and_reap(Duration::from_secs(2)) {
                Ok(cleanup) => {
                    eprintln!(
                        "Mac diagnostic cleanup: direct_child_exit={:?}, cef_acknowledged={}",
                        cleanup.status().code(),
                        self.cef_acknowledged
                    );
                    // Preserve exact terminal evidence before the next await;
                    // interrupted pipe cleanup must never lose a completed reap.
                    self.child_cleanup = Some(cleanup);
                    self.child.take();
                }
                Err(_) => {
                    unknown = true;
                }
            }
        } else if self.spawn_attempted && self.child_cleanup.is_none() && !owned_process {
            // A failed native spawn may have crossed posix_spawn before its
            // internal cleanup failed. Absence of a returned child is no receipt.
            unknown = true;
        }
        if settle_drains(
            &mut self.drains,
            &mut self.drain_failed,
            Duration::from_secs(2),
        )
        .await
        .is_err()
        {
            unknown = true;
        }
        if unknown {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.closed = true;
        let owned_graceful = self.process_owner.as_ref().is_some_and(|owner| {
            owner
                .receipt()
                .is_some_and(|receipt| receipt.main_status == Some(0))
        });
        self.receipt = Some(
            match (owned_process, self.cef_acknowledged, self.child_cleanup) {
                (true, true, _) if owned_graceful => {
                    MacosBrowserShutdownReceipt::GracefulKnownChildrenExit
                }
                (true, _, _) => MacosBrowserShutdownReceipt::TerminatedKnownChildrenExit,
                (_, _, Some(cleanup)) if self.cef_acknowledged && cleanup.status().success() => {
                    MacosBrowserShutdownReceipt::GracefulDirectChildExit
                }
                (_, _, Some(_)) => MacosBrowserShutdownReceipt::TerminatedDirectChildExit,
                (_, _, None) => MacosBrowserShutdownReceipt::NoDirectChildAllocated,
            },
        );
        Ok(())
    }
}

/// Consume completed handles before the next await. Tokio handles must never be
/// polled again after completion, including after an interrupted cleanup retry.
/// Pending handles remain owned; terminal capture failures stay unknown even
/// after every other independent drain has finished.
async fn settle_drains(
    drains: &mut Vec<tokio::task::JoinHandle<std::io::Result<u64>>>,
    failed: &mut bool,
    timeout: Duration,
) -> Result<(), BrowserDriverError> {
    let mut index = 0;
    while index < drains.len() {
        match tokio::time::timeout(timeout, &mut drains[index]).await {
            Ok(Ok(Ok(_))) => {
                drop(drains.remove(index));
            }
            Ok(_) => {
                *failed = true;
                drop(drains.remove(index));
            }
            Err(_) => index += 1,
        }
    }
    if *failed || !drains.is_empty() {
        Err(BrowserDriverError::OutcomeUnknown)
    } else {
        Ok(())
    }
}

pub(super) struct Driver {
    bridge: Arc<BrowserBridgeDriver>,
    resources: Arc<Mutex<Resources>>,
    request: BrowserDriverOpenRequest,
    fenced: Arc<AtomicBool>,
}

impl Driver {
    pub fn new(
        bridge: Arc<BrowserBridgeDriver>,
        resources: Arc<Mutex<Resources>>,
        request: BrowserDriverOpenRequest,
        fenced: Arc<AtomicBool>,
    ) -> Self {
        Self {
            bridge,
            resources,
            request,
            fenced,
        }
    }

    fn accepts(
        &self,
        binding: &colossus_contracts::BrowserSessionBinding,
        session: &BrowserSessionId,
    ) -> bool {
        !self.fenced.load(Ordering::Acquire)
            && binding == &self.request.binding
            && session == &self.request.session_id
    }
}

#[async_trait]
impl BrowserDriver for Driver {
    fn capabilities(&self) -> BrowserCapabilities {
        if self.fenced.load(Ordering::Acquire) {
            BrowserCapabilities::unavailable()
        } else {
            self.bridge.capabilities()
        }
    }
    fn cancellation_disposition(&self) -> BrowserCancelDisposition {
        BrowserCancelDisposition::ClosesContext
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire) || request != self.request {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.open_session(request, control).await
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if !self.accepts(&command.binding, &command.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.execute(command, control).await
    }
    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        if !self.accepts(&command.binding, &command.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.capture(command, control).await
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        if !self.accepts(&request.binding, &request.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.read_screenshot_chunk(request, control).await
    }
    async fn prepare_upload(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if !self.accepts(&request.command.binding, &request.command.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.prepare_upload(request, control).await
    }
    async fn write_upload_chunk(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if !self.accepts(&request.transfer.binding, &request.transfer.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.write_upload_chunk(request, control).await
    }
    async fn commit_upload(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if !self.accepts(&request.binding, &request.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.commit_upload(request, control).await
    }
    async fn download(
        &self,
        request: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        if !self.accepts(&request.binding, &request.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.download(request, control).await
    }
    async fn read_download_chunk(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadChunk, BrowserDriverError> {
        if !self.accepts(&request.binding, &request.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.read_download_chunk(request, control).await
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if !self.accepts(&request.binding, &request.session_id) {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.confirm_native_handoff(request, control).await
    }
    async fn cancel_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.close_session(session).await
    }
    async fn close_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        if session != &self.request.session_id {
            return Err(BrowserDriverError::Denied);
        }
        self.fenced.store(true, Ordering::Release);
        let bridge_result = self.bridge.close_session(session).await;
        if bridge_result.is_ok() {
            self.resources.lock().await.cef_acknowledged = true;
        }
        let cleanup = self.resources.lock().await.cleanup().await;
        cleanup?;
        bridge_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{
        BrowserDocumentId, BrowserOpenOptions, BrowserScope, BrowserSessionBinding, BrowserTabId,
    };
    use std::os::unix::fs::PermissionsExt as _;
    use tokio::sync::oneshot;

    fn unstarted_resources() -> (tempfile::TempDir, Resources) {
        let directory = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let custody =
            colossus_home::ConfinedRoot::bind(directory.path().canonicalize().unwrap()).unwrap();
        let installation = Installation {
            allocation: "/private/tmp/unstarted-browser-test".into(),
            executable: "/private/tmp/unstarted-browser-test/host".into(),
            manifest_digest: [1; 32],
            front_cdhash: [2; 20],
            body_cdhash: [3; 20],
            app: "/private/tmp/unstarted-browser-test/host.app".into(),
            manifest: "/private/tmp/unstarted-browser-test/manifest".into(),
            manifest_entries: Vec::new(),
            policy: "/private/tmp/unstarted-browser-test/policy".into(),
            policy_sha256: [4; 32],
            custody,
            capabilities: BrowserCapabilities::unavailable(),
            audit_session: 123,
            proxy_port: 60924,
            egress: crate::BrowserEgressLimits {
                max_connections: 1,
                lifetime: Duration::from_secs(1),
                connection_timeout: Duration::from_secs(1),
                max_connection_bytes: 64 * 1024,
            },
            owned_process: None,
            tls_owner: None,
            tls_config: None,
            #[cfg(feature = "macos-browser-tls-development")]
            pki_registry: None,
        };
        let request = BrowserDriverOpenRequest {
            binding: BrowserSessionBinding {
                runtime_id: "fixture-runtime".into(),
                workspace_id: "fixture-workspace".into(),
                application_id: "fixture-application".into(),
                scope: BrowserScope::Conversation {
                    id: "fixture-conversation".into(),
                },
            },
            run_id: None,
            session_id: BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).unwrap(),
            tab_id: BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).unwrap(),
            document_id: BrowserDocumentId::parse(format!("bd_{}", "3".repeat(32))).unwrap(),
            options: BrowserOpenOptions {
                public_web: false,
                mode: colossus_contracts::BrowserMode::Embedded,
                allowed_origins: Vec::new(),
                initial_url: None,
                profile: Default::default(),
            },
        };
        (
            directory,
            Resources::new(
                request,
                Arc::new(installation),
                Arc::new(AtomicBool::new(false)),
            ),
        )
    }

    #[tokio::test]
    async fn pre_spawn_admission_failure_closes_without_claiming_child_exit() {
        let (_directory, mut resource) = unstarted_resources();
        assert_eq!(resource.cleanup().await, Ok(()));
        assert!(resource.closed);
        assert!(resource.fenced.load(Ordering::Acquire));
        assert_eq!(
            resource.receipt,
            Some(MacosBrowserShutdownReceipt::NoDirectChildAllocated)
        );
        assert_eq!(resource.cleanup().await, Ok(()));
    }

    #[tokio::test]
    async fn spawn_error_without_returned_owner_never_claims_no_child() {
        let (_directory, mut resource) = unstarted_resources();
        resource.spawn_attempted = true;
        assert_eq!(
            resource.cleanup().await,
            Err(BrowserDriverError::OutcomeUnknown)
        );
        assert!(!resource.closed);
        assert!(resource.receipt.is_none());
        assert_eq!(
            resource.cleanup().await,
            Err(BrowserDriverError::OutcomeUnknown)
        );
    }

    fn blocked_drain() -> (
        tokio::task::JoinHandle<std::io::Result<u64>>,
        oneshot::Sender<()>,
    ) {
        let (release, wait) = oneshot::channel();
        let task = tokio::spawn(async move {
            wait.await.map_err(std::io::Error::other)?;
            Ok(0)
        });
        (task, release)
    }

    #[tokio::test]
    async fn drain_timeout_retry_never_repolls_completed_handle() {
        let completed = tokio::spawn(async { Ok(17) });
        let (blocked, release) = blocked_drain();
        let mut drains = vec![completed, blocked];
        let mut failed = false;
        assert_eq!(
            settle_drains(&mut drains, &mut failed, Duration::from_millis(20)).await,
            Err(BrowserDriverError::OutcomeUnknown)
        );
        assert_eq!(drains.len(), 1);
        assert!(!failed);
        release.send(()).unwrap();
        assert_eq!(
            settle_drains(&mut drains, &mut failed, Duration::from_secs(1)).await,
            Ok(())
        );
        assert!(drains.is_empty());
    }

    #[tokio::test]
    async fn cancelled_drain_wait_retains_only_unconsumed_handles() {
        let completed = tokio::spawn(async { Ok(17) });
        while !completed.is_finished() {
            tokio::task::yield_now().await;
        }
        let (blocked, release) = blocked_drain();
        let mut drains = vec![completed, blocked];
        let mut failed = false;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                settle_drains(&mut drains, &mut failed, Duration::from_secs(1)),
            )
            .await
            .is_err()
        );
        assert_eq!(drains.len(), 1);
        release.send(()).unwrap();
        assert_eq!(
            settle_drains(&mut drains, &mut failed, Duration::from_secs(1)).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn terminal_drain_failure_does_not_skip_other_cleanup_or_become_success() {
        let failed_task =
            tokio::spawn(async { Err(std::io::Error::other("synthetic capture failure")) });
        let (blocked, release) = blocked_drain();
        let mut drains = vec![failed_task, blocked];
        let mut failed = false;
        assert_eq!(
            settle_drains(&mut drains, &mut failed, Duration::from_millis(20)).await,
            Err(BrowserDriverError::OutcomeUnknown)
        );
        assert!(failed);
        assert_eq!(drains.len(), 1);
        release.send(()).unwrap();
        assert_eq!(
            settle_drains(&mut drains, &mut failed, Duration::from_secs(1)).await,
            Err(BrowserDriverError::OutcomeUnknown)
        );
        assert!(drains.is_empty());
        assert_eq!(
            settle_drains(&mut drains, &mut failed, Duration::from_secs(1)).await,
            Err(BrowserDriverError::OutcomeUnknown)
        );
    }
}
