use std::{
    fs::File,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use colossus_browser_bridge::BrowserBridgeDriver;
use colossus_contracts::{
    BrowserCapabilities, BrowserObservation, BrowserSessionBinding, BrowserSessionId,
    BrowserTabSummary,
};
use colossus_home::ConfinedRoot;
use colossus_ports::{
    BrowserCancelDisposition, BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserDriver,
    BrowserDriverCommand, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
    BrowserNativeHandoffRequest, BrowserScreenshotChunk, BrowserScreenshotDescriptor,
    BrowserScreenshotReadRequest, BrowserUploadCommitRequest, BrowserUploadPrepareRequest,
    BrowserUploadReceipt, BrowserUploadWriteRequest,
};
use tokio::sync::Mutex;

use super::process_identity::ProcessIdentity;
use super::{Installation, engine, relay::Relay};
use crate::BrowserEgressLease;

/// Native-only cleanup evidence for an exactly enrolled dedicated process tree.
/// Physical reaping can prove authority is gone after a crash, but cannot attest
/// that the compiled CEF shutdown path completed normally.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OciBrowserShutdownReceipt {
    /// No container create was attempted.
    NoProcess,
    /// Native Close ACK, channel EOF, pinned process exit, and exact Docker exit 0.
    GracefulCefShutdown,
    /// Exact owned container/process removal and egress drain; native exit unknown.
    ForcedReap,
    /// An unresolved process, daemon allocation, relay or artifact obligation remains.
    Unknown,
}

pub(super) struct Resources {
    pub(super) installation: Arc<Installation>,
    pub(super) binding: BrowserSessionBinding,
    pub(super) request: BrowserDriverOpenRequest,
    pub(super) name: String,
    pub(super) nonce: String,
    pub(super) control: ConfinedRoot,
    pub(super) control_handle: File,
    pub(super) quarantine: Option<PathBuf>,
    pub(super) control_removed: bool,
    pub(super) container: Option<String>,
    pub(super) create_attempted: bool,
    pub(super) process: Option<Arc<ProcessIdentity>>,
    pub(super) egress: Option<BrowserEgressLease>,
    pub(super) relay: Option<Relay>,
    pub(super) pki: Option<super::pki::StagedPki>,
    pub(super) profile: Option<crate::BrowserProfileLease>,
    pub(super) presentation: Option<(colossus_browser_presentation::PresentationClient, [u8; 32])>,
    pub(super) closed: bool,
    pub(super) counted: bool,
    pub(super) fenced: Arc<AtomicBool>,
    pub(super) startup_phase: &'static str,
    pub(super) shutdown_receipt: OciBrowserShutdownReceipt,
}

impl Resources {
    pub(super) async fn cleanup(&mut self) -> Result<(), BrowserDriverError> {
        if self.closed {
            return Ok(());
        }
        self.fenced.store(true, Ordering::Release);
        if let Some((presentation, _)) = &self.presentation {
            presentation.disconnect();
        }
        // Stop both sides of the relay independently of the nativehost's health.
        let relay = match self.relay.as_mut() {
            Some(relay) => relay.revoke().await,
            None => Ok(()),
        };
        let egress = match self.egress.as_mut() {
            Some(egress) => egress
                .revoke()
                .await
                .map_err(|_| BrowserDriverError::OutcomeUnknown),
            None => Ok(()),
        };
        if self.container.is_none() && self.create_attempted {
            // A timed-out create can still complete inside the daemon after the
            // CLI exits. An immediate empty inspect/list cannot prove no later
            // allocation. Keep the exact intent until its owned CID is found.
            let inspected = engine::inspect(&self.installation, &self.name).await?;
            self.reconcile_creation(inspected)?;
        }
        if let Some(identity) = &self.container
            && let Some(inspected) = engine::inspect(&self.installation, identity).await?
        {
            self.validate_owned(&inspected)?;
            // Exact retained CID only; cleanup never destroys a nominated name.
            let mut arguments = vec!["container".into(), "rm".into()];
            if inspected.running {
                arguments.push("--force".into());
            }
            arguments.push(identity.clone());
            let _ = engine::run(&self.installation, &arguments).await?;
            if engine::inspect(&self.installation, identity)
                .await?
                .is_some()
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        relay?;
        egress?;
        if self
            .process
            .as_ref()
            .is_some_and(|process| process.exited() != Ok(true))
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        if !self.control_removed {
            // Unknown input/quarantine ownership preserves the entire private control
            // root after physical process and egress reaping. It cannot be recursively
            // pruned merely because the daemon no longer has a live container.
            if let Some(pki) = self.pki.as_mut() {
                pki.release_inputs()?;
            }
            if self.quarantine.is_none() {
                self.control
                    .revalidate()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                let original = self.control.path().to_owned();
                let quarantine = self
                    .installation
                    .root
                    .path()
                    .join(format!("reaped-{}", self.nonce));
                // Detach the name, then compare the moved inode with the retained fd.
                // A raced replacement is preserved, rather than recursively deleted.
                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    &original,
                    rustix::fs::CWD,
                    &quarantine,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                self.quarantine = Some(quarantine);
            }
            remove_quarantined(
                &self.control_handle,
                self.quarantine
                    .as_ref()
                    .ok_or(BrowserDriverError::OutcomeUnknown)?,
            )?;
            self.control_removed = true;
        }
        if let Some(profile) = self.profile.as_mut() {
            // Every native process and both egress directions are already gone.
            // A known pre-create failure never exposed the cache to Chromium;
            // otherwise only the positively observed normal CEF exit is clean.
            profile
                .retire(
                    !self.create_attempted
                        || self.shutdown_receipt == OciBrowserShutdownReceipt::GracefulCefShutdown,
                )
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        }
        self.closed = true;
        if self.shutdown_receipt != OciBrowserShutdownReceipt::GracefulCefShutdown {
            self.shutdown_receipt = if self.create_attempted {
                OciBrowserShutdownReceipt::ForcedReap
            } else {
                OciBrowserShutdownReceipt::NoProcess
            };
        }
        if self.counted {
            self.installation.artifacts.release();
            self.counted = false;
        }
        Ok(())
    }

    pub(super) fn validate_owned(
        &self,
        inspected: &engine::Inspection,
    ) -> Result<(), BrowserDriverError> {
        if !engine::container_id(&inspected.id)
            || inspected.label != self.nonce
            || inspected.image != self.installation.image
            || inspected.network != "none"
            || inspected.user != format!("{}:{}", self.installation.uid, self.installation.gid)
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(())
    }

    pub(super) fn reconcile_creation(
        &mut self,
        inspected: Option<engine::Inspection>,
    ) -> Result<(), BrowserDriverError> {
        let inspected = inspected.ok_or(BrowserDriverError::OutcomeUnknown)?;
        self.validate_owned(&inspected)?;
        self.container = Some(inspected.id);
        Ok(())
    }
}

fn remove_quarantined(retained: &File, quarantine: &PathBuf) -> Result<(), BrowserDriverError> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata =
        std::fs::symlink_metadata(quarantine).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let owned = retained
        .metadata()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.dev() != owned.dev()
        || metadata.ino() != owned.ino()
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    std::fs::remove_dir_all(quarantine).map_err(|_| BrowserDriverError::OutcomeUnknown)
}

