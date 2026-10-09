/// Bounded categorical cloud service failure, without credential or payload diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum CloudError {
    /// Required project permission or enrolled node authority is absent.
    #[error("cloud authority is absent or revoked")]
    PermissionDenied,
    /// A strict field or identity binding was invalid.
    #[error("cloud request is invalid")]
    InvalidArgument,
    /// Resource is absent within the authenticated project.
    #[error("cloud resource was not found")]
    NotFound,
    /// A mutation identity, revision, or event cursor conflicts with durable state.
    #[error("cloud state conflicts with this request")]
    Conflict,
    /// A configured resource budget was exhausted.
    #[error("cloud resource bound was exceeded")]
    ResourceExhausted,
    /// Canonical storage or integrity verification is unavailable.
    #[error("cloud storage is unavailable")]
    Storage,
}

/// Result returned by project-scoped cloud services.
pub type CloudResult<T> = Result<T, CloudError>;

impl From<colossus_ports::StoreError> for CloudError {
    fn from(error: colossus_ports::StoreError) -> Self {
        match error {
            colossus_ports::StoreError::Conflict { .. } => Self::Conflict,
            _ => Self::Storage,
        }
    }
}
