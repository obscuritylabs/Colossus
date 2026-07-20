use colossus_api::ApiError;
use std::path::PathBuf;
use thiserror::Error;

/// Result returned by Rust SDK lifecycle and transport operations.
pub type SdkResult<T> = Result<T, SdkError>;

/// Stable SDK-side failure classification.
///
/// Display strings are intentionally generic. Transport implementations should put
/// bounded, secret-free diagnostics in their own protected logs and correlation data.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SdkError {
    /// SDK configuration violates a local safety invariant.
    #[error("invalid SDK configuration: {0}")]
    InvalidConfiguration(&'static str),
    /// A path required by a local application is not absolute.
    #[error("SDK path must be absolute: {0}")]
    PathNotAbsolute(PathBuf),
    /// No verified daemon endpoint exists for the requested instance.
    #[error("Colossus instance is unavailable")]
    Unavailable,
    /// A daemon endpoint or launch lease is live but currently unavailable.
    #[error("Colossus instance is busy")]
    Busy,
    /// The server did not authenticate with the expected application credential.
    #[error("Colossus API authentication failed")]
    Authentication,
    /// The endpoint identity, instance identity, or TLS pin did not match.
    #[error("Colossus endpoint identity did not match")]
    IdentityMismatch,
    /// The server and client do not share a supported public API major.
    #[error("Colossus API version is incompatible")]
    VersionMismatch,
    /// A verified daemon could not be started.
    #[error("Colossus daemon launch failed")]
    LaunchFailed,
    /// An isolated sidecar could not be bootstrapped or supervised.
    #[error("Colossus sidecar launch failed")]
    SidecarFailed,
    /// An isolated embedded runtime could not acquire its writer lease.
    #[error("Colossus embedded runtime could not be opened")]
    EmbeddedOpenFailed,
    /// A transport failed without disclosing private response bytes.
    #[error("Colossus transport failed")]
    Transport,
    /// Closing the selected backend failed.
    #[error("Colossus backend did not close cleanly")]
    CloseFailed,
    /// An effect may have started and must not be retried automatically.
    #[error("Colossus operation outcome is unknown")]
    OutcomeUnknown,
    /// The public application API rejected the request.
    #[error(transparent)]
    Api(#[from] ApiError),
}
