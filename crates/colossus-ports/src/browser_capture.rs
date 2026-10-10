//! Private bounded screenshot transfer. No bytes, native path, or endpoint is a tool argument.

use colossus_contracts::{BrowserSessionBinding, BrowserSessionId, BrowserTarget};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Complete screenshot ceiling before post-effect release.
pub const MAX_BROWSER_SCREENSHOT_BYTES: u32 = 4 * 1024 * 1024;
/// Raw chunk ceiling; its base64 representation fits the private 128 KiB bridge frame.
pub const BROWSER_SCREENSHOT_CHUNK_BYTES: usize = 64 * 1024;

/// Native capture receipt; the opaque transfer is usable only by the exact writer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserScreenshotDescriptor {
    /// Exact owned session.
    pub session_id: BrowserSessionId,
    /// Captured current document.
    pub target: BrowserTarget,
    /// Current writer generation.
    pub control_generation: u64,
    /// Random native-only 32 lowercase hexadecimal transfer identity.
    pub transfer_id: String,
    /// Complete verified PNG byte count.
    pub size_bytes: u32,
    /// Lowercase SHA-256 of complete PNG bytes.
    pub sha256: String,
    /// Physical PNG width.
    pub width: u32,
    /// Physical PNG height.
    pub height: u32,
}

/// Coordinator-owned request for exactly the next private screenshot chunk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserScreenshotReadRequest {
    /// Full native admission binding, never nominated by model arguments.
    pub binding: BrowserSessionBinding,
    /// Authenticated writer run.
    pub run_id: String,
    /// Exact owned session.
    pub session_id: BrowserSessionId,
    /// Exact captured document.
    pub target: BrowserTarget,
    /// Exact captured control generation.
    pub control_generation: u64,
    /// Capture receipt's private transfer identity.
    pub transfer_id: String,
    /// Strictly ordered zero-based byte offset.
    pub offset: u32,
}

/// Bounded private transport chunk. The encoded content is never released as tool JSON.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserScreenshotChunk {
    /// Exact zero-based byte offset.
    pub offset: u32,
    /// Canonical base64 of at most 64 KiB raw PNG bytes.
    pub data_base64: String,
}

impl std::fmt::Debug for BrowserScreenshotChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserScreenshotChunk")
            .field("offset", &self.offset)
            .field("encoded_bytes", &self.data_base64.len())
            .finish()
    }
}

impl Drop for BrowserScreenshotChunk {
    fn drop(&mut self) {
        self.data_base64.zeroize();
    }
}
