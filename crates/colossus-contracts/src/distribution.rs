use super::*;

/// One immutable regular file declared by a capability pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackFileEntry {
    /// Normalized relative path beneath the pack root.
    pub path: String,
    /// Lowercase SHA-256 digest of the complete file.
    pub sha256: String,
    /// Exact file size in bytes.
    pub size: u64,
    /// Bounded media type used for operator evidence.
    pub content_type: String,
}

/// Reference to one hash-listed pack file or directory tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackPathReference {
    /// Normalized relative path beneath the pack root.
    pub path: String,
}

/// One executable tool exposed by a verified pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackToolDeclaration {
    /// Stable namespaced tool name.
    pub name: String,
    /// Hash-listed executable path relative to the pack root.
    pub command: String,
    /// Exact argument vector; no shell interpolation is performed.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variable name to credential reference mapping.
    #[serde(default)]
    pub env_refs: std::collections::BTreeMap<String, String>,
    /// Permissions required by this executable.
    pub permissions: Vec<String>,
}

/// One out-of-process MCP server exposed by a verified pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackMcpServerDeclaration {
    /// Stable server name.
    pub name: String,
    /// Hash-listed executable path relative to the pack root.
    pub command: String,
    /// Exact argument vector; no shell interpolation is performed.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variable name to credential reference mapping.
    #[serde(default)]
    pub env_refs: std::collections::BTreeMap<String, String>,
    /// Exact model-callable tool allowlist.
    pub allowed_tools: Vec<String>,
    /// Permissions required by this server.
    pub permissions: Vec<String>,
}

/// Detached signature embedded in a capability-pack manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackSignature {
    /// Signature algorithm. Version 1 accepts only `ed25519`.
    pub algorithm: String,
    /// SHA-256 identity of the trusted public key.
    pub key_id: String,
    /// Base64-encoded Ed25519 signature of the canonical unsigned manifest.
    pub signature: String,
}

/// Strict versioned capability-pack manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    /// Pack format version. Version 1 is currently supported.
    pub format_version: u16,
    /// Stable lowercase pack name.
    pub name: String,
    /// Immutable semantic version string.
    pub version: String,
    /// Human-facing description.
    pub description: String,
    /// Publisher identity bound to a trusted signing key.
    pub publisher: String,
    /// SPDX-like license identifier.
    pub license: String,
    /// Optional publisher homepage without credentials.
    #[serde(default)]
    pub homepage: String,
    /// Declared contribution families.
    pub capabilities: Vec<String>,
    /// Maximum permissions requested by pack executables.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Complete regular-file allowlist.
    pub files: Vec<PackFileEntry>,
    /// Declarative integration manifests.
    #[serde(default)]
    pub integrations: Vec<PackPathReference>,
    /// Declarative skill directory roots.
    #[serde(default)]
    pub skills: Vec<PackPathReference>,
    /// Declared executable tools.
    #[serde(default)]
    pub tools: Vec<PackToolDeclaration>,
    /// Declared out-of-process MCP servers.
    #[serde(default)]
    pub mcp_servers: Vec<PackMcpServerDeclaration>,
    /// Hash-listed binary paths.
    #[serde(default)]
    pub binaries: Vec<String>,
    /// Hash-listed container assets.
    #[serde(default)]
    pub docker: Vec<String>,
    /// Hash-listed documentation paths.
    #[serde(default)]
    pub docs: Vec<String>,
    /// Hash-listed test paths.
    #[serde(default)]
    pub tests: Vec<String>,
    /// Exact pack dependencies expressed as `name@version`.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Optional cryptographic signatures. Invalid present signatures are fatal.
    #[serde(default)]
    pub signatures: Vec<PackSignature>,
}

/// Durable lifecycle status for an installed pack.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackStatus {
    /// Verified files may contribute declared capabilities.
    Enabled,
    /// Installed bytes are retained but contribute no capability.
    Disabled,
    /// Files were removed while immutable lifecycle history remains.
    Uninstalled,
}

/// Canonical pack lifecycle state reconstructed from the event journal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackInstallation {
    /// Verified immutable manifest.
    pub manifest: PackManifest,
    /// Current lifecycle status.
    pub status: PackStatus,
    /// Stable provenance for the installed source.
    pub source: String,
    /// Absolute configured installation directory.
    pub installed_path: String,
    /// Hash of the canonical unsigned manifest bytes.
    pub manifest_sha256: String,
    /// Trusted key that authenticated this exact manifest, if any.
    pub trust_key_id: Option<String>,
    /// Original installation timestamp.
    pub installed_at: String,
    /// Last lifecycle transition timestamp.
    pub updated_at: String,
}

/// Publisher trust is explicitly bound to one Ed25519 public key.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherTrust {
    /// Publisher identity accepted for this key.
    pub publisher: String,
    /// Lowercase SHA-256 digest of the raw public key.
    pub key_id: String,
    /// Base64-encoded 32-byte Ed25519 public key.
    pub public_key: String,
    /// UTC timestamp of the trust decision.
    pub added_at: String,
}

/// Retainable evidence produced by strict pack verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackVerification {
    /// Verified manifest.
    pub manifest: PackManifest,
    /// SHA-256 of deterministic unsigned manifest bytes.
    pub manifest_sha256: String,
    /// Number of declared regular files verified.
    pub file_count: usize,
    /// Sum of declared file sizes.
    pub total_bytes: u64,
    /// Whether a publisher-bound trusted signature authenticated the manifest.
    pub trusted: bool,
    /// Trusted key that authenticated the manifest, if any.
    pub trust_key_id: Option<String>,
}

/// Kind of independently verified artifact carried by a signed collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionArtifactKind {
    /// Executable-boundary capability pack with its own manifest and trust decision.
    Pack,
    /// Declarative data-only skill tree.
    Skill,
}

