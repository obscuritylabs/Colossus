use thiserror::Error;

/// Failure at the isolated Windows native boundary.
#[derive(Debug, Error)]
pub enum WindowsNativeError {
    /// The API was called on a non-Windows build.
    #[error("Windows native integration is unavailable on this platform")]
    UnsupportedPlatform,
    /// A path, prompt, or hard bound was invalid.
    #[error("Windows native input is invalid")]
    InvalidInput,
    /// A path contained a link, junction, or another reparse point.
    #[error("Windows path contains a reparse point")]
    ReparsePoint,
    /// A retained path no longer identifies the same filesystem object.
    #[error("Windows filesystem object identity changed")]
    IdentityChanged,
    /// The object owner or DACL grants broader access than private app storage permits.
    #[error("Windows filesystem permissions are not private")]
    UnsafePermissions,
    /// The native credential prompt was cancelled.
    #[error("Windows credential prompt was cancelled")]
    Cancelled,
    /// A bounded operating-system operation failed.
    #[error("Windows native {operation} failed: {source}")]
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Captured operating-system error.
        #[source]
        source: std::io::Error,
    },
}
