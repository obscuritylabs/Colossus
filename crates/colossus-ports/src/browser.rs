//! Private typed browser engine boundary. Ordinary runtime permits never enter the driver.

use crate::RunControl;
use async_trait::async_trait;
use colossus_contracts::{
    BrowserAction, BrowserCapabilities, BrowserDocumentId, BrowserObservation, BrowserOpenOptions,
    BrowserSessionBinding, BrowserSessionId, BrowserSnapshotId, BrowserTabId, BrowserTabSummary,
    BrowserTarget,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Cooperative cancellation for request lifetime and independently revocable writer authority.
#[derive(Clone)]
pub struct BrowserDriverControl {
    request: RunControl,
    authority: RunControl,
}

impl BrowserDriverControl {
    /// Construct from trusted coordinator cancellation signals.
    pub fn new(request: RunControl, authority: RunControl) -> Self {
        Self { request, authority }
    }

    /// Drivers must check this before dispatch and while awaiting native responses.
    pub fn is_cancelled(&self) -> bool {
        self.request.is_cancelled() || self.authority.is_cancelled()
    }
}

/// Coordinator-generated allocation. Engines retain all native identifiers privately.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDriverOpenRequest {
    /// Trusted immutable ownership, including conversation/workflow scope.
    pub binding: BrowserSessionBinding,
    /// Runtime-derived run provenance; absent only for a human-owned allocation.
    pub run_id: Option<String>,
    /// Exact caller-owned session.
    pub session_id: BrowserSessionId,
    /// Coordinator-generated first tab.
    pub tab_id: BrowserTabId,
    /// Coordinator-generated first document.
    pub document_id: BrowserDocumentId,
    /// Validated immutable destination envelope and explicit placement.
    pub options: BrowserOpenOptions,
}

/// One already-authorized typed dispatch; a native process bridge additionally needs its ticket.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDriverCommand {
    /// Trusted immutable session owner, repeated independently of opaque handles.
    pub binding: BrowserSessionBinding,
    /// Current authenticated writer, bound to this exact control generation.
    pub run_id: String,
    /// Exact owned session.
    pub session_id: BrowserSessionId,
    /// Exact current opaque target.
    pub target: BrowserTarget,
    /// Ownership generation; native adapters invalidate pending responses after changes.
    pub control_generation: u64,
    /// Reviewed typed operation; raw CDP and arbitrary JavaScript are absent.
    pub action: BrowserAction,
    /// Reserved identity to use if this operation commits a different document.
    pub next_document_id: BrowserDocumentId,
    /// Issuing identity for a snapshot, absent for other operations.
    pub snapshot_id: Option<BrowserSnapshotId>,
    /// Reserved new tab for TabOpen; absent for other operations.
    pub new_tab: Option<BrowserTabSummary>,
}

/// Categorical driver errors contain no URLs, protocol messages, certificates, or keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum BrowserDriverError {
    /// Verified engine or native dependency is absent.
    #[error("verified browser component unavailable")]
    Unavailable,
    /// Operation is not proved on the installed component.
    #[error("browser operation unsupported")]
    Unsupported,
    /// Native egress, credential-target, or guest boundary rejected an operation.
    #[error("browser native boundary denied operation")]
    Denied,
    /// Native target changed before dispatch.
    #[error("browser target is stale")]
    Stale,
    /// Cancellation was observed before any external effect.
    #[error("browser operation cancelled")]
    Cancelled,
    /// Bounded response or artifact ceiling was exceeded.
    #[error("browser response exceeded bounds")]
    LimitExceeded,
    /// Server trust or client-identity provisioning failed without exposing private material.
    #[error("browser certificate configuration required")]
    AuthenticationRequired,
    /// Host failure with a known no-effect outcome.
    #[error("browser native host failed")]
    Failed,
    /// Effect or cleanup may have occurred; never retry automatically.
    #[error("browser outcome unknown")]
    OutcomeUnknown,
}

/// Installed driver's acknowledged cancellation behavior; selected by native composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserCancelDisposition {
    /// The driver proves quiescence while preserving a context that can receive a new lease.
    RetainsContext,
    /// Quiescence destroys the native context. A fresh session and document are required.
    ClosesContext,
}

/// Supervised engine adapter, injected only by runtime composition.
///
/// Implementations enforce the complete browser egress envelope, guest isolation,
/// certificate selection, native target/document mapping, bounded protocol results,
/// and cancellation. They do not authorize actions or release observations. A driver
/// must fail closed if its installed component cannot prove these guarantees.
#[async_trait]
pub trait BrowserDriver: Send + Sync {
    /// Installed-artifact capability evidence; availability is never inferred from system Chrome.
    fn capabilities(&self) -> BrowserCapabilities;

    /// Describe what a successful cancellation acknowledgement means for page lifetime.
    /// Drivers that reap their host must override this so stale pages cannot be reattached.
    fn cancellation_disposition(&self) -> BrowserCancelDisposition {
        BrowserCancelDisposition::RetainsContext
    }

    /// Allocate isolated browser context and initial tab using only supplied opaque identities.
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError>;

    /// Execute exactly one typed action and return bounded staged evidence.
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError>;

    /// Capture a PNG into an exact-generation private transfer; never release its bytes here.
    async fn capture(
        &self,
        _command: BrowserDriverCommand,
        _control: &BrowserDriverControl,
    ) -> Result<crate::BrowserScreenshotDescriptor, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Read the next bounded chunk of an exact owned capture; no automatic replay is safe.
    async fn read_screenshot_chunk(
        &self,
        _request: crate::BrowserScreenshotReadRequest,
        _control: &BrowserDriverControl,
    ) -> Result<crate::BrowserScreenshotChunk, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Begin one private upload after actual input bytes received pre-effect authorization.
    async fn prepare_upload(
        &self,
        _request: crate::BrowserUploadPrepareRequest,
        _control: &BrowserDriverControl,
    ) -> Result<crate::BrowserUploadReceipt, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Stage the next exact ordered chunk; this does not commit website input.
    async fn write_upload_chunk(
        &self,
        _request: crate::BrowserUploadWriteRequest,
        _control: &BrowserDriverControl,
    ) -> Result<crate::BrowserUploadReceipt, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Commit only the complete hash-verified upload to the originally admitted file input.
    async fn commit_upload(
        &self,
        _request: crate::BrowserUploadCommitRequest,
        _control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Admit one exact current-element download into privately retained native custody.
    async fn download(
        &self,
        _command: BrowserDriverCommand,
        _control: &BrowserDriverControl,
    ) -> Result<crate::BrowserDownloadDescriptor, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Consume one bounded chunk; only the Runtime gateway may release complete bytes.
    async fn read_download_chunk(
        &self,
        _request: crate::BrowserDownloadReadRequest,
        _control: &BrowserDriverControl,
    ) -> Result<crate::BrowserDownloadChunk, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Confirm an irreversible native handoff without granting a run or control generation.
    async fn confirm_native_handoff(
        &self,
        _request: crate::BrowserNativeHandoffRequest,
        _control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }

    /// Cancel pending dispatch and quiesce session background traffic; never renew authority.
    async fn cancel_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError>;

    /// Reap context/processes and release its profile; idempotent when already closed.
    async fn close_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError>;
}
