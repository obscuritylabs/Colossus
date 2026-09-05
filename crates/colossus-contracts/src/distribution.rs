use super::*;

/// One immutable file declared by an offline release bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleFileEntry {
    /// Normalized relative path beneath the bundle root.
    pub path: String,
    /// Lowercase SHA-256 digest of the complete file.
    pub sha256: String,
    /// Exact byte size. Optional only for format-version-1 compatibility.
    #[serde(default)]
    pub size: Option<u64>,
}

/// Detached Ed25519 signature embedded in a release-bundle manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSignature {
    /// Signature algorithm. Format version 1 accepts only `ed25519`.
    pub algorithm: String,
    /// SHA-256 identity of the trusted public key.
    pub key_id: String,
    /// Base64-encoded signature over canonical manifest bytes.
    pub signature: String,
}

/// Strict signed offline release-bundle manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    /// Bundle format version. Version 1 is currently supported.
    pub format_version: u16,
    /// Stable bundle name.
    pub name: String,
    /// Release version represented by this bundle.
    pub version: String,
    /// Publisher identity bound to a configured signing key.
    pub publisher: String,
    /// UTC bundle creation timestamp.
    pub created_at: String,
    /// Optional source revision.
    #[serde(default)]
    pub source_revision: Option<String>,
    /// Complete regular-file allowlist.
    pub files: Vec<BundleFileEntry>,
    /// Cryptographic signatures over the canonical unsigned manifest.
    #[serde(default)]
    pub signatures: Vec<BundleSignature>,
}

/// Evidence produced without network access by bundle verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleVerification {
    /// Bundle identity.
    pub name: String,
    /// Bundle release version.
    pub version: String,
    /// Digest of canonical unsigned manifest bytes.
    pub manifest_sha256: String,
    /// Number of verified payload files.
    pub file_count: usize,
    /// Total verified payload bytes.
    pub total_bytes: u64,
    /// Configured public-key identity that authenticated the manifest.
    pub trust_key_id: String,
    /// Exact source revision when supplied.
    pub source_revision: Option<String>,
}

/// Evidence returned after deterministic signed-bundle materialization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleMaterialization {
    /// Absolute output directory.
    pub path: String,
    /// Verification evidence for the completed bytes.
    pub verification: BundleVerification,
    /// Public signing-key identity.
    pub signing_key_id: String,
    /// Supported release targets found in the payload.
    pub targets: Vec<String>,
}

/// Evidence returned after installing one verified native bundle artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleInstallation {
    /// Verification evidence for the source bundle.
    pub verification: BundleVerification,
    /// Target selected for this executable.
    pub target: String,
    /// Manifest-relative installed artifact.
    pub artifact: String,
    /// Digest of the installed executable.
    pub artifact_sha256: String,
    /// Absolute installed path.
    pub installed_path: String,
}

/// Public identity derived from a referenced release-bundle signing seed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSigningKeyInfo {
    /// SHA-256 identity of the public key.
    pub key_id: String,
    /// Base64-encoded Ed25519 public key.
    pub public_key: String,
}
