//! Private bounded browser file custody. No caller supplies a native pathname.

use crate::{BrowserDriverCommand, BrowserScreenshotChunk, BrowserScreenshotReadRequest};
use colossus_contracts::{BrowserOrigin, BrowserSessionBinding, BrowserSessionId, BrowserTarget};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Complete upload or download ceiling, independently enforced at every boundary.
pub const MAX_BROWSER_TRANSFER_BYTES: u32 = 4 * 1024 * 1024;

/// Verified existing artifact metadata. The display name is never a native path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserUploadDescriptor {
    /// Existing owner-bound RunInput or RunOutput artifact, resolved by trusted composition.
    pub artifact_id: String,
    /// Complete expected byte length.
    pub size_bytes: u32,
    /// Complete lowercase SHA-256.
    pub sha256: String,
    /// Safe display name used only to preserve a bounded ordinary extension.
    pub file_name: String,
}

/// Native-generated one-shot upload identity and next exact ordered offset.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserUploadReceipt {
    /// Random 128-bit lowercase hexadecimal token, never a tool argument.
    pub transfer_id: String,
    /// Exact next byte offset; commit is admitted only after the complete input.
    pub next_offset: u32,
}

/// One independently authenticated upload chunk; authority repeats the entire owner.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserUploadWriteRequest {
    /// Exact enrollment/run/session/tab/document/generation and ordered offset.
    pub transfer: BrowserScreenshotReadRequest,
    /// Canonical base64 of at most 64 KiB, never printable debugging content.
    pub data_base64: String,
}
impl std::fmt::Debug for BrowserUploadWriteRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserUploadWriteRequest")
            .field("offset", &self.transfer.offset)
            .field("encoded_length", &self.data_base64.len())
            .finish_non_exhaustive()
    }
}
impl Drop for BrowserUploadWriteRequest {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.data_base64.zeroize();
    }
}

/// Exact-generation complete download, held privately pending actual-byte policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDownloadDescriptor {
    /// Exact enrolled session.
    pub session_id: BrowserSessionId,
    /// Exact document that admitted the download.
    pub target: BrowserTarget,
    /// Current run ownership generation.
    pub control_generation: u64,
    /// Native one-shot transfer token.
    pub transfer_id: String,
    /// Complete expected bytes, no more than four MiB.
    pub size_bytes: u32,
    /// Complete lowercase SHA-256.
    pub sha256: String,
    /// Native-validated final HTTP(S) origin; every redirect remains independently checked.
    pub origin: BrowserOrigin,
}

/// Same bounded, ordered private bytes as screenshot transport; no native filename.
pub type BrowserDownloadChunk = BrowserScreenshotChunk;
/// Exact owner/generation and ordered offset for one download read.
pub type BrowserDownloadReadRequest = BrowserScreenshotReadRequest;
/// Exact owner/generation, completed offset and token for one irreversible commit.
pub type BrowserUploadCommitRequest = BrowserScreenshotReadRequest;

/// Native-owned custody resolved from an already available application artifact.
/// This type cannot be deserialized as model or renderer authority.
pub struct BrowserUploadArtifact {
    /// Independently verified authoritative artifact metadata.
    pub descriptor: BrowserUploadDescriptor,
    /// Complete bytes to be inspected by pre-effect policy before native commit.
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Resolve only the exact registered initiating application, never a nominated owner.
pub struct BrowserUploadArtifactRequest {
    /// Registered native ownership.
    pub binding: BrowserSessionBinding,
    /// Registered initiating run.
    pub run_id: String,
    /// Existing opaque RunInput or RunOutput artifact.
    pub artifact_id: String,
}

/// Already released downloaded bytes. Artifact storage must not run before release.
pub struct BrowserDownloadPublication {
    /// Registered native owner and scope.
    pub binding: BrowserSessionBinding,
    /// Initiating registered run.
    pub run_id: String,
    /// Exact authorized call, for idempotent publication.
    pub call_id: String,
    /// Full native download custody receipt.
    pub descriptor: BrowserDownloadDescriptor,
    /// Actual complete bytes that passed post-effect inspection.
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Safe released binary artifact metadata; no transfer, path or raw content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserReleasedArtifact {
    /// Opaque owner-bound artifact identity.
    pub artifact_id: String,
    /// Fixed display name, never a native filesystem path.
    pub file_name: String,
    /// Fixed application/octet-stream; page MIME claims do not confer trust.
    pub media_type: String,
    /// Complete released bytes.
    pub size_bytes: u64,
    /// Independently verified complete content digest.
    pub sha256: String,
}

/// Exact native command and independently resolved metadata for private upload staging.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserUploadPrepareRequest {
    /// Coordinator-admitted fresh file-input action.
    pub command: BrowserDriverCommand,
    /// Independently verified application artifact metadata.
    pub descriptor: BrowserUploadDescriptor,
}
