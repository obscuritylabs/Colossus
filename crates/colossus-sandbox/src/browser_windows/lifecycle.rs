use super::{WindowsBrowserPresentation, WindowsBrowserShutdownReceipt, owner};
use crate::BrowserEgressLease;
use async_trait::async_trait;
use colossus_browser_bridge::BrowserBridgeDriver;
use colossus_browser_presentation::PresentationClient;
use colossus_contracts::{
    BrowserCapabilities, BrowserObservation, BrowserSessionId, BrowserTabSummary,
};
use colossus_ports::{
    BrowserCancelDisposition, BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserDriver,
    BrowserDriverCommand, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
    BrowserNativeHandoffRequest, BrowserScreenshotChunk, BrowserScreenshotDescriptor,
    BrowserScreenshotReadRequest, BrowserUploadCommitRequest, BrowserUploadPrepareRequest,
    BrowserUploadReceipt, BrowserUploadWriteRequest,
};
use colossus_windows_native::{BoundPath, PrivateDirectoryCreation};
use colossus_windows_process::PrivateIoLease;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;

pub(super) struct Resources {
    pub request: BrowserDriverOpenRequest,
    pub creation: PrivateDirectoryCreation,
    pub directory: Option<BoundPath>,
    pub profile: PathBuf,
    pub quarantine: Option<PathBuf>,
    pub owner: Option<owner::Owner>,
    pub egress: Option<BrowserEgressLease>,
    pub bridge: Option<Arc<BrowserBridgeDriver>>,
    pub presentation: Option<WindowsBrowserPresentation>,
    pub presenter: Option<PresentationClient>,
    pub io: Vec<PrivateIoLease>,
    pub drains: Vec<tokio::task::JoinHandle<std::io::Result<u64>>>,
    pub fenced: Arc<AtomicBool>,
    pub closed: bool,
    pub cef_acknowledged: bool,
    pub receipt: Option<WindowsBrowserShutdownReceipt>,
}
impl Resources {
    pub(super) async fn cleanup(&mut self) -> Result<(), BrowserDriverError> {
        self.fenced.store(true, Ordering::Release);
        if self.closed {
            return Ok(());
        }
        if let Some(bridge) = &self.bridge {
            bridge.disconnect_for_shutdown();
        }
        if let Some(presenter) = &self.presenter {
            presenter.disconnect();
        }
        if let Some(egress) = &mut self.egress {
            egress
                .revoke()
                .await
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        }
        // Native owner never leaves its thread. WFP/exemption/package retention
        // ends only after exact full-Job-zero, independent of bridge/channel EOF.
        let forced = if let Some(owner) = &self.owner {
            owner.close().await?
        } else {
            true
        };
        for drain in &mut self.drains {
            drain.abort();
            let _ = drain.await;
        }
        self.drains.clear();
        let leases = self.io.clone();
        let drained = tokio::task::spawn_blocking(move || {
            for lease in &leases {
                lease.cancel();
            }
            let mut result = true;
            for lease in leases {
                result &= lease.drain(Duration::from_secs(2));
            }
            result
        })
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if !drained {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.io.clear();
        if let Some(directory) = &self.directory {
            if self.quarantine.is_none() {
                directory
                    .revalidate()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                let original = directory.canonical_path();
                let quarantine = original.with_file_name(format!(
                    "reaped-{}",
                    original
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or(BrowserDriverError::OutcomeUnknown)?
                ));
                // Windows rename refuses an existing destination. Compare the moved
                // kernel identity before removing anything; a replacement is preserved.
                std::fs::rename(original, &quarantine)
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                self.quarantine = Some(quarantine);
            }
            let path = self
                .quarantine
                .as_ref()
                .ok_or(BrowserDriverError::OutcomeUnknown)?;
            let moved =
                BoundPath::open_directory(path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if moved.identity() != directory.identity() {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            moved
                .validate_private_owner_dacl()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            reject_reparse(path)?;
            std::fs::remove_dir_all(path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            drop(moved);
        }
        // Release pathname bindings, then retire only the exact creation handle.
        // This also covers a registered allocation whose initial bind failed.
        self.directory.take();
        self.creation
            .remove_empty()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        self.closed = true;
        self.receipt = Some(if self.cef_acknowledged && !forced {
            WindowsBrowserShutdownReceipt::GracefulCefShutdown
        } else {
            WindowsBrowserShutdownReceipt::ForcedJobExit
        });
        Ok(())
    }
}
fn reject_reparse(root: &std::path::Path) -> Result<(), BrowserDriverError> {
    let mut pending = vec![root.to_owned()];
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        let bound =
            BoundPath::open_directory(&path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        for entry in std::fs::read_dir(bound.canonical_path())
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        {
            visited += 1;
            if visited > 100_000 {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let entry = entry.map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let kind = entry
                .file_type()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else {
                BoundPath::open_file(&entry.path())
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            }
        }
    }
    Ok(())
}
pub(super) struct Driver {
    pub bridge: Arc<BrowserBridgeDriver>,
    pub resources: Arc<Mutex<Resources>>,
    pub request: BrowserDriverOpenRequest,
    pub fenced: Arc<AtomicBool>,
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
        if self.fenced.load(Ordering::Acquire)
            || command.binding != self.request.binding
            || command.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.execute(command, control).await
    }
    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || command.binding != self.request.binding
            || command.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.capture(command, control).await
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.binding != self.request.binding
            || request.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.read_screenshot_chunk(request, control).await
    }
    async fn prepare_upload(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.command.binding != self.request.binding
            || request.command.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.prepare_upload(request, control).await
    }
    async fn write_upload_chunk(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.transfer.binding != self.request.binding
            || request.transfer.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.write_upload_chunk(request, control).await
    }
    async fn commit_upload(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.binding != self.request.binding
            || request.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.commit_upload(request, control).await
    }
    async fn download(
        &self,
        request: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.binding != self.request.binding
            || request.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.download(request, control).await
    }
    async fn read_download_chunk(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.binding != self.request.binding
            || request.session_id != self.request.session_id
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.read_download_chunk(request, control).await
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if self.fenced.load(Ordering::Acquire)
            || request.binding != self.request.binding
            || request.session_id != self.request.session_id
        {
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
        let resources = Arc::clone(&self.resources);
        let bridge = Arc::clone(&self.bridge);
        let session = session.clone();
        // Independent cleanup remains alive after a dropped close future.
        tokio::spawn(async move {
            let mut resources = resources.lock().await;
            if resources.closed {
                return Ok(());
            }
            if let Some(presenter) = &resources.presenter {
                presenter.disconnect();
            }
            if let Some(egress) = &mut resources.egress {
                egress
                    .revoke()
                    .await
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            }
            resources.cef_acknowledged = bridge.close_session(&session).await.is_ok();
            resources.cleanup().await
        })
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
}
