use serde::Serialize;

/// A bounded, credential-free error safe to release to a human interface.
#[derive(Debug, Serialize)]
pub struct Failure {
    /// Stable category; uncertain mutations are distinguished from read failures.
    pub code: &'static str,
    /// Fixed user-facing text with no server body or request data.
    pub message: &'static str,
}
impl Failure {
    pub(crate) const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}
/// Result returned by the native human client.
pub type Result<T> = std::result::Result<T, Failure>;