/// One pack or skill included in a deterministic signed collection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionArtifactEntry {
    /// Artifact family.
    pub kind: CollectionArtifactKind,
    /// Stable manifest identity.
    pub name: String,
    /// Exact artifact version.
    pub version: String,
    /// Normalized directory path beneath the collection root.
    pub path: String,
    /// Pack manifest hash or skill content hash authenticated by the collection.
    pub content_sha256: String,
}

/// Strict signed manifest for offline multi-pack and skill distribution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionManifest {
    /// Collection format version. Version 1 is currently supported.
    pub format_version: u16,
    /// Stable collection name.
    pub name: String,
    /// Immutable collection version.
    pub version: String,
    /// Publisher identity bound to a trusted signing key.
    pub publisher: String,
    /// Explicit reproducible RFC3339 UTC timestamp.
    pub created_at: String,
    /// Complete deterministic pack and skill inventory.
    pub artifacts: Vec<CollectionArtifactEntry>,
    /// Complete regular-file allowlist excluding this manifest.
    pub files: Vec<PackFileEntry>,
    /// At least one trusted Ed25519 signature is required for use.
    pub signatures: Vec<PackSignature>,
}

/// Retainable evidence from strict collection and nested-artifact verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionVerification {
    /// Verified collection manifest.
    pub manifest: CollectionManifest,
    /// SHA-256 of deterministic unsigned collection manifest bytes.
    pub manifest_sha256: String,
    /// Number of hash-listed payload files.
    pub file_count: usize,
    /// Sum of declared payload sizes.
    pub total_bytes: u64,
    /// Trusted key that authenticated the collection manifest.
    pub trust_key_id: String,
    /// Independently verified nested capability packs.
    pub packs: Vec<PackVerification>,
    /// Independently verified data-only skills.
    pub skills: Vec<SkillValidationResult>,
}

/// Result of deterministically building and signing a collection directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionMaterialization {
    /// Published collection directory.
    pub path: String,
    /// Verification evidence for the published bytes.
    pub verification: CollectionVerification,
    /// Public signing-key identity without secret material.
    pub signing_key_id: String,
}

/// Result of a no-clobber signed collection installation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionInstallation {
    /// Verification evidence for the source collection.
    pub verification: CollectionVerification,
    /// Canonical pack lifecycle states committed in one journal transaction.
    pub packs: Vec<PackInstallation>,
    /// Data-only skills installed into the configured user library.
    pub skills: Vec<SkillInstallResult>,
}

/// Evidence from an authenticated registry pull into a clean local directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryPullResult {
    /// Credential-free source URL.
    pub url: String,
    /// Published clean local collection directory.
    pub path: String,
    /// SHA-256 of the received deterministic tar transport.
    pub transport_sha256: String,
    /// Exact received transport size.
    pub transport_bytes: u64,
    /// Verification evidence for the materialized collection.
    pub verification: CollectionVerification,
}

/// Evidence from an authenticated create-only registry push.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryPushResult {
    /// Credential-free destination URL.
    pub url: String,
    /// Verified collection identity.
    pub collection: String,
    /// Immutable collection version.
    pub version: String,
    /// SHA-256 of the deterministic tar transport.
    pub transport_sha256: String,
    /// Exact uploaded transport size.
    pub transport_bytes: u64,
    /// True when a pre-existing identical object made the push replay-safe.
    pub already_present: bool,
}

/// One immutable file declared by an offline release bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleFileEntry {
    /// Normalized relative path beneath the bundle root.
    pub path: String,
    /// Lowercase SHA-256 digest of the complete file.
    pub sha256: String,
    /// Optional exact byte size for newer bundle producers.
    #[serde(default)]
    pub size: Option<u64>,
}

/// Strict signed offline-bundle manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    /// Bundle format version. Version 1 is currently supported.
    pub format_version: u16,
    /// Human-facing bundle name.
    pub name: String,
    /// Release version represented by this bundle.
    pub version: String,
    /// Publisher identity bound to the trusted signing key.
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
    pub signatures: Vec<PackSignature>,
}

/// Retainable evidence produced without network access by bundle verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleVerification {
    /// Bundle identity.
    pub name: String,
    /// Bundle release version.
    pub version: String,
    /// SHA-256 of deterministic unsigned manifest bytes.
    pub manifest_sha256: String,
    /// Number of regular payload files verified.
    pub file_count: usize,
    /// Sum of verified file sizes.
    pub total_bytes: u64,
    /// Trusted key that authenticated the manifest.
    pub trust_key_id: String,
    /// Exact source revision when supplied.
    pub source_revision: Option<String>,
}

/// Evidence returned after deterministic signed-bundle materialization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleMaterialization {
    /// Absolute destination directory containing the completed bundle.
    pub path: String,
    /// Verification evidence for the exact completed bytes.
    pub verification: BundleVerification,
    /// Publisher-bound Ed25519 key identity used for the signature.
    pub signing_key_id: String,
    /// Supported native artifact targets discovered in the bundle.
    pub targets: Vec<String>,
}

/// Evidence returned after installing one verified native bundle artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleInstallation {
    /// Verification evidence for the source bundle.
    pub verification: BundleVerification,
    /// Native release target selected from the running executable.
    pub target: String,
    /// Manifest-relative artifact path copied into the prefix.
    pub artifact: String,
    /// SHA-256 of the installed executable.
    pub artifact_sha256: String,
    /// Absolute installed executable path.
    pub installed_path: String,
}

/// Public identity derived from a referenced offline-bundle signing seed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSigningKeyInfo {
    /// SHA-256 identity of the raw Ed25519 public key.
    pub key_id: String,
    /// Base64-encoded Ed25519 public key safe to add to publisher trust.
    pub public_key: String,
}
