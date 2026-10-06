//! Explicit application-owned workspace disclosure contracts.
use crate::Run;
use serde::{Deserialize, Serialize};

/// Change only the authenticated application's sharing relationship in this workspace.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetWorkspaceSharingRequest {
    /// Independently authenticated application receiving released session access.
    pub recipient_application_id: String,
    /// Release caller-owned sessions, or revoke future disclosure.
    pub enabled: bool,
    /// Permit the recipient to continue shared sessions beneath its own grant.
    pub allow_continuation: bool,
}

/// Durable local sharing posture, independent of cloud enrollment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSharingState {
    /// Application receiving this explicit local disclosure.
    pub recipient_application_id: String,
    /// Whether new reads may disclose the source application's sessions.
    pub enabled: bool,
    /// Whether the recipient may submit its own runs in shared sessions.
    pub allow_continuation: bool,
}

/// A released run with authority derived by the authenticated runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibleRun {
    /// Released run projection, never private worker or model content.
    pub run: Run,
    /// Whether this caller owns the run's cancel/respond authority.
    pub controllable: bool,
    /// Whether this caller may continue the containing session using its own grant.
    pub continuable: bool,
}

/// Bounded source-stable page of owned and explicitly shared runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListVisibleRunsResponse {
    /// Caller-owned and explicitly shared released run projections.
    pub runs: Vec<VisibleRun>,
    /// Opaque continuation token bound to the authenticated disclosure relationships.
    pub next_page_token: Option<String>,
}
