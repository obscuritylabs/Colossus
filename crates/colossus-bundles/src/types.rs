use super::*;

/// Publisher to key-id to base64 Ed25519 public-key bindings loaded from strict config.
pub type BundleTrustStore = BTreeMap<String, BTreeMap<String, String>>;

/// Offline release-bundle contract failure.
#[derive(Debug, Error)]
pub enum BundleError {
    /// A filesystem operation failed.
    #[error("bundle filesystem failure: {0}")]
    Io(#[from] std::io::Error),
    /// A manifest could not be encoded or decoded.
    #[error("bundle manifest failure: {0}")]
    Json(#[from] serde_json::Error),
    /// The bundle violated its format, integrity, or trust contract.
    #[error("bundle verification failed: {0}")]
    Invalid(String),
}

/// Gateway-routed release-bundle operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum BundleOperation {
    /// Verify a staged release-bundle directory.
    Verify {
        /// Bundle directory to verify.
        path: String,
    },
    /// Build and sign a release bundle.
    Build {
        /// Staged release payload directory.
        source: String,
        /// New bundle directory to create.
        destination: String,
        /// Bundle name.
        name: String,
        /// Release version.
        version: String,
        /// Publisher identity bound by configuration.
        publisher: String,
        /// RFC 3339 creation timestamp.
        created_at: String,
        /// Optional source revision.
        source_revision: Option<String>,
        /// Reference used to resolve the signing seed at execution time.
        signing_key_reference: String,
    },
    /// Install the current platform artifact from a verified bundle.
    Install {
        /// Bundle directory to install.
        path: String,
        /// Installation prefix.
        prefix: String,
    },
    /// Inspect the public identity of a referenced signing key.
    KeyInfo {
        /// Reference used to resolve the signing seed at execution time.
        signing_key_reference: String,
    },
}

impl BundleOperation {
    /// Return the policy action authorized for this operation.
    #[must_use]
    pub fn action(&self) -> &'static str {
        match self {
            Self::Verify { .. } => "bundle.verify",
            Self::Build { .. } => "bundle.build",
            Self::Install { .. } => "bundle.install",
            Self::KeyInfo { .. } => "bundle.key.inspect",
        }
    }

    /// Return the policy resource authorized for this operation.
    #[must_use]
    pub fn resource(&self) -> String {
        match self {
            Self::Verify { path } => format!("bundle-source:{path}"),
            Self::Build { destination, .. } => format!("bundle-destination:{destination}"),
            Self::Install { path, prefix } => {
                format!("bundle-source:{path}:install-prefix:{prefix}")
            }
            Self::KeyInfo { .. } => "bundle-signing-key:referenced".into(),
        }
    }
}

/// Native artifact target expected by this running executable.
pub fn current_release_target() -> Result<&'static str, BundleError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, arch) => Err(BundleError::Invalid(format!(
            "no native release target is defined for {os}/{arch}"
        ))),
    }
}
