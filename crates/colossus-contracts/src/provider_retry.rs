//! Safe provider transport progress released independently of model output.

use serde::{Deserialize, Serialize};

/// Current stage of bounded automatic provider recovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRetryState {
    /// Waiting until the next permitted request attempt.
    Backoff,
    /// The next request is being dispatched.
    Retrying,
    /// The provider accepted the request again.
    Recovered,
}

/// Credential-free progress for one confirmed transient HTTP response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRetry {
    /// One-based retry number, excluding the original request.
    pub attempt: u32,
    /// Maximum number of retries for the request.
    pub max_retries: u32,
    /// Confirmed transient upstream status.
    pub http_status: u32,
    /// Current recovery stage.
    pub state: ProviderRetryState,
    /// UTC RFC3339 backoff deadline; present only while waiting.
    pub retry_at: Option<String>,
}
