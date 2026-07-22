use super::*;

pub(super) const PACK_MANIFEST: &str = "colossus.pack.json";
pub(super) const BUNDLE_MANIFEST: &str = "manifest.json";
pub(super) const COLLECTION_MANIFEST: &str = "colossus.collection.json";
pub(super) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(super) const MAX_FILES: usize = 10_000;
pub(super) const MAX_PACK_SKILL_REFERENCES: usize = 64;
pub(super) const MAX_TEXT_BYTES: usize = 8 * 1024;
pub(super) const MAX_ARCHIVE_BYTES: u64 = MAX_TOTAL_BYTES;
pub(super) const RELEASE_TARGETS: [&str; 6] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

pub(super) const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub(super) const OCI_TAR_MEDIA_TYPES: [&str; 2] = [
    "application/vnd.colossus.pack.v1.tar",
    "application/vnd.oci.image.layer.v1.tar",
];
pub(super) const OCI_GZIP_MEDIA_TYPES: [&str; 2] = [
    "application/vnd.colossus.pack.v1.tar+gzip",
    "application/vnd.oci.image.layer.v1.tar+gzip",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OciLayout {
    pub(super) image_layout_version: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OciDescriptor {
    pub(super) media_type: String,
    pub(super) digest: String,
    pub(super) size: u64,
    #[serde(default)]
    pub(super) annotations: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) urls: Vec<String>,
    #[serde(default)]
    pub(super) platform: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OciIndex {
    pub(super) schema_version: u16,
    pub(super) manifests: Vec<OciDescriptor>,
    #[serde(default)]
    pub(super) media_type: Option<String>,
    #[serde(default)]
    pub(super) artifact_type: Option<String>,
    #[serde(default)]
    pub(super) annotations: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OciManifest {
    pub(super) schema_version: u16,
    pub(super) layers: Vec<OciDescriptor>,
    #[serde(default)]
    pub(super) media_type: Option<String>,
    #[serde(default)]
    pub(super) artifact_type: Option<String>,
    #[serde(default)]
    pub(super) config: Option<OciDescriptor>,
    #[serde(default)]
    pub(super) subject: Option<OciDescriptor>,
    #[serde(default)]
    pub(super) annotations: BTreeMap<String, String>,
}

pub(super) struct MaterializedPack {
    pub(super) root: PathBuf,
    pub(super) _temporary: Option<tempfile::TempDir>,
}

/// Pack or offline-bundle contract failure.
#[derive(Debug, Error)]
pub enum PackError {
    /// Filesystem operation failed.
    #[error("pack filesystem failure: {0}")]
    Io(#[from] std::io::Error),
    /// Strict JSON parsing or serialization failed.
    #[error("pack manifest failure: {0}")]
    Json(#[from] serde_json::Error),
    /// A security or schema invariant was violated.
    #[error("pack verification failed: {0}")]
    Invalid(String),
    /// A remote mutation may have completed and must not be retried implicitly.
    #[error("pack remote outcome is unknown: {0}")]
    OutcomeUnknown(String),
    /// Durable lifecycle state failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Gateway-routed pack or bundle operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackOperation {
    /// Verify and validate a local pack directory.
    Verify {
        /// Absolute or workspace-relative pack directory.
        path: String,
    },
    /// Install a verified local pack.
    Install {
        /// Absolute or workspace-relative pack directory.
        path: String,
        /// Explicit development override for an unsigned pack.
        allow_untrusted: bool,
    },
    /// Reverify and activate an installed pack.
    Enable {
        /// Installed pack name.
        name: String,
    },
    /// Deactivate an installed pack without deleting bytes.
    Disable {
        /// Installed pack name.
        name: String,
    },
    /// Deactivate and remove installed bytes.
    Uninstall {
        /// Installed pack name.
        name: String,
    },
    /// Bind a publisher identity to one Ed25519 public key.
    TrustAdd {
        /// Publisher identity to bind.
        publisher: String,
        /// Base64-encoded 32-byte Ed25519 public key.
        public_key: String,
    },
    /// Verify a signed offline release bundle without network access.
    BundleVerify {
        /// Absolute or workspace-relative bundle directory.
        path: String,
    },
    /// Materialize and sign an offline release bundle from a staged payload tree.
    BundleBuild {
        /// Absolute or workspace-relative staged payload directory.
        source: String,
        /// Absolute or workspace-relative destination directory, which must not exist.
        destination: String,
        /// Stable bundle identity.
        name: String,
        /// Release version represented by the payload.
        version: String,
        /// Publisher already bound to the signing key in canonical trust state.
        publisher: String,
        /// Explicit reproducible RFC3339 UTC timestamp.
        created_at: String,
        /// Optional source revision.
        source_revision: Option<String>,
        /// Environment reference containing a 32-byte Ed25519 signing seed.
        signing_key_reference: String,
    },
    /// Verify and install the current-target native executable from an offline bundle.
    BundleInstall {
        /// Absolute or workspace-relative bundle directory.
        path: String,
        /// Absolute clean installation prefix.
        prefix: String,
    },
    /// Derive the safe public identity for a referenced signing seed.
    BundleKeyInfo {
        /// Environment reference containing a 32-byte Ed25519 signing seed.
        signing_key_reference: String,
    },
    /// Verify a signed multi-pack and skill collection without installing it.
    CollectionVerify {
        /// Absolute or workspace-relative collection directory.
        path: String,
    },
    /// Build and sign a deterministic collection from `packs/` and `skills/` trees.
    CollectionBuild {
        /// Absolute or workspace-relative staged collection payload.
        source: String,
        /// Absolute or workspace-relative destination, which must not exist.
        destination: String,
        /// Stable collection identity.
        name: String,
        /// Immutable collection version.
        version: String,
        /// Publisher already bound to the signing key in canonical trust state.
        publisher: String,
        /// Explicit reproducible RFC3339 UTC timestamp.
        created_at: String,
        /// Environment reference containing a 32-byte Ed25519 signing seed.
        signing_key_reference: String,
    },
    /// Verify and install every pack and skill from a signed collection without clobbering.
    CollectionInstall {
        /// Absolute or workspace-relative collection directory.
        path: String,
    },
    /// Pull an authenticated signed collection transport into a clean local directory.
    RegistryPull {
        /// Credential-free HTTPS URL or explicit loopback HTTP URL.
        url: String,
        /// Absolute clean destination directory.
        destination: String,
        /// Optional environment-backed bearer credential reference.
        credential_reference: Option<String>,
    },
    /// Push a verified collection as a deterministic create-only tar transport.
    RegistryPush {
        /// Absolute local collection directory.
        path: String,
        /// Credential-free HTTPS URL or explicit loopback HTTP URL.
        url: String,
        /// Optional environment-backed bearer credential reference.
        credential_reference: Option<String>,
    },
}

impl PackOperation {
    /// Exact policy action for this operation.
    pub fn action(&self) -> &'static str {
        match self {
            Self::Verify { .. } => "pack.verify",
            Self::Install { .. } => "pack.install",
            Self::Enable { .. } => "pack.enable",
            Self::Disable { .. } => "pack.disable",
            Self::Uninstall { .. } => "pack.uninstall",
            Self::TrustAdd { .. } => "pack.trust.add",
            Self::BundleVerify { .. } => "bundle.verify",
            Self::BundleBuild { .. } => "bundle.build",
            Self::BundleInstall { .. } => "bundle.install",
            Self::BundleKeyInfo { .. } => "bundle.key.inspect",
            Self::CollectionVerify { .. } => "collection.verify",
            Self::CollectionBuild { .. } => "collection.build",
            Self::CollectionInstall { .. } => "collection.install",
            Self::RegistryPull { .. } => "registry.pull",
            Self::RegistryPush { .. } => "registry.push",
        }
    }

    /// Stable resource identity for authorization and audit.
    pub fn resource(&self) -> String {
        match self {
            Self::Verify { path } | Self::Install { path, .. } => format!("pack-source:{path}"),
            Self::Enable { name } | Self::Disable { name } | Self::Uninstall { name } => {
                format!("pack:{name}")
            }
            Self::TrustAdd { publisher, .. } => format!("publisher:{publisher}"),
            Self::BundleVerify { path } => format!("bundle-source:{path}"),
            Self::BundleBuild { destination, .. } => {
                format!("bundle-destination:{destination}")
            }
            Self::BundleInstall { path, prefix } => {
                format!("bundle-source:{path}:install-prefix:{prefix}")
            }
            Self::BundleKeyInfo { .. } => "bundle-signing-key:referenced".into(),
            Self::CollectionVerify { path } | Self::CollectionInstall { path } => {
                format!("collection-source:{path}")
            }
            Self::CollectionBuild { destination, .. } => {
                format!("collection-destination:{destination}")
            }
            Self::RegistryPull { url, .. } | Self::RegistryPush { url, .. } => url.clone(),
        }
    }
}

/// Native artifact target expected by this running executable.
pub fn current_release_target() -> Result<&'static str, PackError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, arch) => Err(PackError::Invalid(format!(
            "no native release target is defined for {os}/{arch}"
        ))),
    }
}