pub(super) struct Driver {
    pub(super) bridge: Arc<BrowserBridgeDriver>,
    pub(super) resources: Arc<Mutex<Resources>>,
    pub(super) fenced: Arc<AtomicBool>,
    pub(super) binding: BrowserSessionBinding,
    pub(super) session: BrowserSessionId,
    pub(super) options: colossus_contracts::BrowserOpenOptions,
    pub(super) request: BrowserDriverOpenRequest,
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
        if self.fenced.load(Ordering::Acquire)
            || request.binding != self.binding
            || request.session_id != self.session
            || request.options != self.options
            || request != self.request
        {
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
            || command.binding != self.binding
            || command.session_id != self.session
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
            || command.binding != self.binding
            || command.session_id != self.session
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
            || request.binding != self.binding
            || request.session_id != self.session
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
            || request.command.binding != self.binding
            || request.command.session_id != self.session
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
            || request.transfer.binding != self.binding
            || request.transfer.session_id != self.session
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
            || request.binding != self.binding
            || request.session_id != self.session
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
            || request.binding != self.binding
            || request.session_id != self.session
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
            || request.binding != self.binding
            || request.session_id != self.session
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
            || request.binding != self.binding
            || request.session_id != self.session
        {
            return Err(BrowserDriverError::Denied);
        }
        self.bridge.confirm_native_handoff(request, control).await
    }
    async fn cancel_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.close_session(session).await
    }
    async fn close_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        if session != &self.session {
            return Err(BrowserDriverError::Denied);
        }
        self.fenced.store(true, Ordering::Release);
        let mut resources = self.resources.lock().await;
        if resources.closed {
            return Ok(());
        }
        if let Some((presentation, _)) = &resources.presentation {
            presentation.disconnect();
        }
        // A private native acknowledgement is necessary for normal shutdown, but
        // physical container reaping plus proxy drain independently proves cleanup
        // after a crash/disconnect. Neither path retries a typed browser action.
        let native_acknowledged = self.bridge.close_session(session).await.is_ok();
        self.bridge.disconnect_for_shutdown();
        if let Some(process) = &resources.process {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while process.exited() == Ok(false) && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        let mut graceful = false;
        if native_acknowledged
            && resources
                .process
                .as_ref()
                .is_some_and(|process| process.exited() == Ok(true))
            && let Some(identity) = resources.container.as_ref()
            && let Some(inspected) = engine::inspect(&resources.installation, identity).await?
        {
            resources.validate_owned(&inspected)?;
            if !inspected.running && inspected.exit_code == 0 && !inspected.oom_killed {
                graceful = true;
            }
        }
        if graceful {
            resources.shutdown_receipt = OciBrowserShutdownReceipt::GracefulCefShutdown;
        }
        resources.cleanup().await?;
        Ok(())
    }
}
