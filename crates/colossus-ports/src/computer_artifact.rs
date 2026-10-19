//! Owner-only publication of actual computer PNG bytes after mandatory policy release.

use crate::EventJournal;
use async_trait::async_trait;
use colossus_contracts::{ComputerBinding, ComputerScreenshotDescriptor, ModelImageReference};
use std::sync::Arc;
use thiserror::Error;
use zeroize::Zeroizing;

/// Native-only publication. Neither ownership nor released PNG bytes are model inputs.
pub struct ComputerArtifactPublication {
    /// Registered authenticated owner, scope, and initiating run.
    pub binding: ComputerBinding,
    /// Original authorized tool-call identity.
    pub call_id: String,
    /// Exact capture metadata, independently revalidated against released bytes.
    pub descriptor: ComputerScreenshotDescriptor,
    /// Complete post-authorized PNG held privately until artifact publication.
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Categorical publication error; never includes image bytes or private native details.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum ComputerArtifactError {
    /// Ownership, capture metadata, or released bytes are invalid.
    #[error("released computer artifact invalid")]
    Invalid,
    /// Publication did not positively complete; capture must not be repeated automatically.
    #[error("computer artifact publication outcome unknown")]
    OutcomeUnknown,
}

/// Trusted host adapter for already policy-released PNGs, independent of public API crates.
#[async_trait]
pub trait ComputerArtifactPublisher: Send + Sync + std::fmt::Debug {
    /// Publish exact complete released PNG for its registered owner, returning metadata only.
    async fn publish(
        &self,
        journal: Arc<dyn EventJournal>,
        request: ComputerArtifactPublication,
    ) -> Result<ModelImageReference, ComputerArtifactError>;
}
