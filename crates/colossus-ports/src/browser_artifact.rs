//! Publication of actual browser PNG bytes only after mandatory effect release.

use crate::{BrowserScreenshotDescriptor, EventJournal};
use async_trait::async_trait;
use colossus_contracts::{BrowserSessionBinding, ModelImageReference};
use std::sync::Arc;
use thiserror::Error;
use zeroize::Zeroizing;

/// Native-only request. Neither authority nor PNG bytes are deserializable tool inputs.
pub struct BrowserArtifactPublication {
    /// Registered authenticated owner and conversation/workflow scope.
    pub binding: BrowserSessionBinding,
    /// Registered initiating run.
    pub run_id: String,
    /// Already-authorized original tool call identity, used for idempotent publication.
    pub call_id: String,
    /// Exact native capture metadata, revalidated against released bytes by the adapter.
    pub descriptor: BrowserScreenshotDescriptor,
    /// Complete post-authorized PNG, retained privately until atomic artifact publication.
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Categorical publication error; never contains image bytes or a private path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum BrowserArtifactError {
    /// Released bytes or native ownership evidence is invalid.
    #[error("released browser artifact invalid")]
    Invalid,
    /// Storage or artifact publication did not positively complete; capture cannot be retried.
    #[error("browser artifact publication outcome unknown")]
    OutcomeUnknown,
}

/// Trusted composition supplies this adapter; runtime remains independent of public API crates.
#[async_trait]
pub trait BrowserArtifactPublisher: Send + Sync + std::fmt::Debug {
    /// Authorize and resolve actual existing RunInput bytes for pre-effect inspection.
    async fn resolve_upload(
        &self,
        _journal: Arc<dyn EventJournal>,
        _request: crate::BrowserUploadArtifactRequest,
    ) -> Result<crate::BrowserUploadArtifact, BrowserArtifactError> {
        Err(BrowserArtifactError::Invalid)
    }

    /// Publish complete downloaded bytes only after mandatory post-effect release.
    async fn publish_download(
        &self,
        _journal: Arc<dyn EventJournal>,
        _request: crate::BrowserDownloadPublication,
    ) -> Result<crate::BrowserReleasedArtifact, BrowserArtifactError> {
        Err(BrowserArtifactError::Invalid)
    }
    /// Publish actual released PNG under its initiating owner, returning metadata only.
    async fn publish(
        &self,
        journal: Arc<dyn EventJournal>,
        request: BrowserArtifactPublication,
    ) -> Result<ModelImageReference, BrowserArtifactError>;
}
