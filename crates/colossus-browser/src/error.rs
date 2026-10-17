use colossus_ports::BrowserDriverError;
use thiserror::Error;

/// Categorical errors suitable for runtime tool translation, with no rejected input.
#[derive(Debug, Error)]
pub enum BrowserError {
    /// Requested handle is absent or belongs to another authenticated binding.
    #[error("owned browser session not found")]
    NotFound,
    /// Installed engine/mode/action is absent.
    #[error("browser capability unavailable")]
    Unavailable,
    /// Invalid bounded argument or ownership metadata.
    #[error("invalid browser arguments")]
    InvalidArguments,
    /// Destination is outside the session's immutable envelope.
    #[error("browser destination denied")]
    OriginDenied,
    /// Another run or action already owns the writer boundary.
    #[error("browser session busy")]
    Busy,
    /// Lease expired, was revoked, changed generation, or belongs to another run.
    #[error("browser control lease is stale")]
    StaleControl,
    /// Target document or snapshot is no longer current.
    #[error("browser document or element is stale")]
    StaleDocument,
    /// A mutation requires a new current snapshot after attachment or takeover.
    #[error("browser requires a fresh snapshot")]
    SnapshotRequired,
    /// Runtime/session limits prevent allocation or result acceptance.
    #[error("browser limit exceeded")]
    LimitExceeded,
    /// Request cancellation was observed before dispatch.
    #[error("browser operation cancelled")]
    Cancelled,
    /// Driver protocol identity/bounds failed validation; session dispatch is blocked.
    #[error("browser driver returned invalid evidence")]
    InvalidEvidence,
    /// Native response, external mutation, or cleanup cannot be proven.
    #[error("browser outcome unknown; automatic retry is prohibited")]
    OutcomeUnknown,
    /// Known native terminal error.
    #[error(transparent)]
    Driver(#[from] BrowserDriverError),
}
