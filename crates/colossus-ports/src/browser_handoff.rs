//! Native-only irreversible human handoff confirmation; never a public authority input.

use colossus_contracts::{BrowserSessionBinding, BrowserSessionId, BrowserTarget};
use serde::{Deserialize, Serialize};

/// Trusted presentation receipt bound to the original human allocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserNativeHandoffRequest {
    /// Exact original native owner.
    pub binding: BrowserSessionBinding,
    /// Exact original human session.
    pub session_id: BrowserSessionId,
    /// Original coordinator document.
    pub expected_target: BrowserTarget,
    /// Native confirmed current document after irreversible presentation revocation.
    pub confirmed_target: BrowserTarget,
    /// Positive native document generation from the same retained host.
    pub native_document_generation: u64,
}
