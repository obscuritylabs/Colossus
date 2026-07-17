//! Strict capability-pack and signed offline-bundle verification and lifecycle management.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, BundleFileEntry, BundleInstallation, BundleManifest, BundleMaterialization,
    BundleSigningKeyInfo, BundleVerification, CollectionArtifactEntry, CollectionArtifactKind,
    CollectionInstallation, CollectionManifest, CollectionMaterialization, CollectionVerification,
    EffectRequest, PackFileEntry, PackInstallation, PackManifest, PackSignature, PackStatus,
    PackVerification, PublisherTrust, QuarantinedEffectResult, RegistryPullResult,
    RegistryPushResult, SkillInstallResult, SkillValidationResult,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit};
use colossus_ports::{ExtensionRepository, StoreError};
use colossus_skills::{copy_verified_skill, inspect_skill_directory};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use futures::StreamExt as _;
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write as _},
    net::IpAddr,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::net::lookup_host;
use tokio_util::io::ReaderStream;

const PACK_MANIFEST: &str = "colossus.pack.json";
const BUNDLE_MANIFEST: &str = "manifest.json";
const COLLECTION_MANIFEST: &str = "colossus.collection.json";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
const MAX_TEXT_BYTES: usize = 8 * 1024;
const MAX_ARCHIVE_BYTES: u64 = MAX_TOTAL_BYTES;
const RELEASE_TARGETS: [&str; 6] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_TAR_MEDIA_TYPES: [&str; 2] = [
    "application/vnd.colossus.pack.v1.tar",
    "application/vnd.oci.image.layer.v1.tar",
];
const OCI_GZIP_MEDIA_TYPES: [&str; 2] = [
    "application/vnd.colossus.pack.v1.tar+gzip",
    "application/vnd.oci.image.layer.v1.tar+gzip",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OciLayout {
    image_layout_version: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OciDescriptor {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    platform: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OciIndex {
    schema_version: u16,
    manifests: Vec<OciDescriptor>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OciManifest {
    schema_version: u16,
    layers: Vec<OciDescriptor>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    config: Option<OciDescriptor>,
    #[serde(default)]
    subject: Option<OciDescriptor>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

struct MaterializedPack {
    root: PathBuf,
    _temporary: Option<tempfile::TempDir>,
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

/// Strict verifier and event-sourced lifecycle service.
pub struct PackService {
    repository: Arc<dyn ExtensionRepository>,
    install_root: PathBuf,
    skill_install_root: PathBuf,
}

impl PackService {
    /// Bind pack operations to the canonical extension repository and configured install root.
    pub fn new(repository: Arc<dyn ExtensionRepository>, install_root: PathBuf) -> Self {
        let skill_install_root = install_root
            .parent()
            .map_or_else(|| PathBuf::from("skills"), |parent| parent.join("skills"));
        Self {
            repository,
            install_root,
            skill_install_root,
        }
    }

    /// Override the configured user-skill installation root used by signed collections.
    #[must_use]
    pub fn with_skill_install_root(mut self, skill_install_root: PathBuf) -> Self {
        self.skill_install_root = skill_install_root;
        self
    }

    /// Reconstruct one canonical pack lifecycle.
    pub fn get(&self, name: &str) -> Result<Option<PackInstallation>, PackError> {
        Ok(self.repository.get_pack(name)?)
    }

    /// List bounded canonical pack lifecycles.
    pub fn list(&self, limit: usize) -> Result<Vec<PackInstallation>, PackError> {
        Ok(self.repository.list_packs(limit)?)
    }

    /// List publisher/key trust bindings.
    pub fn list_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, PackError> {
        Ok(self.repository.list_publisher_trust(limit)?)
    }

    /// Verify a local pack against strict file, manifest, and publisher-key contracts.
    pub fn verify(&self, root: &Path) -> Result<PackVerification, PackError> {
        let materialized = materialize_pack_source(root)?;
        verify_pack(&materialized.root, self.repository.as_ref())
    }

    fn add_trust(
        &self,
        publisher: &str,
        public_key: &str,
        actor: Actor,
    ) -> Result<PublisherTrust, PackError> {
        validate_identity("publisher", publisher)?;
        let bytes = BASE64
            .decode(public_key)
            .map_err(|_| PackError::Invalid("publisher public key must be base64".into()))?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| {
            PackError::Invalid("publisher Ed25519 public key must be exactly 32 bytes".into())
        })?;
        VerifyingKey::from_bytes(&key)
            .map_err(|_| PackError::Invalid("publisher Ed25519 public key is invalid".into()))?;
        let trust = PublisherTrust {
            publisher: publisher.into(),
            key_id: digest_hex(&key),
            public_key: BASE64.encode(key),
            added_at: now()?,
        };
        Ok(self.repository.add_publisher_trust(trust, actor)?)
    }

    fn install(
        &self,
        source: &Path,
        allow_untrusted: bool,
        actor: Actor,
    ) -> Result<PackInstallation, PackError> {
        let materialized = materialize_pack_source(source)?;
        let verification = verify_pack(&materialized.root, self.repository.as_ref())?;
        if !verification.trusted && !allow_untrusted {
            return Err(PackError::Invalid(format!(
                "pack {} is unsigned or not trusted; explicit approval-gated allow_untrusted is required",
                verification.manifest.name
            )));
        }
        self.validate_dependencies(&verification.manifest)?;
        let install_root = ensure_install_root(&self.install_root)?;
        let destination = install_root
            .join(&verification.manifest.name)
            .join(&verification.manifest.version);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "pack destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| PackError::Invalid("pack destination has no parent directory".into()))?;
        fs::create_dir_all(parent)?;
        reject_symlink_chain(&install_root, parent)?;
        let temp = tempfile::Builder::new()
            .prefix(".pack-install-")
            .tempdir_in(parent)?;
        copy_verified_pack(&materialized.root, temp.path(), &verification.manifest)?;
        let copied = self.verify(temp.path())?;
        if copied.manifest_sha256 != verification.manifest_sha256
            || copied.trusted != verification.trusted
        {
            return Err(PackError::Invalid(
                "pack source changed while it was copied".into(),
            ));
        }
        fs::rename(temp.path(), &destination)?;
        let timestamp = now()?;
        let installation = PackInstallation {
            manifest: verification.manifest,
            status: PackStatus::Enabled,
            source: source.display().to_string(),
            installed_path: destination.display().to_string(),
            manifest_sha256: verification.manifest_sha256,
            trust_key_id: verification.trust_key_id,
            installed_at: timestamp.clone(),
            updated_at: timestamp,
        };
        match self.repository.install_pack(installation, actor) {
            Ok(installation) => Ok(installation),
            Err(error) => {
                let _ = fs::remove_dir_all(&destination);
                Err(error.into())
            }
        }
    }

    fn enable(&self, name: &str, actor: Actor) -> Result<PackInstallation, PackError> {
        let current = self
            .repository
            .get_pack(name)?
            .ok_or_else(|| StoreError::NotFound(format!("pack {name}")))?;
        if current.status == PackStatus::Uninstalled {
            return Err(PackError::Invalid(format!("pack {name} is uninstalled")));
        }
        self.validate_dependencies(&current.manifest)?;
        let verification = self.verify(Path::new(&current.installed_path))?;
        if verification.manifest_sha256 != current.manifest_sha256
            || verification.trust_key_id != current.trust_key_id
        {
            return Err(PackError::Invalid(format!(
                "installed pack {name} no longer matches its canonical installation"
            )));
        }
        Ok(self
            .repository
            .set_pack_status(name, PackStatus::Enabled, actor, &now()?)?)
    }

    fn disable(&self, name: &str, actor: Actor) -> Result<PackInstallation, PackError> {
        Ok(self
            .repository
            .set_pack_status(name, PackStatus::Disabled, actor, &now()?)?)
    }

    fn uninstall(&self, name: &str, actor: Actor) -> Result<PackInstallation, PackError> {
        let current = self
            .repository
            .get_pack(name)?
            .ok_or_else(|| StoreError::NotFound(format!("pack {name}")))?;
        let install_root = ensure_install_root(&self.install_root)?;
        let path = PathBuf::from(&current.installed_path);
        let expected_parent = install_root.join(name);
        if path.parent() != Some(expected_parent.as_path()) {
            return Err(PackError::Invalid(
                "canonical pack path is outside its configured installation slot".into(),
            ));
        }
        if fs::symlink_metadata(&path).is_ok() {
            reject_symlink_chain(&install_root, &path)?;
        }
        let installation =
            self.repository
                .set_pack_status(name, PackStatus::Uninstalled, actor, &now()?)?;
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_dir_all(&path)?;
        }
        Ok(installation)
    }

    fn validate_dependencies(&self, manifest: &PackManifest) -> Result<(), PackError> {
        for dependency in &manifest.dependencies {
            let (name, version) = dependency.split_once('@').ok_or_else(|| {
                PackError::Invalid(format!(
                    "pack dependency must be name@version: {dependency}"
                ))
            })?;
            let installed = self.repository.get_pack(name)?.ok_or_else(|| {
                PackError::Invalid(format!("required pack dependency is absent: {dependency}"))
            })?;
            if installed.status != PackStatus::Enabled || installed.manifest.version != version {
                return Err(PackError::Invalid(format!(
                    "required pack dependency is not enabled at the exact version: {dependency}"
                )));
            }
        }
        Ok(())
    }

    fn verify_collection(&self, root: &Path) -> Result<CollectionVerification, PackError> {
        verify_collection(root, self.repository.as_ref())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_collection(
        &self,
        source: &Path,
        destination: &Path,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        signing_seed: [u8; 32],
    ) -> Result<CollectionMaterialization, PackError> {
        validate_identity("collection name", name)?;
        validate_identity("publisher", publisher)?;
        validate_bounded("collection version", version, 128)?;
        validate_bundle_timestamp(created_at)?;
        let source = verified_root(source)?;
        if fs::symlink_metadata(source.join(COLLECTION_MANIFEST)).is_ok() {
            return Err(PackError::Invalid(format!(
                "staged collection payload must not contain {COLLECTION_MANIFEST}"
            )));
        }
        validate_absolute_normalized(destination, "collection destination")?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "collection destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            PackError::Invalid("collection destination has no parent directory".into())
        })?;
        let parent = verified_root(parent)?;
        if parent.starts_with(&source) {
            return Err(PackError::Invalid(
                "collection destination cannot be inside the staged payload".into(),
            ));
        }
        let temporary = tempfile::Builder::new()
            .prefix(".collection-build-")
            .tempdir_in(&parent)?;
        copy_bundle_payload(&source, temporary.path())?;
        let artifacts = discover_collection_artifacts(temporary.path(), self.repository.as_ref())?;
        if artifacts.is_empty() {
            return Err(PackError::Invalid(
                "collection must contain at least one pack or skill".into(),
            ));
        }
        let files = collect_collection_entries(temporary.path())?;
        let signing_key = SigningKey::from_bytes(&signing_seed);
        let signing_key_id = digest_hex(signing_key.verifying_key().as_bytes());
        let mut manifest = CollectionManifest {
            format_version: 1,
            name: name.into(),
            version: version.into(),
            publisher: publisher.into(),
            created_at: created_at.into(),
            artifacts,
            files,
            signatures: Vec::new(),
        };
        let unsigned = canonical_collection_signing_bytes(&manifest)?;
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: signing_key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        let manifest_path = temporary.path().join(COLLECTION_MANIFEST);
        let mut manifest_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&manifest_path)?;
        serde_json::to_writer_pretty(&mut manifest_file, &manifest)?;
        manifest_file.write_all(b"\n")?;
        manifest_file.sync_all()?;
        drop(manifest_file);
        let verification = self.verify_collection(temporary.path())?;
        fs::rename(temporary.path(), destination)?;
        Ok(CollectionMaterialization {
            path: destination.display().to_string(),
            verification,
            signing_key_id,
        })
    }

    fn install_collection(
        &self,
        root: &Path,
        actor: Actor,
    ) -> Result<CollectionInstallation, PackError> {
        let root = verified_root(root)?;
        let verification = self.verify_collection(&root)?;
        let pack_root = ensure_install_root(&self.install_root)?;
        let skill_root = ensure_install_root(&self.skill_install_root)?;
        let mut pack_staging = Vec::new();
        let mut skill_staging = Vec::new();
        let timestamp = now()?;

        for pack in &verification.packs {
            if self
                .repository
                .get_pack(&pack.manifest.name)?
                .is_some_and(|installed| installed.status != PackStatus::Uninstalled)
            {
                return Err(PackError::Invalid(format!(
                    "collection refuses to replace installed pack: {}",
                    pack.manifest.name
                )));
            }
            let destination = pack_root
                .join(&pack.manifest.name)
                .join(&pack.manifest.version);
            if fs::symlink_metadata(&destination).is_ok() {
                return Err(PackError::Invalid(format!(
                    "collection pack destination already exists: {}",
                    destination.display()
                )));
            }
            let parent = destination.parent().ok_or_else(|| {
                PackError::Invalid("collection pack destination has no parent".into())
            })?;
            fs::create_dir_all(parent)?;
            reject_symlink_chain(&pack_root, parent)?;
            let temporary = tempfile::Builder::new()
                .prefix(".collection-pack-")
                .tempdir_in(parent)?;
            let artifact = verification
                .manifest
                .artifacts
                .iter()
                .find(|artifact| {
                    artifact.kind == CollectionArtifactKind::Pack
                        && artifact.name == pack.manifest.name
                })
                .ok_or_else(|| PackError::Invalid("verified collection pack is absent".into()))?;
            let source = root.join(&artifact.path);
            copy_verified_pack(&source, temporary.path(), &pack.manifest)?;
            let copied = verify_pack(temporary.path(), self.repository.as_ref())?;
            if copied.manifest_sha256 != pack.manifest_sha256
                || copied.trust_key_id != pack.trust_key_id
            {
                return Err(PackError::Invalid(
                    "collection pack changed while it was staged".into(),
                ));
            }
            pack_staging.push((temporary, destination, pack.clone()));
        }

        for skill in &verification.skills {
            let destination = skill_root.join(&skill.name);
            if fs::symlink_metadata(&destination).is_ok() {
                return Err(PackError::Invalid(format!(
                    "collection refuses to replace installed skill: {}",
                    skill.name
                )));
            }
            let temporary = tempfile::Builder::new()
                .prefix(".collection-skill-")
                .tempdir_in(&skill_root)?;
            let staged = temporary.path().join("skill");
            let artifact = verification
                .manifest
                .artifacts
                .iter()
                .find(|artifact| {
                    artifact.kind == CollectionArtifactKind::Skill && artifact.name == skill.name
                })
                .ok_or_else(|| PackError::Invalid("verified collection skill is absent".into()))?;
            let result = copy_verified_skill(
                &root.join(&artifact.path),
                &staged,
                &skill.name,
                &skill.content_sha256,
            )?;
            skill_staging.push((temporary, staged, destination, result));
        }

        let installations = pack_staging
            .iter()
            .map(|(_, destination, pack)| PackInstallation {
                manifest: pack.manifest.clone(),
                status: PackStatus::Enabled,
                source: format!(
                    "collection:{}@{}",
                    verification.manifest.name, verification.manifest.version
                ),
                installed_path: destination.display().to_string(),
                manifest_sha256: pack.manifest_sha256.clone(),
                trust_key_id: pack.trust_key_id.clone(),
                installed_at: timestamp.clone(),
                updated_at: timestamp.clone(),
            })
            .collect::<Vec<_>>();
        let mut committed = Vec::new();
        let commit_result = (|| {
            for (temporary, destination, _) in &pack_staging {
                fs::rename(temporary.path(), destination)?;
                committed.push(destination.clone());
            }
            for (_, staged, destination, _) in &skill_staging {
                fs::rename(staged, destination)?;
                committed.push(destination.clone());
            }
            if installations.is_empty() {
                Ok(Vec::new())
            } else {
                self.repository
                    .install_packs(installations, actor)
                    .map_err(PackError::from)
            }
        })();
        let packs = match commit_result {
            Ok(packs) => packs,
            Err(error) => {
                for path in committed.iter().rev() {
                    let _ = fs::remove_dir_all(path);
                }
                return Err(error);
            }
        };
        Ok(CollectionInstallation {
            verification,
            packs,
            skills: skill_staging
                .into_iter()
                .map(|(_, _, _, result)| result)
                .collect::<Vec<SkillInstallResult>>(),
        })
    }

    async fn registry_pull(
        &self,
        url: &str,
        destination: &Path,
        credential_reference: Option<&str>,
        permit: &ExecutionPermit,
    ) -> Result<RegistryPullResult, PackError> {
        validate_absolute_normalized(destination, "registry pull destination")?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "registry pull destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            PackError::Invalid("registry pull destination has no parent directory".into())
        })?;
        let parent = verified_root(parent)?;
        let (url, client) = registry_client(url, permit).await?;
        let request = registry_auth(
            client
                .get(url.clone())
                .header("accept", "application/vnd.colossus.collection.v1.tar"),
            credential_reference,
            permit,
        )?;
        let response = request
            .send()
            .await
            .map_err(|error| PackError::Invalid(format!("registry pull failed: {error}")))?;
        if !response.status().is_success() {
            return Err(PackError::Invalid(format!(
                "registry pull returned {}",
                response.status()
            )));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some("application/vnd.colossus.collection.v1.tar")
        {
            return Err(PackError::Invalid(
                "registry pull returned an unexpected content type".into(),
            ));
        }
        let limit = permit.obligations().max_output_bytes.min(MAX_ARCHIVE_BYTES);
        if response.content_length().is_some_and(|size| size > limit) {
            return Err(PackError::Invalid(
                "registry collection transport exceeds the permitted bound".into(),
            ));
        }
        let mut transport = tempfile::NamedTempFile::new_in(&parent)?;
        let mut transport_bytes = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                PackError::Invalid(format!("registry pull stream failed: {error}"))
            })?;
            transport_bytes = transport_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| PackError::Invalid("registry transport size overflow".into()))?;
            if transport_bytes > limit {
                return Err(PackError::Invalid(
                    "registry collection transport exceeds the permitted bound".into(),
                ));
            }
            transport.write_all(&chunk)?;
        }
        transport.as_file_mut().sync_all()?;
        let transport_sha256 = hash_file(transport.path(), limit)?;
        let staging = tempfile::Builder::new()
            .prefix(".registry-pull-")
            .tempdir_in(&parent)?;
        extract_collection_archive(transport.path(), staging.path())?;
        let verification = self.verify_collection(staging.path())?;
        fs::rename(staging.path(), destination)?;
        Ok(RegistryPullResult {
            url: url.to_string(),
            path: destination.display().to_string(),
            transport_sha256,
            transport_bytes,
            verification,
        })
    }

    async fn registry_push(
        &self,
        root: &Path,
        url: &str,
        credential_reference: Option<&str>,
        permit: &ExecutionPermit,
    ) -> Result<RegistryPushResult, PackError> {
        let root = verified_root(root)?;
        let verification = self.verify_collection(&root)?;
        let mut transport = tempfile::NamedTempFile::new()?;
        write_collection_archive(&root, &verification, transport.as_file_mut())?;
        transport.as_file_mut().sync_all()?;
        let transport_bytes = transport.as_file().metadata()?.len();
        let limit = permit.obligations().max_output_bytes.min(MAX_ARCHIVE_BYTES);
        if transport_bytes > limit {
            return Err(PackError::Invalid(
                "registry collection transport exceeds the permitted bound".into(),
            ));
        }
        let transport_sha256 = hash_file(transport.path(), limit)?;
        let (url, client) = registry_client(url, permit).await?;
        let file = tokio::fs::File::from_std(transport.reopen()?);
        let body = reqwest::Body::wrap_stream(ReaderStream::new(file));
        let request = registry_auth(
            client
                .put(url.clone())
                .header("content-type", "application/vnd.colossus.collection.v1.tar")
                .header("content-length", transport_bytes)
                .header("if-none-match", "*")
                .header("x-content-sha256", &transport_sha256)
                .body(body),
            credential_reference,
            permit,
        )?;
        let response = request.send().await.map_err(|error| {
            PackError::OutcomeUnknown(format!(
                "registry push may have completed after transport failure: {error}"
            ))
        })?;
        let already_present = response.status() == reqwest::StatusCode::PRECONDITION_FAILED;
        if !response.status().is_success() && !already_present {
            return Err(PackError::Invalid(format!(
                "registry push returned {}",
                response.status()
            )));
        }
        if already_present
            && response
                .headers()
                .get("x-content-sha256")
                .and_then(|value| value.to_str().ok())
                != Some(transport_sha256.as_str())
        {
            return Err(PackError::Invalid(
                "registry create-only conflict did not prove identical content".into(),
            ));
        }
        Ok(RegistryPushResult {
            url: url.to_string(),
            collection: verification.manifest.name,
            version: verification.manifest.version,
            transport_sha256,
            transport_bytes,
            already_present,
        })
    }

    fn verify_bundle(&self, root: &Path) -> Result<BundleVerification, PackError> {
        verify_bundle(root, self.repository.as_ref())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_bundle(
        &self,
        source: &Path,
        destination: &Path,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        source_revision: Option<String>,
        signing_seed: [u8; 32],
    ) -> Result<BundleMaterialization, PackError> {
        validate_identity("bundle name", name)?;
        validate_identity("publisher", publisher)?;
        validate_bounded("bundle version", version, 128)?;
        validate_bundle_timestamp(created_at)?;
        if let Some(revision) = source_revision.as_deref() {
            validate_bounded("bundle source_revision", revision, 256)?;
        }
        let source = verified_root(source)?;
        if fs::symlink_metadata(source.join(BUNDLE_MANIFEST)).is_ok() {
            return Err(PackError::Invalid(format!(
                "staged bundle payload must not contain {BUNDLE_MANIFEST}"
            )));
        }
        validate_absolute_normalized(destination, "bundle destination")?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(PackError::Invalid(format!(
                "bundle destination already exists: {}",
                destination.display()
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            PackError::Invalid("bundle destination has no parent directory".into())
        })?;
        let parent = verified_root(parent)?;
        if parent.starts_with(&source) {
            return Err(PackError::Invalid(
                "bundle destination cannot be inside the staged payload".into(),
            ));
        }
        let temporary = tempfile::Builder::new()
            .prefix(".bundle-build-")
            .tempdir_in(&parent)?;
        copy_bundle_payload(&source, temporary.path())?;
        let files = collect_bundle_entries(temporary.path())?;
        let targets = installable_bundle_targets(&files);
        if targets.is_empty() {
            return Err(PackError::Invalid(
                "bundle must contain at least one artifacts/TARGET/colossus native executable"
                    .into(),
            ));
        }
        let signing_key = SigningKey::from_bytes(&signing_seed);
        let signing_key_id = digest_hex(signing_key.verifying_key().as_bytes());
        let mut manifest = BundleManifest {
            format_version: 1,
            name: name.into(),
            version: version.into(),
            publisher: publisher.into(),
            created_at: created_at.into(),
            source_revision,
            files,
            signatures: Vec::new(),
        };
        let unsigned = canonical_bundle_signing_bytes(&manifest)?;
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: signing_key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        let manifest_path = temporary.path().join(BUNDLE_MANIFEST);
        let mut manifest_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&manifest_path)?;
        serde_json::to_writer_pretty(&mut manifest_file, &manifest)?;
        manifest_file.write_all(b"\n")?;
        manifest_file.sync_all()?;
        // Windows will not rename a directory while a file inside it is still open.
        // Close the durable manifest before verification and atomic publication.
        drop(manifest_file);
        let verification = self.verify_bundle(temporary.path())?;
        fs::rename(temporary.path(), destination)?;
        Ok(BundleMaterialization {
            path: destination.display().to_string(),
            verification,
            signing_key_id,
            targets,
        })
    }

    fn install_bundle(&self, root: &Path, prefix: &Path) -> Result<BundleInstallation, PackError> {
        let root = verified_root(root)?;
        let verification = self.verify_bundle(&root)?;
        let manifest: BundleManifest = read_manifest(&root.join(BUNDLE_MANIFEST))?;
        let target = current_release_target()?.to_owned();
        let artifact = bundle_artifact_path(&target);
        let entry = manifest
            .files
            .iter()
            .find(|entry| entry.path == artifact)
            .ok_or_else(|| {
                PackError::Invalid(format!(
                    "bundle does not contain a native executable for {target}"
                ))
            })?;
        let source = root.join(&artifact);
        reject_symlink_chain(&root, &source)?;
        checked_regular_file(&source)?;
        if hash_file(&source, MAX_FILE_BYTES)? != entry.sha256 {
            return Err(PackError::Invalid(
                "bundle artifact changed after verification".into(),
            ));
        }
        let prefix = ensure_real_directory(prefix, "bundle install prefix")?;
        let bin = prefix.join("bin");
        let bin = ensure_real_directory(&bin, "bundle install bin directory")?;
        let installed = bin.join(if cfg!(windows) {
            "colossus.exe"
        } else {
            "colossus"
        });
        if fs::symlink_metadata(&installed).is_ok() {
            return Err(PackError::Invalid(format!(
                "bundle installation refuses to replace existing path: {}",
                installed.display()
            )));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&bin)?;
        let mut input = fs::File::open(&source)?;
        std::io::copy(&mut input, temporary.as_file_mut())?;
        temporary.as_file_mut().sync_all()?;
        set_executable_permissions(temporary.path())?;
        if hash_file(temporary.path(), MAX_FILE_BYTES)? != entry.sha256 {
            return Err(PackError::Invalid(
                "bundle artifact changed while it was copied".into(),
            ));
        }
        temporary
            .persist_noclobber(&installed)
            .map_err(|error| error.error)?;
        Ok(BundleInstallation {
            verification,
            target,
            artifact,
            artifact_sha256: entry.sha256.clone(),
            installed_path: installed.display().to_string(),
        })
    }
}

/// Permit-bearing adapter for pack and bundle effects.
pub struct PackExecutor {
    service: Arc<PackService>,
}

impl PackExecutor {
    /// Construct the adapter around one lifecycle service.
    pub fn new(service: Arc<PackService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EffectExecutor for PackExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: PackOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(ExecutionError::Failed(
                "pack request does not match its authorized action and resource".into(),
            ));
        }
        if let Some(path) = source_path(&operation) {
            enforce_read_grant(path, &permit)?;
        }
        if let Some(path) = destination_path(&operation) {
            enforce_write_grant(path, &permit)?;
        }
        if matches!(
            operation,
            PackOperation::RegistryPull { .. } | PackOperation::RegistryPush { .. }
        ) {
            enforce_registry_credentials(&operation, request)?;
        }
        let value = match operation {
            PackOperation::Verify { path } => {
                serde_json::to_value(self.service.verify(Path::new(&path)).map_err(execution)?)
            }
            PackOperation::Install {
                path,
                allow_untrusted,
            } => serde_json::to_value(
                self.service
                    .install(Path::new(&path), allow_untrusted, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::Enable { name } => serde_json::to_value(
                self.service
                    .enable(&name, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::Disable { name } => serde_json::to_value(
                self.service
                    .disable(&name, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::Uninstall { name } => serde_json::to_value(
                self.service
                    .uninstall(&name, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::TrustAdd {
                publisher,
                public_key,
            } => serde_json::to_value(
                self.service
                    .add_trust(&publisher, &public_key, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::BundleVerify { path } => serde_json::to_value(
                self.service
                    .verify_bundle(Path::new(&path))
                    .map_err(execution)?,
            ),
            PackOperation::BundleBuild {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                source_revision,
                signing_key_reference,
            } => serde_json::to_value(
                self.service
                    .build_bundle(
                        Path::new(&source),
                        Path::new(&destination),
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        source_revision,
                        resolve_signing_seed(&signing_key_reference).map_err(execution)?,
                    )
                    .map_err(execution)?,
            ),
            PackOperation::BundleInstall { path, prefix } => serde_json::to_value(
                self.service
                    .install_bundle(Path::new(&path), Path::new(&prefix))
                    .map_err(execution)?,
            ),
            PackOperation::BundleKeyInfo {
                signing_key_reference,
            } => serde_json::to_value(signing_key_info(
                resolve_signing_seed(&signing_key_reference).map_err(execution)?,
            )),
            PackOperation::CollectionVerify { path } => serde_json::to_value(
                self.service
                    .verify_collection(Path::new(&path))
                    .map_err(execution)?,
            ),
            PackOperation::CollectionBuild {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                signing_key_reference,
            } => serde_json::to_value(
                self.service
                    .build_collection(
                        Path::new(&source),
                        Path::new(&destination),
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        resolve_signing_seed(&signing_key_reference).map_err(execution)?,
                    )
                    .map_err(execution)?,
            ),
            PackOperation::CollectionInstall { path } => serde_json::to_value(
                self.service
                    .install_collection(Path::new(&path), request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::RegistryPull {
                url,
                destination,
                credential_reference,
            } => serde_json::to_value(
                self.service
                    .registry_pull(
                        &url,
                        Path::new(&destination),
                        credential_reference.as_deref(),
                        &permit,
                    )
                    .await
                    .map_err(pack_execution)?,
            ),
            PackOperation::RegistryPush {
                path,
                url,
                credential_reference,
            } => serde_json::to_value(
                self.service
                    .registry_push(
                        Path::new(&path),
                        &url,
                        credential_reference.as_deref(),
                        &permit,
                    )
                    .await
                    .map_err(pack_execution)?,
            ),
        }
        .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

fn source_path(operation: &PackOperation) -> Option<&Path> {
    match operation {
        PackOperation::Verify { path }
        | PackOperation::Install { path, .. }
        | PackOperation::BundleVerify { path }
        | PackOperation::BundleInstall { path, .. }
        | PackOperation::CollectionVerify { path }
        | PackOperation::CollectionInstall { path }
        | PackOperation::RegistryPush { path, .. } => Some(Path::new(path)),
        PackOperation::BundleBuild { source, .. }
        | PackOperation::CollectionBuild { source, .. } => Some(Path::new(source)),
        _ => None,
    }
}

fn destination_path(operation: &PackOperation) -> Option<&Path> {
    match operation {
        PackOperation::BundleBuild { destination, .. } => Some(Path::new(destination)),
        PackOperation::BundleInstall { prefix, .. } => Some(Path::new(prefix)),
        PackOperation::CollectionBuild { destination, .. } => Some(Path::new(destination)),
        PackOperation::RegistryPull { destination, .. } => Some(Path::new(destination)),
        _ => None,
    }
}

fn enforce_registry_credentials(
    operation: &PackOperation,
    request: &EffectRequest,
) -> Result<(), ExecutionError> {
    let expected = match operation {
        PackOperation::RegistryPull {
            credential_reference,
            ..
        }
        | PackOperation::RegistryPush {
            credential_reference,
            ..
        } => credential_reference.as_deref(),
        _ => None,
    };
    let actual = request
        .credential_references
        .iter()
        .map(|credential| credential.reference.as_str())
        .collect::<Vec<_>>();
    let matches = match expected {
        Some(reference) => actual == [reference],
        None => actual.is_empty(),
    };
    if !matches {
        return Err(ExecutionError::Failed(
            "registry credential references do not match the authorized operation".into(),
        ));
    }
    Ok(())
}

fn signing_key_info(seed: [u8; 32]) -> BundleSigningKeyInfo {
    let signing_key = SigningKey::from_bytes(&seed);
    let public = signing_key.verifying_key().to_bytes();
    BundleSigningKeyInfo {
        key_id: digest_hex(&public),
        public_key: BASE64.encode(public),
    }
}

fn enforce_read_grant(path: &Path, permit: &ExecutionPermit) -> Result<(), ExecutionError> {
    let canonical = fs::canonicalize(path).map_err(execution)?;
    let allowed = permit.obligations().filesystem.iter().any(|grant| {
        matches!(grant.mode.as_str(), "read" | "write")
            && fs::canonicalize(&grant.root).is_ok_and(|root| canonical.starts_with(root))
    });
    if !allowed {
        return Err(ExecutionError::Failed(format!(
            "pack source {} is outside policy-authorized filesystem roots",
            canonical.display()
        )));
    }
    Ok(())
}

fn enforce_write_grant(path: &Path, permit: &ExecutionPermit) -> Result<(), ExecutionError> {
    validate_absolute_normalized(path, "bundle write destination").map_err(execution)?;
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    ExecutionError::Failed(format!(
                        "bundle destination {} has no existing ancestor",
                        path.display()
                    ))
                })?;
            }
            Err(error) => return Err(execution(error)),
        }
    }
    let resolved_existing = fs::canonicalize(existing).map_err(execution)?;
    let allowed = permit.obligations().filesystem.iter().any(|grant| {
        if grant.mode != "write" {
            return false;
        }
        fs::canonicalize(&grant.root).is_ok_and(|root| resolved_existing.starts_with(root))
    });
    if !allowed {
        return Err(ExecutionError::Failed(format!(
            "bundle destination {} is outside policy-authorized write roots",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_signing_seed(reference: &str) -> Result<[u8; 32], PackError> {
    let variable = reference.strip_prefix("env:").ok_or_else(|| {
        PackError::Invalid("bundle signing keys must use env:VARIABLE references".into())
    })?;
    if variable.is_empty()
        || !variable.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
    {
        return Err(PackError::Invalid(
            "bundle signing keys must use env:VARIABLE references".into(),
        ));
    }
    let encoded = std::env::var(variable).map_err(|_| {
        PackError::Invalid(format!("bundle signing credential {variable} is unset"))
    })?;
    let decoded = hex::decode(&encoded)
        .or_else(|_| BASE64.decode(&encoded))
        .map_err(|_| PackError::Invalid("bundle signing seed must be hex or base64".into()))?;
    decoded.try_into().map_err(|_| {
        PackError::Invalid("bundle signing seed must decode to exactly 32 bytes".into())
    })
}

fn execution(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

fn pack_execution(error: PackError) -> ExecutionError {
    match error {
        PackError::OutcomeUnknown(message) => ExecutionError::OutcomeUnknown(message),
        error => ExecutionError::Failed(error.to_string()),
    }
}

async fn registry_client(
    endpoint: &str,
    permit: &ExecutionPermit,
) -> Result<(Url, Client), PackError> {
    let url = Url::parse(endpoint)
        .map_err(|error| PackError::Invalid(format!("invalid registry URL: {error}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| PackError::Invalid("registry URL must include a host".into()))?;
    let host_ip = host.parse::<IpAddr>().ok();
    let loopback_http = url.scheme() == "http" && host_ip.is_some_and(|ip| ip.is_loopback());
    if !(url.scheme() == "https" || loopback_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(PackError::Invalid(
            "registry URLs require HTTPS (or explicit loopback HTTP) and no credentials, query, or fragment"
                .into(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    if !permit
        .obligations()
        .network_destinations
        .iter()
        .any(|allowed| allowed == &origin)
    {
        return Err(PackError::Invalid(format!(
            "registry origin {origin} is absent from permit obligations"
        )));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PackError::Invalid("registry URL must resolve to a known port".into()))?;
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|error| PackError::Invalid(format!("registry DNS resolution failed: {error}")))?
        .filter(|address| match host_ip {
            Some(ip) => ip.is_loopback() || !non_public_ip(address.ip()),
            None => !non_public_ip(address.ip()),
        })
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(PackError::Invalid(
            "registry host resolved to no permitted address".into(),
        ));
    }
    addresses.sort();
    addresses.dedup();
    addresses.truncate(16);
    let client = Client::builder()
        .no_proxy()
        .redirect(RedirectPolicy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_millis(permit.obligations().timeout_ms))
        .build()
        .map_err(|error| PackError::Invalid(format!("registry client failed: {error}")))?;
    Ok((url, client))
}

fn registry_auth(
    request: reqwest::RequestBuilder,
    credential_reference: Option<&str>,
    permit: &ExecutionPermit,
) -> Result<reqwest::RequestBuilder, PackError> {
    let Some(reference) = credential_reference else {
        return Ok(request);
    };
    let variable = reference.strip_prefix("env:").ok_or_else(|| {
        PackError::Invalid("registry credentials must use env:VARIABLE references".into())
    })?;
    if variable.is_empty()
        || !variable.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
        || !permit
            .obligations()
            .allowed_environment
            .iter()
            .any(|allowed| allowed == variable)
    {
        return Err(PackError::Invalid(
            "registry credential is absent from permit environment obligations".into(),
        ));
    }
    let secret = std::env::var(variable)
        .map_err(|_| PackError::Invalid(format!("registry credential {variable} is unset")))?;
    if secret.is_empty() {
        return Err(PackError::Invalid(format!(
            "registry credential {variable} is empty"
        )));
    }
    Ok(request.bearer_auth(secret))
}

fn non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn write_collection_archive(
    root: &Path,
    verification: &CollectionVerification,
    output: &mut fs::File,
) -> Result<(), PackError> {
    let mut paths = vec![COLLECTION_MANIFEST.to_owned()];
    paths.extend(
        verification
            .manifest
            .files
            .iter()
            .map(|entry| entry.path.clone()),
    );
    paths.sort();
    let mut archive = tar::Builder::new(output);
    for relative in paths {
        validate_relative_path(&relative)?;
        let path = root.join(&relative);
        reject_symlink_chain(root, &path)?;
        let metadata = checked_regular_file(&path)?;
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(metadata.len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            header.set_mode(if metadata.permissions().mode() & 0o111 == 0 {
                0o644
            } else {
                0o755
            });
        }
        #[cfg(not(unix))]
        header.set_mode(0o644);
        header.set_cksum();
        let mut input = fs::File::open(&path)?;
        archive.append_data(&mut header, &relative, &mut input)?;
    }
    archive.finish()?;
    Ok(())
}

fn extract_collection_archive(archive_path: &Path, destination: &Path) -> Result<(), PackError> {
    let input = fs::File::open(archive_path)?;
    let mut archive = tar::Archive::new(input);
    let mut paths = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if paths.len() > MAX_FILES {
            return Err(PackError::Invalid(format!(
                "registry collection transport exceeds {} entries",
                MAX_FILES + 1
            )));
        }
        if !entry.header().entry_type().is_file() {
            return Err(PackError::Invalid(
                "registry collection transport contains a link, directory, or special entry".into(),
            ));
        }
        let relative = entry
            .path()?
            .to_str()
            .ok_or_else(|| PackError::Invalid("registry collection paths must be UTF-8".into()))?
            .to_owned();
        validate_relative_path(&relative)?;
        if !paths.insert(relative.clone()) {
            return Err(PackError::Invalid(format!(
                "duplicate registry collection path: {relative}"
            )));
        }
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(PackError::Invalid(format!(
                "registry collection file exceeds {MAX_FILE_BYTES} bytes: {relative}"
            )));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| PackError::Invalid("registry extracted size overflow".into()))?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(PackError::Invalid(format!(
                "registry extracted files exceed {MAX_TOTAL_BYTES} bytes"
            )));
        }
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)?;
        let copied = std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
        if copied != size {
            return Err(PackError::Invalid(format!(
                "registry collection file length mismatch: {relative}"
            )));
        }
        apply_archive_permissions(&target, entry.header().mode().unwrap_or(0))?;
    }
    if !paths.contains(COLLECTION_MANIFEST) {
        return Err(PackError::Invalid(format!(
            "registry collection transport is missing {COLLECTION_MANIFEST}"
        )));
    }
    Ok(())
}

fn materialize_pack_source(source: &Path) -> Result<MaterializedPack, PackError> {
    let root = verified_root(source)?;
    if fs::symlink_metadata(root.join(PACK_MANIFEST)).is_ok() {
        return Ok(MaterializedPack {
            root,
            _temporary: None,
        });
    }
    if fs::symlink_metadata(root.join("oci-layout")).is_err()
        || fs::symlink_metadata(root.join("index.json")).is_err()
    {
        return Err(PackError::Invalid(
            "pack source must be a pack directory or local OCI layout".into(),
        ));
    }
    let temporary = tempfile::Builder::new()
        .prefix("colossus-oci-pack-")
        .tempdir()?;
    extract_oci_layout(&root, temporary.path())?;
    let pack_root = locate_extracted_pack(temporary.path())?;
    Ok(MaterializedPack {
        root: pack_root,
        _temporary: Some(temporary),
    })
}

fn extract_oci_layout(source: &Path, destination: &Path) -> Result<(), PackError> {
    let layout: OciLayout = read_bounded_json(source, "oci-layout")?;
    if layout.image_layout_version != "1.0.0" {
        return Err(PackError::Invalid(
            "OCI imageLayoutVersion must be exactly 1.0.0".into(),
        ));
    }
    let index: OciIndex = read_bounded_json(source, "index.json")?;
    if index.schema_version != 2 || index.manifests.len() != 1 {
        return Err(PackError::Invalid(
            "OCI index must use schemaVersion 2 and contain exactly one manifest".into(),
        ));
    }
    if index
        .media_type
        .as_deref()
        .is_some_and(|value| value != "application/vnd.oci.image.index.v1+json")
    {
        return Err(PackError::Invalid("unsupported OCI index mediaType".into()));
    }
    validate_optional_text("OCI index artifactType", index.artifact_type.as_deref())?;
    validate_annotations("OCI index annotations", &index.annotations)?;
    let manifest_descriptor = &index.manifests[0];
    validate_oci_descriptor(manifest_descriptor)?;
    if manifest_descriptor.media_type != OCI_MANIFEST_MEDIA_TYPE {
        return Err(PackError::Invalid(
            "OCI index descriptor must name an OCI image manifest".into(),
        ));
    }
    let manifest_path = oci_blob(source, manifest_descriptor, MAX_MANIFEST_BYTES)?;
    let manifest: OciManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest.schema_version != 2 || manifest.layers.is_empty() {
        return Err(PackError::Invalid(
            "OCI manifest must use schemaVersion 2 and contain a layer".into(),
        ));
    }
    if manifest
        .media_type
        .as_deref()
        .is_some_and(|value| value != OCI_MANIFEST_MEDIA_TYPE)
    {
        return Err(PackError::Invalid(
            "unsupported OCI manifest mediaType".into(),
        ));
    }
    validate_optional_text(
        "OCI manifest artifactType",
        manifest.artifact_type.as_deref(),
    )?;
    validate_annotations("OCI manifest annotations", &manifest.annotations)?;
    if manifest.subject.is_some() {
        return Err(PackError::Invalid(
            "OCI pack manifests cannot use an external subject descriptor".into(),
        ));
    }
    if let Some(config) = &manifest.config {
        validate_oci_descriptor(config)?;
        let _ = oci_blob(source, config, MAX_MANIFEST_BYTES)?;
    }
    let layer = manifest
        .layers
        .iter()
        .find(|descriptor| {
            OCI_TAR_MEDIA_TYPES.contains(&descriptor.media_type.as_str())
                || OCI_GZIP_MEDIA_TYPES.contains(&descriptor.media_type.as_str())
        })
        .ok_or_else(|| PackError::Invalid("OCI manifest has no supported pack layer".into()))?;
    validate_oci_descriptor(layer)?;
    let layer_path = oci_blob(source, layer, MAX_ARCHIVE_BYTES)?;
    extract_pack_layer(
        &layer_path,
        destination,
        OCI_GZIP_MEDIA_TYPES.contains(&layer.media_type.as_str()),
    )
}

fn read_bounded_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    relative: &str,
) -> Result<T, PackError> {
    let path = root.join(relative);
    reject_symlink_chain(root, &path)?;
    read_manifest(&path)
}

fn validate_oci_descriptor(descriptor: &OciDescriptor) -> Result<(), PackError> {
    validate_bounded("OCI descriptor mediaType", &descriptor.media_type, 256)?;
    let _ = oci_digest(&descriptor.digest)?;
    if descriptor.size == 0 || descriptor.size > MAX_ARCHIVE_BYTES {
        return Err(PackError::Invalid(
            "OCI descriptor size is zero or exceeds the archive bound".into(),
        ));
    }
    if !descriptor.urls.is_empty() {
        return Err(PackError::Invalid(
            "offline OCI descriptors cannot contain remote URLs".into(),
        ));
    }
    validate_annotations("OCI descriptor annotations", &descriptor.annotations)?;
    if descriptor.platform.as_ref().is_some_and(|platform| {
        serde_json::to_vec(platform).map_or(true, |bytes| bytes.len() > 4096)
    }) {
        return Err(PackError::Invalid(
            "OCI descriptor platform metadata exceeds 4096 bytes".into(),
        ));
    }
    Ok(())
}

fn validate_annotations(
    label: &str,
    annotations: &BTreeMap<String, String>,
) -> Result<(), PackError> {
    if annotations.len() > 128 {
        return Err(PackError::Invalid(format!("{label} exceeds 128 entries")));
    }
    for (key, value) in annotations {
        validate_bounded(label, key, 256)?;
        validate_bounded(label, value, 4096)?;
    }
    Ok(())
}

fn validate_optional_text(label: &str, value: Option<&str>) -> Result<(), PackError> {
    if let Some(value) = value {
        validate_bounded(label, value, 256)?;
    }
    Ok(())
}

fn oci_digest(value: &str) -> Result<&str, PackError> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or_else(|| PackError::Invalid("OCI descriptors must use sha256 digests".into()))?;
    validate_sha256(digest)?;
    Ok(digest)
}

fn oci_blob(root: &Path, descriptor: &OciDescriptor, max_bytes: u64) -> Result<PathBuf, PackError> {
    let digest = oci_digest(&descriptor.digest)?;
    let path = root.join("blobs").join("sha256").join(digest);
    reject_symlink_chain(root, &path)?;
    let metadata = checked_regular_file(&path)?;
    if metadata.len() != descriptor.size || metadata.len() > max_bytes {
        return Err(PackError::Invalid(format!(
            "OCI blob size mismatch or bound exceeded: {}",
            descriptor.digest
        )));
    }
    if hash_file(&path, max_bytes)? != digest {
        return Err(PackError::Invalid(format!(
            "OCI blob hash mismatch: {}",
            descriptor.digest
        )));
    }
    Ok(path)
}

fn extract_pack_layer(path: &Path, destination: &Path, gzip: bool) -> Result<(), PackError> {
    let file = fs::File::open(path)?;
    let reader: Box<dyn Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut paths = BTreeSet::new();
    let mut total_bytes = 0_u64;
    let mut count = 0_usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        count = count.saturating_add(1);
        if count > MAX_FILES {
            return Err(PackError::Invalid(
                "OCI pack layer exceeds 10000 entries".into(),
            ));
        }
        let relative = entry
            .path()?
            .to_str()
            .ok_or_else(|| PackError::Invalid("OCI layer paths must be UTF-8".into()))?
            .to_owned();
        validate_relative_path(&relative)?;
        if !paths.insert(relative.clone()) {
            return Err(PackError::Invalid(format!(
                "duplicate OCI layer path: {relative}"
            )));
        }
        let target = destination.join(&relative);
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(PackError::Invalid(format!(
                "OCI pack layer contains a link or special entry: {relative}"
            )));
        }
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(PackError::Invalid(format!(
                "OCI layer file exceeds {MAX_FILE_BYTES} bytes: {relative}"
            )));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| PackError::Invalid("OCI extracted size overflow".into()))?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(PackError::Invalid(format!(
                "OCI extracted files exceed {MAX_TOTAL_BYTES} bytes"
            )));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let copied = std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
        if copied != size {
            return Err(PackError::Invalid(format!(
                "OCI layer file length mismatch: {relative}"
            )));
        }
        apply_archive_permissions(&target, entry.header().mode().unwrap_or(0))?;
    }
    Ok(())
}

fn apply_archive_permissions(path: &Path, mode: u32) -> Result<(), PackError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let safe_mode = if mode & 0o111 == 0 { 0o600 } else { 0o700 };
        fs::set_permissions(path, fs::Permissions::from_mode(safe_mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn locate_extracted_pack(root: &Path) -> Result<PathBuf, PackError> {
    if fs::symlink_metadata(root.join(PACK_MANIFEST)).is_ok() {
        return Ok(root.to_owned());
    }
    let children = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    if children.len() != 1 {
        return Err(PackError::Invalid(format!(
            "OCI layer must contain {PACK_MANIFEST} at its root or in one top-level directory"
        )));
    }
    let child = children[0].path();
    let metadata = fs::symlink_metadata(&child)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || fs::symlink_metadata(child.join(PACK_MANIFEST)).is_err()
    {
        return Err(PackError::Invalid(format!(
            "OCI layer is missing {PACK_MANIFEST}"
        )));
    }
    Ok(child)
}

fn verify_pack(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<PackVerification, PackError> {
    let root = verified_root(root)?;
    let manifest_path = root.join(PACK_MANIFEST);
    let manifest: PackManifest = read_manifest(&manifest_path)?;
    validate_pack_manifest(&manifest)?;
    let (files, total_bytes) = verify_declared_files(&root, &manifest.files)?;
    reject_undeclared_files(&root, &files, PACK_MANIFEST)?;
    validate_pack_references(&root, &manifest, &files)?;
    let unsigned = canonical_pack_signing_bytes(&manifest)?;
    let manifest_sha256 = digest_hex(&unsigned);
    let trust_key_id = verify_signatures(
        &manifest.publisher,
        &manifest.signatures,
        &unsigned,
        repository,
        false,
    )?;
    Ok(PackVerification {
        manifest,
        manifest_sha256,
        file_count: files.len(),
        total_bytes,
        trusted: trust_key_id.is_some(),
        trust_key_id,
    })
}

fn verify_collection(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<CollectionVerification, PackError> {
    let root = verified_root(root)?;
    let manifest: CollectionManifest = read_manifest(&root.join(COLLECTION_MANIFEST))?;
    validate_collection_manifest(&manifest)?;
    let (files, total_bytes) = verify_declared_files(&root, &manifest.files)?;
    reject_undeclared_files(&root, &files, COLLECTION_MANIFEST)?;
    let unsigned = canonical_collection_signing_bytes(&manifest)?;
    let manifest_sha256 = digest_hex(&unsigned);
    let trust_key_id = verify_signatures(
        &manifest.publisher,
        &manifest.signatures,
        &unsigned,
        repository,
        true,
    )?
    .ok_or_else(|| PackError::Invalid("collection must have a trusted signature".into()))?;

    let mut packs = Vec::new();
    let mut skills = Vec::new();
    for artifact in &manifest.artifacts {
        let path = root.join(&artifact.path);
        reject_symlink_chain(&root, &path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PackError::Invalid(format!(
                "collection artifact is not a real directory: {}",
                artifact.path
            )));
        }
        match artifact.kind {
            CollectionArtifactKind::Pack => {
                let verification = verify_pack(&path, repository)?;
                if !verification.trusted {
                    return Err(PackError::Invalid(format!(
                        "collection pack must have its own trusted signature: {}",
                        artifact.name
                    )));
                }
                if verification.manifest.name != artifact.name
                    || verification.manifest.version != artifact.version
                    || verification.manifest_sha256 != artifact.content_sha256
                {
                    return Err(PackError::Invalid(format!(
                        "collection pack identity does not match its inventory: {}",
                        artifact.path
                    )));
                }
                packs.push(verification);
            }
            CollectionArtifactKind::Skill => {
                let inspection =
                    inspect_skill_directory(&path, &format!("collection:{}", artifact.path))?;
                if inspection.manifest.name != artifact.name
                    || inspection.manifest.version != artifact.version
                    || inspection.content_sha256 != artifact.content_sha256
                {
                    return Err(PackError::Invalid(format!(
                        "collection skill identity does not match its inventory: {}",
                        artifact.path
                    )));
                }
                skills.push(SkillValidationResult {
                    name: inspection.manifest.name,
                    source: inspection.source,
                    file_count: inspection.files.len(),
                    content_sha256: inspection.content_sha256,
                });
            }
        }
    }
    let packs = order_collection_packs(packs)?;
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(CollectionVerification {
        manifest,
        manifest_sha256,
        file_count: files.len(),
        total_bytes,
        trust_key_id,
        packs,
        skills,
    })
}

fn verify_bundle(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<BundleVerification, PackError> {
    let root = verified_root(root)?;
    let manifest: BundleManifest = read_manifest(&root.join(BUNDLE_MANIFEST))?;
    if manifest.format_version != 1 {
        return Err(PackError::Invalid(
            "unsupported bundle format_version".into(),
        ));
    }
    validate_identity("bundle name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_bounded("bundle version", &manifest.version, 128)?;
    validate_bounded("bundle created_at", &manifest.created_at, 128)?;
    if !manifest.created_at.ends_with('Z') {
        return Err(PackError::Invalid(
            "bundle created_at must use the UTC Z designator".into(),
        ));
    }
    OffsetDateTime::parse(&manifest.created_at, &Rfc3339)
        .map_err(|_| PackError::Invalid("bundle created_at must be RFC3339 UTC".into()))?;
    if let Some(revision) = &manifest.source_revision {
        validate_bounded("bundle source_revision", revision, 256)?;
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "bundle files must contain 1..=10000 entries".into(),
        ));
    }
    let entries = manifest
        .files
        .iter()
        .map(|file| {
            validate_relative_path(&file.path)?;
            let path = root.join(&file.path);
            reject_symlink_chain(&root, &path)?;
            let metadata = checked_regular_file(&path)?;
            if let Some(size) = file.size
                && metadata.len() != size
            {
                return Err(PackError::Invalid(format!(
                    "bundle file size mismatch: {}",
                    file.path
                )));
            }
            Ok(colossus_contracts::PackFileEntry {
                path: file.path.clone(),
                sha256: file.sha256.clone(),
                size: metadata.len(),
                content_type: "application/octet-stream".into(),
            })
        })
        .collect::<Result<Vec<_>, PackError>>()?;
    let (files, total_bytes) = verify_declared_files(&root, &entries)?;
    reject_undeclared_files(&root, &files, BUNDLE_MANIFEST)?;
    let unsigned = canonical_bundle_signing_bytes(&manifest)?;
    let manifest_sha256 = digest_hex(&unsigned);
    let trust_key_id = verify_signatures(
        &manifest.publisher,
        &manifest.signatures,
        &unsigned,
        repository,
        true,
    )?
    .ok_or_else(|| PackError::Invalid("offline bundle must have a trusted signature".into()))?;
    Ok(BundleVerification {
        name: manifest.name,
        version: manifest.version,
        manifest_sha256,
        file_count: files.len(),
        total_bytes,
        trust_key_id,
        source_revision: manifest.source_revision,
    })
}

fn validate_collection_manifest(manifest: &CollectionManifest) -> Result<(), PackError> {
    if manifest.format_version != 1 {
        return Err(PackError::Invalid(
            "unsupported collection format_version".into(),
        ));
    }
    validate_identity("collection name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_bounded("collection version", &manifest.version, 128)?;
    validate_bundle_timestamp(&manifest.created_at)?;
    if manifest.artifacts.is_empty() || manifest.artifacts.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "collection artifacts must contain 1..=10000 entries".into(),
        ));
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "collection files must contain 1..=10000 entries".into(),
        ));
    }
    if manifest.signatures.is_empty() {
        return Err(PackError::Invalid(
            "collection signatures cannot be empty".into(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut previous = None::<&str>;
    for artifact in &manifest.artifacts {
        validate_identity("collection artifact name", &artifact.name)?;
        validate_bounded("collection artifact version", &artifact.version, 128)?;
        validate_relative_path(&artifact.path)?;
        validate_sha256(&artifact.content_sha256)?;
        let expected_root = match artifact.kind {
            CollectionArtifactKind::Pack => "packs",
            CollectionArtifactKind::Skill => "skills",
        };
        let components = artifact.path.split('/').collect::<Vec<_>>();
        if components.len() != 2 || components[0] != expected_root {
            return Err(PackError::Invalid(format!(
                "collection artifact path must be {expected_root}/NAME: {}",
                artifact.path
            )));
        }
        if !paths.insert(&artifact.path) {
            return Err(PackError::Invalid(format!(
                "duplicate collection artifact path: {}",
                artifact.path
            )));
        }
        let kind = match artifact.kind {
            CollectionArtifactKind::Pack => "pack",
            CollectionArtifactKind::Skill => "skill",
        };
        if !identities.insert(format!("{kind}:{}", artifact.name)) {
            return Err(PackError::Invalid(format!(
                "duplicate collection artifact identity: {kind}:{}",
                artifact.name
            )));
        }
        if previous.is_some_and(|value| value >= artifact.path.as_str()) {
            return Err(PackError::Invalid(
                "collection artifacts must be sorted by unique path".into(),
            ));
        }
        previous = Some(&artifact.path);
    }
    let mut previous_file = None::<&str>;
    let mut artifact_files = BTreeMap::<&str, usize>::new();
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        validate_sha256(&file.sha256)?;
        validate_bounded("content_type", &file.content_type, 256)?;
        if previous_file.is_some_and(|value| value >= file.path.as_str()) {
            return Err(PackError::Invalid(
                "collection files must be sorted by unique path".into(),
            ));
        }
        previous_file = Some(&file.path);
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| {
                file.path
                    .strip_prefix(&artifact.path)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .ok_or_else(|| {
                PackError::Invalid(format!(
                    "collection file is outside every declared artifact: {}",
                    file.path
                ))
            })?;
        *artifact_files.entry(&artifact.path).or_default() += 1;
    }
    if manifest
        .artifacts
        .iter()
        .any(|artifact| !artifact_files.contains_key(artifact.path.as_str()))
    {
        return Err(PackError::Invalid(
            "every collection artifact must contain at least one file".into(),
        ));
    }
    Ok(())
}

fn discover_collection_artifacts(
    root: &Path,
    repository: &dyn ExtensionRepository,
) -> Result<Vec<CollectionArtifactEntry>, PackError> {
    let mut artifacts = Vec::new();
    for (directory, kind) in [
        ("packs", CollectionArtifactKind::Pack),
        ("skills", CollectionArtifactKind::Skill),
    ] {
        let container = root.join(directory);
        if !container.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&container)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PackError::Invalid(format!(
                "collection {directory} root is not a real directory"
            )));
        }
        let mut entries = fs::read_dir(&container)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PackError::Invalid(format!(
                    "collection {directory} entries must be real directories: {}",
                    path.display()
                )));
            }
            let relative = normalized_relative(root, &path)?;
            match kind {
                CollectionArtifactKind::Pack => {
                    let verification = verify_pack(&path, repository)?;
                    if !verification.trusted {
                        return Err(PackError::Invalid(format!(
                            "collection pack must have its own trusted signature: {}",
                            verification.manifest.name
                        )));
                    }
                    artifacts.push(CollectionArtifactEntry {
                        kind,
                        name: verification.manifest.name,
                        version: verification.manifest.version,
                        path: relative,
                        content_sha256: verification.manifest_sha256,
                    });
                }
                CollectionArtifactKind::Skill => {
                    let inspection = inspect_skill_directory(&path, "collection-build")?;
                    artifacts.push(CollectionArtifactEntry {
                        kind,
                        name: inspection.manifest.name,
                        version: inspection.manifest.version,
                        path: relative,
                        content_sha256: inspection.content_sha256,
                    });
                }
            }
        }
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn collect_collection_entries(root: &Path) -> Result<Vec<PackFileEntry>, PackError> {
    collect_bundle_entries(root)?
        .into_iter()
        .map(|entry| {
            Ok(PackFileEntry {
                path: entry.path,
                sha256: entry.sha256,
                size: entry
                    .size
                    .ok_or_else(|| PackError::Invalid("collection file size is absent".into()))?,
                content_type: "application/octet-stream".into(),
            })
        })
        .collect()
}

fn order_collection_packs(
    packs: Vec<PackVerification>,
) -> Result<Vec<PackVerification>, PackError> {
    fn visit(
        name: &str,
        packs: &BTreeMap<String, PackVerification>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<PackVerification>,
    ) -> Result<(), PackError> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.into()) {
            return Err(PackError::Invalid(format!(
                "collection pack dependency cycle includes {name}"
            )));
        }
        let pack = packs
            .get(name)
            .ok_or_else(|| PackError::Invalid(format!("collection pack is absent: {name}")))?;
        for dependency in &pack.manifest.dependencies {
            let (dependency_name, version) = dependency.split_once('@').ok_or_else(|| {
                PackError::Invalid(format!(
                    "pack dependency must be name@version: {dependency}"
                ))
            })?;
            let dependency_pack = packs.get(dependency_name).ok_or_else(|| {
                PackError::Invalid(format!(
                    "collection is missing dependency closure entry: {dependency}"
                ))
            })?;
            if dependency_pack.manifest.version != version {
                return Err(PackError::Invalid(format!(
                    "collection dependency has the wrong exact version: {dependency}"
                )));
            }
            visit(dependency_name, packs, visiting, visited, ordered)?;
        }
        visiting.remove(name);
        visited.insert(name.into());
        ordered.push(pack.clone());
        Ok(())
    }

    let mut by_name = BTreeMap::new();
    for pack in packs {
        let name = pack.manifest.name.clone();
        if by_name.insert(name.clone(), pack).is_some() {
            return Err(PackError::Invalid(format!(
                "duplicate collection pack identity: {name}"
            )));
        }
    }
    let names = by_name.keys().cloned().collect::<Vec<_>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::with_capacity(by_name.len());
    for name in names {
        visit(&name, &by_name, &mut visiting, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}

fn validate_bundle_timestamp(created_at: &str) -> Result<(), PackError> {
    validate_bounded("bundle created_at", created_at, 128)?;
    if !created_at.ends_with('Z') {
        return Err(PackError::Invalid(
            "bundle created_at must use the UTC Z designator".into(),
        ));
    }
    OffsetDateTime::parse(created_at, &Rfc3339)
        .map(|_| ())
        .map_err(|_| PackError::Invalid("bundle created_at must be RFC3339 UTC".into()))
}

fn copy_bundle_payload(source: &Path, destination: &Path) -> Result<(), PackError> {
    fn copy_directory(source: &Path, destination: &Path) -> Result<(), PackError> {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let before = fs::symlink_metadata(&source_path)?;
            if before.file_type().is_symlink() {
                return Err(PackError::Invalid(format!(
                    "symlink is forbidden: {}",
                    source_path.display()
                )));
            }
            if before.is_dir() {
                fs::create_dir(&destination_path)?;
                copy_directory(&source_path, &destination_path)?;
            } else if before.is_file() {
                fs::copy(&source_path, &destination_path)?;
                let after = fs::symlink_metadata(&source_path)?;
                if after.file_type().is_symlink()
                    || !after.is_file()
                    || after.len() != before.len()
                    || hash_file(&source_path, MAX_FILE_BYTES)?
                        != hash_file(&destination_path, MAX_FILE_BYTES)?
                {
                    return Err(PackError::Invalid(format!(
                        "bundle source changed while it was copied: {}",
                        source_path.display()
                    )));
                }
            } else {
                return Err(PackError::Invalid(format!(
                    "special filesystem entry is forbidden: {}",
                    source_path.display()
                )));
            }
        }
        Ok(())
    }
    copy_directory(source, destination)
}

fn collect_bundle_entries(root: &Path) -> Result<Vec<BundleFileEntry>, PackError> {
    fn collect(
        root: &Path,
        directory: &Path,
        entries: &mut Vec<BundleFileEntry>,
        total: &mut u64,
    ) -> Result<(), PackError> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(PackError::Invalid(format!(
                    "symlink is forbidden: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                collect(root, &path, entries, total)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(PackError::Invalid(format!(
                        "bundle file exceeds {MAX_FILE_BYTES} bytes: {}",
                        path.display()
                    )));
                }
                *total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| PackError::Invalid("bundle payload size overflow".into()))?;
                if *total > MAX_TOTAL_BYTES {
                    return Err(PackError::Invalid(format!(
                        "bundle payload exceeds {MAX_TOTAL_BYTES} bytes"
                    )));
                }
                entries.push(BundleFileEntry {
                    path: normalized_relative(root, &path)?,
                    sha256: hash_file(&path, MAX_FILE_BYTES)?,
                    size: Some(metadata.len()),
                });
                if entries.len() > MAX_FILES {
                    return Err(PackError::Invalid(format!(
                        "bundle contains more than {MAX_FILES} files"
                    )));
                }
            } else {
                return Err(PackError::Invalid(format!(
                    "special filesystem entry is forbidden: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    let mut entries = Vec::new();
    let mut total = 0;
    collect(root, root, &mut entries, &mut total)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    if entries.is_empty() {
        return Err(PackError::Invalid(
            "bundle payload must contain at least one file".into(),
        ));
    }
    Ok(entries)
}

fn bundle_artifact_path(target: &str) -> String {
    format!(
        "artifacts/{target}/{}",
        if target.ends_with("windows-msvc") {
            "colossus.exe"
        } else {
            "colossus"
        }
    )
}

fn installable_bundle_targets(files: &[BundleFileEntry]) -> Vec<String> {
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    RELEASE_TARGETS
        .iter()
        .filter(|target| paths.contains(bundle_artifact_path(target).as_str()))
        .map(|target| (*target).to_owned())
        .collect()
}

fn validate_absolute_normalized(path: &Path, label: &str) -> Result<(), PackError> {
    if !path.is_absolute()
        || path.components().any(|component| {
            !matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
    {
        return Err(PackError::Invalid(format!(
            "{label} must be absolute and normalized: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_real_directory(path: &Path, label: &str) -> Result<PathBuf, PackError> {
    validate_absolute_normalized(path, label)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PackError::Invalid(format!(
                    "{label} must be a real directory: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                PackError::Invalid(format!("{label} has no parent: {}", path.display()))
            })?;
            ensure_real_directory(parent, label)?;
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PackError::Invalid(format!(
                    "{label} became unsafe while it was created: {}",
                    path.display()
                )));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(fs::canonicalize(path)?)
}

fn set_executable_permissions(path: &Path) -> Result<(), PackError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn read_manifest<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, PackError> {
    let metadata = checked_regular_file(path)?;
    if metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES {
        return Err(PackError::Invalid(format!(
            "manifest must be in 1..={MAX_MANIFEST_BYTES} bytes"
        )));
    }
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_pack_manifest(manifest: &PackManifest) -> Result<(), PackError> {
    if manifest.format_version != 1 {
        return Err(PackError::Invalid("unsupported pack format_version".into()));
    }
    validate_identity("pack name", &manifest.name)?;
    validate_identity("publisher", &manifest.publisher)?;
    validate_bounded("pack version", &manifest.version, 128)?;
    validate_bounded("pack description", &manifest.description, MAX_TEXT_BYTES)?;
    validate_bounded("pack license", &manifest.license, 128)?;
    if !manifest.homepage.is_empty() {
        validate_bounded("pack homepage", &manifest.homepage, 2048)?;
        let homepage = url::Url::parse(&manifest.homepage)
            .map_err(|_| PackError::Invalid("pack homepage must be an absolute URL".into()))?;
        if !matches!(homepage.scheme(), "https" | "http")
            || homepage.host_str().is_none()
            || !homepage.username().is_empty()
            || homepage.password().is_some()
        {
            return Err(PackError::Invalid(
                "pack homepage must be HTTP(S), have a host, and contain no credentials".into(),
            ));
        }
    }
    if manifest.files.is_empty() || manifest.files.len() > MAX_FILES {
        return Err(PackError::Invalid(
            "pack files must contain 1..=10000 entries".into(),
        ));
    }
    let capabilities = unique_values("capabilities", &manifest.capabilities)?;
    let known = BTreeSet::from([
        "integrations",
        "skills",
        "tools",
        "mcp_servers",
        "binaries",
        "docker",
        "docs",
        "tests",
    ]);
    if let Some(value) = capabilities
        .iter()
        .find(|value| !known.contains(value.as_str()))
    {
        return Err(PackError::Invalid(format!(
            "unknown pack capability {value}"
        )));
    }
    unique_values("permissions", &manifest.permissions)?;
    let known_permissions = BTreeSet::from([
        "process",
        "network",
        "filesystem.read",
        "filesystem.write",
        "credentials",
    ]);
    if let Some(permission) = manifest
        .permissions
        .iter()
        .find(|permission| !known_permissions.contains(permission.as_str()))
    {
        return Err(PackError::Invalid(format!(
            "unknown pack permission {permission}"
        )));
    }
    unique_values("dependencies", &manifest.dependencies)?;
    for dependency in &manifest.dependencies {
        let Some((name, version)) = dependency.split_once('@') else {
            return Err(PackError::Invalid(format!(
                "pack dependency must be name@version: {dependency}"
            )));
        };
        validate_identity("dependency name", name)?;
        validate_bounded("dependency version", version, 128)?;
    }
    Ok(())
}

fn verify_declared_files(
    root: &Path,
    entries: &[colossus_contracts::PackFileEntry],
) -> Result<(BTreeSet<String>, u64), PackError> {
    let mut files = BTreeSet::new();
    let mut total = 0_u64;
    for entry in entries {
        validate_relative_path(&entry.path)?;
        if !files.insert(entry.path.clone()) {
            return Err(PackError::Invalid(format!(
                "duplicate file declaration: {}",
                entry.path
            )));
        }
        if entry.size > MAX_FILE_BYTES {
            return Err(PackError::Invalid(format!(
                "declared file exceeds {MAX_FILE_BYTES} bytes: {}",
                entry.path
            )));
        }
        validate_sha256(&entry.sha256)?;
        validate_bounded("content_type", &entry.content_type, 256)?;
        let path = root.join(&entry.path);
        reject_symlink_chain(root, &path)?;
        let metadata = checked_regular_file(&path)?;
        if metadata.len() != entry.size {
            return Err(PackError::Invalid(format!(
                "file size mismatch: {}",
                entry.path
            )));
        }
        if hash_file(&path, MAX_FILE_BYTES)? != entry.sha256 {
            return Err(PackError::Invalid(format!(
                "file hash mismatch: {}",
                entry.path
            )));
        }
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| PackError::Invalid("declared file size overflow".into()))?;
        if total > MAX_TOTAL_BYTES {
            return Err(PackError::Invalid(format!(
                "declared files exceed {MAX_TOTAL_BYTES} bytes"
            )));
        }
    }
    Ok((files, total))
}

fn validate_pack_references(
    root: &Path,
    manifest: &PackManifest,
    files: &BTreeSet<String>,
) -> Result<(), PackError> {
    #[cfg(not(unix))]
    let _ = root;
    let permissions = manifest.permissions.iter().collect::<BTreeSet<_>>();
    let capabilities = manifest
        .capabilities
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for (capability, present) in [
        ("integrations", !manifest.integrations.is_empty()),
        ("skills", !manifest.skills.is_empty()),
        ("tools", !manifest.tools.is_empty()),
        ("mcp_servers", !manifest.mcp_servers.is_empty()),
        ("binaries", !manifest.binaries.is_empty()),
        ("docker", !manifest.docker.is_empty()),
        ("docs", !manifest.docs.is_empty()),
        ("tests", !manifest.tests.is_empty()),
    ] {
        if capabilities.contains(capability) != present {
            return Err(PackError::Invalid(format!(
                "capability {capability} must exactly match its declared contributions"
            )));
        }
    }
    for path in manifest
        .integrations
        .iter()
        .map(|value| value.path.as_str())
        .chain(manifest.binaries.iter().map(String::as_str))
        .chain(manifest.docker.iter().map(String::as_str))
        .chain(manifest.docs.iter().map(String::as_str))
        .chain(manifest.tests.iter().map(String::as_str))
    {
        validate_relative_path(path)?;
        if !files.contains(path) {
            return Err(PackError::Invalid(format!(
                "referenced pack file is not hash-listed: {path}"
            )));
        }
    }
    for skill in &manifest.skills {
        validate_relative_path(&skill.path)?;
        let prefix = format!("{}/", skill.path.trim_end_matches('/'));
        if !files.iter().any(|path| path.starts_with(&prefix))
            || !files.contains(&format!("{}SKILL.md", prefix))
        {
            return Err(PackError::Invalid(format!(
                "skill {} must contain a hash-listed SKILL.md",
                skill.path
            )));
        }
    }
    let mut tool_names = BTreeSet::new();
    for tool in &manifest.tools {
        validate_identity("tool name", &tool.name)?;
        if !tool_names.insert(&tool.name) {
            return Err(PackError::Invalid(format!(
                "duplicate pack tool name {}",
                tool.name
            )));
        }
        validate_command(&tool.command, files)?;
        if !manifest.binaries.contains(&tool.command) {
            return Err(PackError::Invalid(format!(
                "tool command {} must also be declared in binaries",
                tool.command
            )));
        }
        validate_executable_permissions(&tool.permissions, &permissions)?;
        validate_env_refs(&tool.env_refs)?;
        if !tool.env_refs.is_empty()
            && !tool
                .permissions
                .iter()
                .any(|permission| permission == "credentials")
        {
            return Err(PackError::Invalid(format!(
                "tool {} uses credential refs without the credentials permission",
                tool.name
            )));
        }
    }
    let mut server_names = BTreeSet::new();
    for server in &manifest.mcp_servers {
        validate_identity("MCP server name", &server.name)?;
        if !server_names.insert(&server.name) {
            return Err(PackError::Invalid(format!(
                "duplicate pack MCP server name {}",
                server.name
            )));
        }
        validate_command(&server.command, files)?;
        if !manifest.binaries.contains(&server.command) {
            return Err(PackError::Invalid(format!(
                "MCP command {} must also be declared in binaries",
                server.command
            )));
        }
        validate_executable_permissions(&server.permissions, &permissions)?;
        validate_env_refs(&server.env_refs)?;
        if !server.env_refs.is_empty()
            && !server
                .permissions
                .iter()
                .any(|permission| permission == "credentials")
        {
            return Err(PackError::Invalid(format!(
                "MCP server {} uses credential refs without the credentials permission",
                server.name
            )));
        }
        if server.allowed_tools.is_empty() {
            return Err(PackError::Invalid(format!(
                "MCP server {} has an empty tool allowlist",
                server.name
            )));
        }
        unique_values("MCP allowed_tools", &server.allowed_tools)?;
    }
    for binary in &manifest.binaries {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            if fs::metadata(root.join(binary))?.permissions().mode() & 0o111 == 0 {
                return Err(PackError::Invalid(format!(
                    "declared binary is not executable: {binary}"
                )));
            }
        }
        if !manifest.tools.iter().any(|tool| &tool.command == binary)
            && !manifest
                .mcp_servers
                .iter()
                .any(|server| &server.command == binary)
        {
            return Err(PackError::Invalid(format!(
                "binary {binary} is not bound to a declared tool or MCP server"
            )));
        }
    }
    Ok(())
}

fn validate_command(command: &str, files: &BTreeSet<String>) -> Result<(), PackError> {
    validate_relative_path(command)?;
    if !files.contains(command) {
        return Err(PackError::Invalid(format!(
            "executable command is not hash-listed: {command}"
        )));
    }
    Ok(())
}

fn validate_executable_permissions(
    requested: &[String],
    pack_permissions: &BTreeSet<&String>,
) -> Result<(), PackError> {
    if requested.is_empty() {
        return Err(PackError::Invalid(
            "executable tools and MCP servers must declare permissions".into(),
        ));
    }
    unique_values("executable permissions", requested)?;
    if !requested.iter().any(|permission| permission == "process") {
        return Err(PackError::Invalid(
            "executable tools and MCP servers require the process permission".into(),
        ));
    }
    if let Some(permission) = requested
        .iter()
        .find(|permission| !pack_permissions.contains(permission))
    {
        return Err(PackError::Invalid(format!(
            "executable permission {permission} exceeds the pack permission ceiling"
        )));
    }
    Ok(())
}

fn validate_env_refs(env_refs: &BTreeMap<String, String>) -> Result<(), PackError> {
    for (name, reference) in env_refs {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            || !reference.starts_with("env:")
            || reference.len() <= 4
        {
            return Err(PackError::Invalid(format!(
                "invalid environment credential reference for {name}"
            )));
        }
    }
    Ok(())
}

/// Produce deterministic bytes signed by capability-pack publishers.
pub fn canonical_pack_signing_bytes(manifest: &PackManifest) -> Result<Vec<u8>, PackError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    canonical_json(&unsigned)
}

/// Produce deterministic bytes signed by offline-bundle publishers.
pub fn canonical_bundle_signing_bytes(manifest: &BundleManifest) -> Result<Vec<u8>, PackError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    canonical_json(&unsigned)
}

/// Deterministic signed bytes for a collection manifest with signatures removed.
pub fn canonical_collection_signing_bytes(
    manifest: &CollectionManifest,
) -> Result<Vec<u8>, PackError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    canonical_json(&unsigned)
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, PackError> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            serde_json::Value::Object(values) => {
                let values = values
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect::<BTreeMap<_, _>>();
                serde_json::Value::Object(values.into_iter().collect())
            }
            value => value,
        }
    }
    Ok(serde_json::to_vec(&sorted(serde_json::to_value(value)?))?)
}

fn verify_signatures(
    publisher: &str,
    signatures: &[PackSignature],
    message: &[u8],
    repository: &dyn ExtensionRepository,
    require_signature: bool,
) -> Result<Option<String>, PackError> {
    if signatures.is_empty() {
        if require_signature {
            return Err(PackError::Invalid("a trusted signature is required".into()));
        }
        return Ok(None);
    }
    let mut authenticated = None;
    let mut key_ids = BTreeSet::new();
    for pack_signature in signatures {
        if pack_signature.algorithm != "ed25519" {
            return Err(PackError::Invalid(format!(
                "unsupported signature algorithm {}",
                pack_signature.algorithm
            )));
        }
        validate_sha256(&pack_signature.key_id)?;
        if !key_ids.insert(&pack_signature.key_id) {
            return Err(PackError::Invalid(format!(
                "duplicate signature key {}",
                pack_signature.key_id
            )));
        }
        let trust = repository
            .get_publisher_trust(publisher, &pack_signature.key_id)?
            .ok_or_else(|| {
                PackError::Invalid(format!(
                    "signature key {} is not trusted for publisher {publisher}",
                    pack_signature.key_id
                ))
            })?;
        let public = BASE64
            .decode(&trust.public_key)
            .map_err(|_| PackError::Invalid("stored publisher public key is invalid".into()))?;
        let public: [u8; 32] = public.try_into().map_err(|_| {
            PackError::Invalid("stored publisher public key has an invalid size".into())
        })?;
        if digest_hex(&public) != pack_signature.key_id {
            return Err(PackError::Invalid(
                "stored publisher key does not match its key_id".into(),
            ));
        }
        let verifying_key = VerifyingKey::from_bytes(&public)
            .map_err(|_| PackError::Invalid("stored publisher public key is invalid".into()))?;
        let signature_bytes = BASE64
            .decode(&pack_signature.signature)
            .map_err(|_| PackError::Invalid("signature must be base64".into()))?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| PackError::Invalid("Ed25519 signature has an invalid size".into()))?;
        verifying_key
            .verify(message, &signature)
            .map_err(|_| PackError::Invalid("Ed25519 signature verification failed".into()))?;
        authenticated = Some(pack_signature.key_id.clone());
    }
    Ok(authenticated)
}

fn verified_root(root: &Path) -> Result<PathBuf, PackError> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PackError::Invalid(
            "pack or bundle root must be a real directory, not a symlink".into(),
        ));
    }
    Ok(fs::canonicalize(root)?)
}

fn checked_regular_file(path: &Path) -> Result<fs::Metadata, PackError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PackError::Invalid(format!(
            "expected regular non-symlink file: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

fn reject_symlink_chain(root: &Path, path: &Path) -> Result<(), PackError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        PackError::Invalid(format!("path escapes trusted root: {}", path.display()))
    })?;
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(PackError::Invalid("path is not normalized".into()));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(PackError::Invalid(format!(
                "symlink is forbidden: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn reject_undeclared_files(
    root: &Path,
    declared: &BTreeSet<String>,
    manifest_name: &str,
) -> Result<(), PackError> {
    fn visit(
        root: &Path,
        directory: &Path,
        declared: &BTreeSet<String>,
        manifest_name: &str,
    ) -> Result<(), PackError> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(PackError::Invalid(format!(
                    "symlink is forbidden: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                visit(root, &path, declared, manifest_name)?;
            } else if metadata.is_file() {
                let relative = normalized_relative(root, &path)?;
                if relative != manifest_name && !declared.contains(&relative) {
                    return Err(PackError::Invalid(format!(
                        "undeclared payload file: {relative}"
                    )));
                }
            } else {
                return Err(PackError::Invalid(format!(
                    "special filesystem entry is forbidden: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
    visit(root, root, declared, manifest_name)
}

fn normalized_relative(root: &Path, path: &Path) -> Result<String, PackError> {
    path.strip_prefix(root)
        .map_err(|_| PackError::Invalid("payload path escapes root".into()))?
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| PackError::Invalid("payload path must contain valid UTF-8".into())),
            _ => Err(PackError::Invalid("payload path is not normalized".into())),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn validate_relative_path(path: &str) -> Result<(), PackError> {
    if path.is_empty() || path.len() > 1024 || path.contains('\\') || path.contains('\0') {
        return Err(PackError::Invalid(format!("invalid relative path: {path}")));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PackError::Invalid(format!(
            "path must be normalized and relative: {path}"
        )));
    }
    Ok(())
}

fn validate_identity(label: &str, value: &str) -> Result<(), PackError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(PackError::Invalid(format!("invalid {label}: {value}")));
    }
    Ok(())
}

fn validate_bounded(label: &str, value: &str, max: usize) -> Result<(), PackError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(PackError::Invalid(format!(
            "{label} must contain 1..={max} non-control bytes"
        )));
    }
    Ok(())
}

fn unique_values<'a>(label: &str, values: &'a [String]) -> Result<BTreeSet<&'a String>, PackError> {
    let mut unique = BTreeSet::new();
    for value in values {
        validate_bounded(label, value, 256)?;
        if !unique.insert(value) {
            return Err(PackError::Invalid(format!(
                "duplicate {label} value: {value}"
            )));
        }
    }
    Ok(unique)
}

fn validate_sha256(value: &str) -> Result<(), PackError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PackError::Invalid(
            "SHA-256 digests must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn hash_file(path: &Path, max_bytes: u64) -> Result<String, PackError> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| PackError::Invalid("file size overflow".into()))?;
        if total > max_bytes {
            return Err(PackError::Invalid(format!(
                "file exceeds {max_bytes} bytes: {}",
                path.display()
            )));
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn ensure_install_root(root: &Path) -> Result<PathBuf, PackError> {
    if fs::symlink_metadata(root).is_err() {
        fs::create_dir_all(root)?;
    }
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !root.is_absolute() {
        return Err(PackError::Invalid(
            "pack install root must be an absolute real directory".into(),
        ));
    }
    Ok(fs::canonicalize(root)?)
}

fn copy_verified_pack(
    source: &Path,
    destination: &Path,
    manifest: &PackManifest,
) -> Result<(), PackError> {
    fs::copy(source.join(PACK_MANIFEST), destination.join(PACK_MANIFEST))?;
    for entry in &manifest.files {
        let target = destination.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source.join(&entry.path), target)?;
    }
    Ok(())
}

fn now() -> Result<String, PackError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| PackError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{
        BUNDLE_MANIFEST, COLLECTION_MANIFEST, PACK_MANIFEST, PackError, PackExecutor,
        PackOperation, PackService, RELEASE_TARGETS, bundle_artifact_path,
        canonical_bundle_signing_bytes, canonical_collection_signing_bytes,
        canonical_pack_signing_bytes, current_release_target, digest_hex,
        extract_collection_archive, write_collection_archive,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use colossus_contracts::{
        Actor, ActorType, BundleFileEntry, BundleManifest, CredentialReference, DecisionOutcome,
        PackFileEntry, PackManifest, PackPathReference, PackSignature, PackStatus, PublisherTrust,
        SkillManifest,
    };
    use colossus_integrations::EventSourcedExtensionRepository;
    use colossus_policy::{
        BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, effect_request,
    };
    use colossus_ports::{EventJournal, ExtensionRepository};
    use colossus_testkit::InMemoryEventJournal;
    use ed25519_dalek::{Signer as _, SigningKey};
    use sha2::{Digest as _, Sha256};
    use std::{
        fs,
        io::Write as _,
        path::Path,
        sync::{Arc, Mutex},
    };
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    fn actor() -> Actor {
        Actor {
            actor_type: ActorType::User,
            id: "pack-test".into(),
        }
    }

    fn repository() -> (
        Arc<InMemoryEventJournal>,
        Arc<EventSourcedExtensionRepository>,
    ) {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository = Arc::new(EventSourcedExtensionRepository::new(
            Arc::clone(&journal) as Arc<dyn EventJournal>
        ));
        (journal, repository)
    }

    fn write_pack(root: &Path) -> PackManifest {
        let docs = root.join("docs");
        fs::create_dir_all(&docs).expect("create docs");
        let body = b"verified pack documentation\n";
        fs::write(docs.join("README.md"), body).expect("write pack body");
        let manifest = PackManifest {
            format_version: 1,
            name: "demo-pack".into(),
            version: "0.1.0".into(),
            description: "A strict test pack.".into(),
            publisher: "example".into(),
            license: "Apache-2.0".into(),
            homepage: String::new(),
            capabilities: vec!["docs".into()],
            permissions: Vec::new(),
            files: vec![PackFileEntry {
                path: "docs/README.md".into(),
                sha256: hex::encode(Sha256::digest(body)),
                size: body.len() as u64,
                content_type: "text/markdown".into(),
            }],
            integrations: Vec::new(),
            skills: Vec::<PackPathReference>::new(),
            tools: Vec::new(),
            mcp_servers: Vec::new(),
            binaries: Vec::new(),
            docker: Vec::new(),
            docs: vec!["docs/README.md".into()],
            tests: Vec::new(),
            dependencies: Vec::new(),
            signatures: Vec::new(),
        };
        fs::write(
            root.join(PACK_MANIFEST),
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        manifest
    }

    fn trust_key(
        repository: &dyn ExtensionRepository,
        publisher: &str,
        signing_key: &SigningKey,
    ) -> String {
        let public = signing_key.verifying_key().to_bytes();
        let key_id = digest_hex(&public);
        repository
            .add_publisher_trust(
                PublisherTrust {
                    publisher: publisher.into(),
                    key_id: key_id.clone(),
                    public_key: BASE64.encode(public),
                    added_at: "2026-07-11T00:00:00Z".into(),
                },
                actor(),
            )
            .expect("add trust");
        key_id
    }

    fn write_signed_pack(
        root: &Path,
        name: &str,
        version: &str,
        dependencies: Vec<String>,
        signing_key: &SigningKey,
        key_id: &str,
    ) {
        let mut manifest = write_pack(root);
        manifest.name = name.into();
        manifest.version = version.into();
        manifest.dependencies = dependencies;
        let unsigned = canonical_pack_signing_bytes(&manifest).expect("canonical pack");
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: key_id.into(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        fs::write(
            root.join(PACK_MANIFEST),
            serde_json::to_vec_pretty(&manifest).expect("signed pack manifest"),
        )
        .expect("write signed pack");
    }

    fn write_skill(root: &Path, name: &str, version: &str) {
        fs::create_dir_all(root).expect("skill root");
        let manifest = SkillManifest {
            name: name.into(),
            version: version.into(),
            description: "Collection skill.".into(),
            triggers: vec![name.into()],
            required_tools: Vec::new(),
            permissions: Vec::new(),
            offline_compatible: true,
        };
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("skill manifest"),
        )
        .expect("write skill manifest");
        fs::write(root.join("SKILL.md"), "Use this data-only skill safely.\n")
            .expect("write skill instructions");
    }

    fn write_oci_layout(layout: &Path, pack: &Path, gzip: bool) {
        let mut tar_bytes = Vec::new();
        {
            let mut archive = tar::Builder::new(&mut tar_bytes);
            archive
                .append_path_with_name(pack.join(PACK_MANIFEST), format!("demo/{PACK_MANIFEST}"))
                .expect("append manifest");
            archive
                .append_path_with_name(pack.join("docs/README.md"), "demo/docs/README.md")
                .expect("append body");
            archive.finish().expect("finish tar");
        }
        let (layer, media_type) = if gzip {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&tar_bytes).expect("compress layer");
            (
                encoder.finish().expect("finish gzip"),
                "application/vnd.colossus.pack.v1.tar+gzip",
            )
        } else {
            (tar_bytes, "application/vnd.colossus.pack.v1.tar")
        };
        let blobs = layout.join("blobs/sha256");
        fs::create_dir_all(&blobs).expect("blobs");
        let layer_digest = hex::encode(Sha256::digest(&layer));
        fs::write(blobs.join(&layer_digest), &layer).expect("layer blob");
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "layers": [{
                "mediaType": media_type,
                "digest": format!("sha256:{layer_digest}"),
                "size": layer.len()
            }]
        }))
        .expect("OCI manifest");
        let manifest_digest = hex::encode(Sha256::digest(&manifest));
        fs::write(blobs.join(&manifest_digest), &manifest).expect("manifest blob");
        fs::write(
            layout.join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )
        .expect("layout marker");
        fs::write(
            layout.join("index.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "manifests": [{
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": format!("sha256:{manifest_digest}"),
                    "size": manifest.len()
                }]
            }))
            .expect("index"),
        )
        .expect("index file");
    }

    #[test]
    fn unsigned_pack_verifies_but_is_not_trusted() {
        let source = TempDir::new().expect("source");
        write_pack(source.path());
        let (_, repository) = repository();
        let service = PackService::new(repository, source.path().join("installed"));
        let evidence = service.verify(source.path()).expect("verify");
        assert_eq!(evidence.file_count, 1);
        assert!(!evidence.trusted);
        assert_eq!(evidence.manifest.name, "demo-pack");
    }

    #[test]
    fn local_oci_tar_and_gzip_layouts_materialize_into_the_same_verified_pack() {
        let root = TempDir::new().expect("root");
        let pack = root.path().join("pack");
        fs::create_dir(&pack).expect("pack");
        write_pack(&pack);
        let (_, repository) = repository();
        let service = PackService::new(repository, root.path().join("installed"));
        for gzip in [false, true] {
            let layout = root
                .path()
                .join(if gzip { "gzip-layout" } else { "tar-layout" });
            fs::create_dir(&layout).expect("layout");
            write_oci_layout(&layout, &pack, gzip);
            let evidence = service.verify(&layout).expect("verify OCI layout");
            assert_eq!(evidence.manifest.name, "demo-pack");
            assert_eq!(evidence.file_count, 1);
        }
    }

    #[test]
    fn oci_layer_link_entries_fail_before_pack_materialization() {
        let root = TempDir::new().expect("root");
        let layout = root.path().join("layout");
        let blobs = layout.join("blobs/sha256");
        fs::create_dir_all(&blobs).expect("blobs");
        let mut layer = Vec::new();
        {
            let mut archive = tar::Builder::new(&mut layer);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_path("demo/link").expect("link path");
            header.set_link_name("../../outside").expect("link target");
            header.set_cksum();
            archive
                .append(&header, std::io::empty())
                .expect("append link");
            archive.finish().expect("finish tar");
        }
        let layer_digest = hex::encode(Sha256::digest(&layer));
        fs::write(blobs.join(&layer_digest), &layer).expect("layer");
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "layers": [{
                "mediaType": "application/vnd.colossus.pack.v1.tar",
                "digest": format!("sha256:{layer_digest}"),
                "size": layer.len()
            }]
        }))
        .expect("manifest");
        let manifest_digest = hex::encode(Sha256::digest(&manifest));
        fs::write(blobs.join(&manifest_digest), &manifest).expect("manifest blob");
        fs::write(
            layout.join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )
        .expect("layout marker");
        fs::write(
            layout.join("index.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "manifests": [{
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": format!("sha256:{manifest_digest}"),
                    "size": manifest.len()
                }]
            }))
            .expect("index"),
        )
        .expect("index file");
        let (_, repository) = repository();
        let service = PackService::new(repository, root.path().join("installed"));
        assert!(matches!(
            service.verify(&layout),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn signed_pack_requires_the_exact_publisher_key_and_rejects_tampering() {
        let source = TempDir::new().expect("source");
        let mut manifest = write_pack(source.path());
        let (_, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let key_id = trust_key(repository.as_ref(), "example", &signing_key);
        let unsigned = canonical_pack_signing_bytes(&manifest).expect("canonical manifest");
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        fs::write(
            source.path().join(PACK_MANIFEST),
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write signed manifest");
        let service = PackService::new(repository, source.path().join("installed"));
        let evidence = service.verify(source.path()).expect("trusted verify");
        assert!(evidence.trusted);
        assert_eq!(evidence.trust_key_id.as_deref(), Some(key_id.as_str()));

        fs::write(
            source.path().join("docs/README.md"),
            b"tampered pack documentation\n",
        )
        .expect("tamper body");
        assert!(matches!(
            service.verify(source.path()),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn traversal_and_undeclared_payloads_fail_closed() {
        let parent = TempDir::new().expect("parent");
        let source = parent.path().join("pack");
        fs::create_dir(&source).expect("source");
        fs::write(parent.path().join("outside"), b"outside").expect("outside");
        let mut manifest = write_pack(&source);
        manifest.files[0] = PackFileEntry {
            path: "../outside".into(),
            sha256: hex::encode(Sha256::digest(b"outside")),
            size: 7,
            content_type: "application/octet-stream".into(),
        };
        manifest.docs = vec!["../outside".into()];
        fs::write(
            source.join(PACK_MANIFEST),
            serde_json::to_vec(&manifest).expect("serialize traversal"),
        )
        .expect("write traversal");
        let (_, repository) = repository();
        let service = PackService::new(repository, parent.path().join("installed"));
        assert!(matches!(
            service.verify(&source),
            Err(PackError::Invalid(_))
        ));

        write_pack(&source);
        fs::write(source.join("undeclared.bin"), b"hidden executable").expect("undeclared");
        assert!(matches!(
            service.verify(&source),
            Err(PackError::Invalid(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_payload_is_rejected_even_when_it_points_inside_the_pack() {
        use std::os::unix::fs::symlink;

        let source = TempDir::new().expect("source");
        let mut manifest = write_pack(source.path());
        symlink("docs/README.md", source.path().join("alias.md")).expect("symlink");
        manifest.files.push(PackFileEntry {
            path: "alias.md".into(),
            sha256: manifest.files[0].sha256.clone(),
            size: manifest.files[0].size,
            content_type: "text/markdown".into(),
        });
        manifest.docs.push("alias.md".into());
        fs::write(
            source.path().join(PACK_MANIFEST),
            serde_json::to_vec(&manifest).expect("serialize symlink manifest"),
        )
        .expect("write manifest");
        let (_, repository) = repository();
        let service = PackService::new(repository, source.path().join("installed"));
        assert!(matches!(
            service.verify(source.path()),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn install_disable_enable_and_uninstall_are_event_sourced() {
        let root = TempDir::new().expect("root");
        let source = root.path().join("source");
        fs::create_dir(&source).expect("source");
        write_pack(&source);
        let (journal, repository) = repository();
        let service = PackService::new(repository.clone(), root.path().join("installed"));
        let installed = service
            .install(&source, true, actor())
            .expect("install unsigned with explicit override");
        assert_eq!(installed.status, PackStatus::Enabled);
        assert!(Path::new(&installed.installed_path).is_dir());
        assert_eq!(
            service
                .disable("demo-pack", actor())
                .expect("disable")
                .status,
            PackStatus::Disabled
        );
        assert_eq!(
            service.enable("demo-pack", actor()).expect("enable").status,
            PackStatus::Enabled
        );
        let removed = service.uninstall("demo-pack", actor()).expect("uninstall");
        assert_eq!(removed.status, PackStatus::Uninstalled);
        assert!(!Path::new(&removed.installed_path).exists());
        let event_types = journal
            .read_stream("pack:demo-pack")
            .expect("stream")
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert_eq!(
            event_types,
            vec![
                "pack.installed.v1",
                "pack.disabled.v1",
                "pack.enabled.v1",
                "pack.uninstalled.v1"
            ]
        );
    }

    #[test]
    fn signed_collection_build_is_reproducible_and_installs_dependency_order_without_clobbering() {
        let root = TempDir::new().expect("root");
        let source = root.path().join("source");
        let base = source.join("packs/base");
        let app = source.join("packs/app");
        let skill = source.join("skills/reviewer");
        fs::create_dir_all(&base).expect("base pack");
        fs::create_dir_all(&app).expect("app pack");
        write_skill(&skill, "reviewer", "1.0.0");
        let (journal, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[31_u8; 32]);
        let key_id = trust_key(repository.as_ref(), "example", &signing_key);
        write_signed_pack(&base, "base", "1.0.0", Vec::new(), &signing_key, &key_id);
        write_signed_pack(
            &app,
            "app",
            "2.0.0",
            vec!["base@1.0.0".into()],
            &signing_key,
            &key_id,
        );
        let pack_root = root.path().join("installed-packs");
        let skill_root = root.path().join("installed-skills");
        let service = PackService::new(repository, pack_root.clone())
            .with_skill_install_root(skill_root.clone());
        let first = root.path().join("first");
        let second = root.path().join("second");
        let built = service
            .build_collection(
                &source,
                &first,
                "starter-kit",
                "1.0.0",
                "example",
                "2026-07-16T12:00:00Z",
                signing_key.to_bytes(),
            )
            .expect("build collection");
        service
            .build_collection(
                &source,
                &second,
                "starter-kit",
                "1.0.0",
                "example",
                "2026-07-16T12:00:00Z",
                signing_key.to_bytes(),
            )
            .expect("rebuild collection");
        assert_eq!(built.verification.manifest.artifacts.len(), 3);
        assert_eq!(
            built
                .verification
                .packs
                .iter()
                .map(|pack| pack.manifest.name.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "app"]
        );
        assert_eq!(
            fs::read(first.join(COLLECTION_MANIFEST)).expect("first manifest"),
            fs::read(second.join(COLLECTION_MANIFEST)).expect("second manifest")
        );
        assert_eq!(
            canonical_collection_signing_bytes(&built.verification.manifest)
                .expect("collection bytes"),
            canonical_collection_signing_bytes(
                &service
                    .verify_collection(&second)
                    .expect("second verification")
                    .manifest
            )
            .expect("second collection bytes")
        );

        let installed = service
            .install_collection(&first, actor())
            .expect("install collection");
        assert_eq!(
            installed
                .packs
                .iter()
                .map(|pack| pack.manifest.name.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "app"]
        );
        assert_eq!(installed.skills[0].name, "reviewer");
        assert!(pack_root.join("base/1.0.0").is_dir());
        assert!(pack_root.join("app/2.0.0").is_dir());
        assert!(skill_root.join("reviewer").is_dir());
        assert!(matches!(
            service.install_collection(&first, actor()),
            Err(PackError::Invalid(_))
        ));
        assert_eq!(
            journal.read_stream("pack:base").expect("base events").len(),
            1
        );
        assert_eq!(
            journal.read_stream("pack:app").expect("app events").len(),
            1
        );
        let reopened =
            EventSourcedExtensionRepository::new(Arc::clone(&journal) as Arc<dyn EventJournal>);
        assert_eq!(
            reopened
                .list_packs(10)
                .expect("reconstruct collection pack lifecycles")
                .iter()
                .map(|pack| pack.manifest.name.as_str())
                .collect::<Vec<_>>(),
            vec!["app", "base"]
        );

        fs::write(
            second.join("skills/reviewer/SKILL.md"),
            "tampered instructions\n",
        )
        .expect("tamper collection");
        assert!(matches!(
            service.verify_collection(&second),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn collection_rejects_incomplete_pack_dependency_closure() {
        let root = TempDir::new().expect("root");
        let source = root.path().join("source");
        let app = source.join("packs/app");
        fs::create_dir_all(&app).expect("app pack");
        let (_, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[37_u8; 32]);
        let key_id = trust_key(repository.as_ref(), "example", &signing_key);
        write_signed_pack(
            &app,
            "app",
            "2.0.0",
            vec!["missing@1.0.0".into()],
            &signing_key,
            &key_id,
        );
        let service = PackService::new(repository, root.path().join("installed"));
        assert!(matches!(
            service.build_collection(
                &source,
                &root.path().join("collection"),
                "incomplete",
                "1.0.0",
                "example",
                "2026-07-16T12:00:00Z",
                signing_key.to_bytes(),
            ),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn collection_archive_is_deterministic_and_rejects_special_entries() {
        let root = TempDir::new().expect("root");
        let source = root.path().join("source");
        let skill = source.join("skills/reviewer");
        write_skill(&skill, "reviewer", "1.0.0");
        let (_, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[41_u8; 32]);
        trust_key(repository.as_ref(), "example", &signing_key);
        let service = PackService::new(repository, root.path().join("installed"));
        let collection = root.path().join("collection");
        let built = service
            .build_collection(
                &source,
                &collection,
                "archive-test",
                "1.0.0",
                "example",
                "2026-07-16T12:00:00Z",
                signing_key.to_bytes(),
            )
            .expect("build collection");
        let first = root.path().join("first.tar");
        let second = root.path().join("second.tar");
        write_collection_archive(
            &collection,
            &built.verification,
            &mut fs::File::create(&first).expect("first archive"),
        )
        .expect("write first archive");
        write_collection_archive(
            &collection,
            &built.verification,
            &mut fs::File::create(&second).expect("second archive"),
        )
        .expect("write second archive");
        assert_eq!(
            fs::read(&first).expect("first"),
            fs::read(&second).expect("second")
        );
        let extracted = root.path().join("extracted");
        fs::create_dir(&extracted).expect("extracted root");
        extract_collection_archive(&first, &extracted).expect("extract archive");
        assert_eq!(
            service
                .verify_collection(&extracted)
                .expect("verify extracted")
                .manifest_sha256,
            built.verification.manifest_sha256
        );

        let hostile = root.path().join("hostile.tar");
        let output = fs::File::create(&hostile).expect("hostile archive");
        let mut archive = tar::Builder::new(output);
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("../../outside").expect("link name");
        header.set_cksum();
        archive
            .append_data(&mut header, COLLECTION_MANIFEST, std::io::empty())
            .expect("hostile entry");
        archive.finish().expect("finish hostile archive");
        let hostile_destination = root.path().join("hostile-extracted");
        fs::create_dir(&hostile_destination).expect("hostile destination");
        assert!(matches!(
            extract_collection_archive(&hostile, &hostile_destination),
            Err(PackError::Invalid(_))
        ));
        assert!(!root.path().join("outside").exists());
    }

    #[tokio::test]
    async fn authenticated_registry_push_and_pull_round_trip_through_effect_gateway() {
        const TOKEN_VARIABLE: &str = "PATH";
        assert!(!std::env::var(TOKEN_VARIABLE).expect("test PATH").is_empty());
        let root = TempDir::new().expect("root");
        let source = root.path().join("source");
        write_skill(&source.join("skills/reviewer"), "reviewer", "1.0.0");
        let (journal, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[43_u8; 32]);
        trust_key(repository.as_ref(), "example", &signing_key);
        let service = Arc::new(PackService::new(
            repository,
            root.path().join("installed-packs"),
        ));
        let collection = root.path().join("collection");
        service
            .build_collection(
                &source,
                &collection,
                "registry-test",
                "1.0.0",
                "example",
                "2026-07-16T12:00:00Z",
                signing_key.to_bytes(),
            )
            .expect("build collection");

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let origin = format!("http://{address}");
        let endpoint = format!("{origin}/collections/registry-test/1.0.0");
        let stored = Arc::new(Mutex::new(None::<Vec<u8>>));
        let server_stored = Arc::clone(&stored);
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut request = Vec::new();
                let header_end = loop {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.expect("read request");
                    assert_ne!(count, 0, "request ended before headers");
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8(request[..header_end].to_vec()).expect("headers");
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer ")
                );
                if headers.starts_with("PUT ") {
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .expect("content length");
                    while request.len() - header_end < content_length {
                        let mut chunk = [0_u8; 4096];
                        let count = stream.read(&mut chunk).await.expect("read body");
                        assert_ne!(count, 0, "request ended before body");
                        request.extend_from_slice(&chunk[..count]);
                    }
                    *server_stored.lock().expect("stored") =
                        Some(request[header_end..header_end + content_length].to_vec());
                    stream
                        .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await
                        .expect("write push response");
                } else {
                    let body = server_stored
                        .lock()
                        .expect("stored")
                        .clone()
                        .expect("pushed body");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.colossus.collection.v1.tar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write pull headers");
                    stream.write_all(&body).await.expect("write pull body");
                }
            }
        });

        let policy = BuiltInPolicy::offline_default()
            .with_action("registry.push", DecisionOutcome::Allow)
            .with_action("registry.pull", DecisionOutcome::Allow)
            .with_filesystem_root(root.path().display().to_string(), "write")
            .with_environment(TOKEN_VARIABLE)
            .with_network_destination(&origin);
        let gateway = EffectGateway::new(
            Arc::clone(&journal) as Arc<dyn EventJournal>,
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(["registry.push".into(), "registry.pull".into()]),
            [47_u8; 32],
        );
        let executor = PackExecutor::new(Arc::clone(&service));
        let credential_reference = format!("env:{TOKEN_VARIABLE}");
        let push = PackOperation::RegistryPush {
            path: collection.display().to_string(),
            url: endpoint.clone(),
            credential_reference: Some(credential_reference.clone()),
        };
        let mut request = effect_request(
            actor(),
            push.action(),
            push.resource(),
            serde_json::to_value(&push).expect("push operation"),
        );
        request.capabilities = vec![push.action().into()];
        request.credential_references = vec![CredentialReference {
            reference: credential_reference.clone(),
            value_hash: None,
        }];
        gateway
            .execute(request, &executor)
            .await
            .expect("push collection");

        let destination = root.path().join("pulled");
        let pull = PackOperation::RegistryPull {
            url: endpoint,
            destination: destination.display().to_string(),
            credential_reference: Some(credential_reference.clone()),
        };
        let mut request = effect_request(
            actor(),
            pull.action(),
            pull.resource(),
            serde_json::to_value(&pull).expect("pull operation"),
        );
        request.capabilities = vec![pull.action().into()];
        request.credential_references = vec![CredentialReference {
            reference: credential_reference,
            value_hash: None,
        }];
        gateway
            .execute(request, &executor)
            .await
            .expect("pull collection");
        server.await.expect("server");
        assert_eq!(
            service
                .verify_collection(&destination)
                .expect("verify pulled collection")
                .manifest_sha256,
            service
                .verify_collection(&collection)
                .expect("verify source collection")
                .manifest_sha256
        );
        let secret = std::env::var(TOKEN_VARIABLE).expect("test credential");
        let audit = serde_json::to_string(&journal.read_global(1, 200).expect("audit events"))
            .expect("audit JSON");
        assert!(!audit.contains(&secret));

        let mismatched_destination = root.path().join("mismatched");
        let mismatched = PackOperation::RegistryPull {
            url: format!("{origin}/never-contacted"),
            destination: mismatched_destination.display().to_string(),
            credential_reference: Some(format!("env:{TOKEN_VARIABLE}")),
        };
        let mut request = effect_request(
            actor(),
            mismatched.action(),
            mismatched.resource(),
            serde_json::to_value(&mismatched).expect("mismatched operation"),
        );
        request.capabilities = vec![mismatched.action().into()];
        request.credential_references = vec![CredentialReference {
            reference: "env:HOME".into(),
            value_hash: None,
        }];
        assert!(gateway.execute(request, &executor).await.is_err());
        assert!(!mismatched_destination.exists());
    }

    #[test]
    fn offline_bundle_requires_a_valid_trusted_signature() {
        let source = TempDir::new().expect("bundle");
        fs::write(source.path().join("artifact.bin"), b"release bytes").expect("artifact");
        let (_, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let key_id = trust_key(repository.as_ref(), "colossus", &signing_key);
        let mut manifest = BundleManifest {
            format_version: 1,
            name: "colossus-offline".into(),
            version: "0.6.0".into(),
            publisher: "colossus".into(),
            created_at: "2026-07-11T00:00:00Z".into(),
            source_revision: Some("deadbeef".into()),
            files: vec![BundleFileEntry {
                path: "artifact.bin".into(),
                sha256: hex::encode(Sha256::digest(b"release bytes")),
                size: Some(13),
            }],
            signatures: Vec::new(),
        };
        let unsigned = canonical_bundle_signing_bytes(&manifest).expect("canonical bundle");
        manifest.signatures.push(PackSignature {
            algorithm: "ed25519".into(),
            key_id: key_id.clone(),
            signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
        });
        fs::write(
            source.path().join(BUNDLE_MANIFEST),
            serde_json::to_vec_pretty(&manifest).expect("serialize bundle"),
        )
        .expect("manifest");
        let service = PackService::new(repository, source.path().join("installed"));
        let evidence = service.verify_bundle(source.path()).expect("verify bundle");
        assert_eq!(evidence.trust_key_id, key_id);
        assert_eq!(evidence.total_bytes, 13);

        manifest.signatures[0].signature = BASE64.encode([0_u8; 64]);
        fs::write(
            source.path().join(BUNDLE_MANIFEST),
            serde_json::to_vec(&manifest).expect("serialize bad signature"),
        )
        .expect("bad manifest");
        assert!(matches!(
            service.verify_bundle(source.path()),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn signed_bundle_build_is_reproducible_and_installs_only_into_a_clean_prefix() {
        let root = TempDir::new().expect("root");
        let root = fs::canonicalize(root.path()).expect("canonical root");
        let source = root.join("staged");
        let target = current_release_target().expect("release target");
        let artifact = bundle_artifact_path(target);
        let artifact_path = source.join(&artifact);
        fs::create_dir_all(artifact_path.parent().expect("artifact parent"))
            .expect("artifact directory");
        fs::write(&artifact_path, b"standalone-native-binary").expect("artifact");
        fs::write(source.join("LICENSE"), b"Apache-2.0\n").expect("license");

        let (_, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[33_u8; 32]);
        let key_id = trust_key(repository.as_ref(), "colossus", &signing_key);
        let service = PackService::new(repository, root.join("packs"));
        let first = root.join("bundle-one");
        let second = root.join("bundle-two");
        for destination in [&first, &second] {
            let materialization = service
                .build_bundle(
                    &source,
                    destination,
                    "colossus-offline",
                    "0.6.0",
                    "colossus",
                    "2026-07-11T00:00:00Z",
                    Some("0123456789abcdef".into()),
                    signing_key.to_bytes(),
                )
                .expect("build bundle");
            assert_eq!(materialization.signing_key_id, key_id);
            assert_eq!(materialization.targets, [target.to_owned()]);
            assert_eq!(materialization.verification.file_count, 2);
        }
        assert_eq!(
            fs::read(first.join(BUNDLE_MANIFEST)).expect("first manifest"),
            fs::read(second.join(BUNDLE_MANIFEST)).expect("second manifest")
        );

        let prefix = root.join("prefix");
        let installation = service
            .install_bundle(&first, &prefix)
            .expect("install bundle");
        assert_eq!(installation.target, target);
        assert_eq!(installation.artifact, artifact);
        assert_eq!(
            fs::read(&installation.installed_path).expect("installed bytes"),
            b"standalone-native-binary"
        );
        assert!(matches!(
            service.install_bundle(&first, &prefix),
            Err(PackError::Invalid(message))
                if message.contains("refuses to replace existing path")
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let actual_prefix = root.join("actual-prefix");
            fs::create_dir(&actual_prefix).expect("actual prefix");
            let linked_prefix = root.join("linked-prefix");
            symlink(&actual_prefix, &linked_prefix).expect("linked prefix");
            let error = service
                .install_bundle(&first, &linked_prefix)
                .expect_err("linked prefix must fail");
            assert!(
                error.to_string().contains("must be a real directory"),
                "{error}"
            );
        }

        let other_target = RELEASE_TARGETS
            .iter()
            .find(|candidate| **candidate != target)
            .expect("other release target");
        let other_source = root.join("other-staged");
        let other_artifact = other_source.join(bundle_artifact_path(other_target));
        fs::create_dir_all(other_artifact.parent().expect("other artifact parent"))
            .expect("other artifact directory");
        fs::write(&other_artifact, b"other-native-binary").expect("other artifact");
        let other_bundle = root.join("other-bundle");
        service
            .build_bundle(
                &other_source,
                &other_bundle,
                "colossus-offline-other",
                "0.6.0",
                "colossus",
                "2026-07-11T00:00:00Z",
                None,
                signing_key.to_bytes(),
            )
            .expect("build other-target bundle");
        let error = service
            .install_bundle(&other_bundle, &root.join("other-prefix"))
            .expect_err("wrong-target bundle must not install");
        assert!(
            error
                .to_string()
                .contains("does not contain a native executable"),
            "{error}"
        );

        fs::OpenOptions::new()
            .write(true)
            .open(first.join(&installation.artifact))
            .expect("open artifact for tampering")
            .write_all(b"tampered")
            .expect("tamper artifact");
        let error = service
            .verify_bundle(&first)
            .expect_err("tampered bundle must fail");
        assert!(error.to_string().contains("file hash mismatch"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn signed_bundle_build_rejects_linked_staging_payloads() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().expect("root");
        let root = fs::canonicalize(root.path()).expect("canonical root");
        let source = root.join("staged");
        let artifact = source.join(bundle_artifact_path(
            current_release_target().expect("release target"),
        ));
        fs::create_dir_all(artifact.parent().expect("artifact parent"))
            .expect("artifact directory");
        let outside = root.join("outside-binary");
        fs::write(&outside, b"outside").expect("outside binary");
        symlink(&outside, &artifact).expect("linked artifact");
        let (_, repository) = repository();
        let signing_key = SigningKey::from_bytes(&[34_u8; 32]);
        trust_key(repository.as_ref(), "colossus", &signing_key);
        let service = PackService::new(repository, root.join("packs"));
        let error = service
            .build_bundle(
                &source,
                &root.join("bundle"),
                "colossus-linked",
                "0.6.0",
                "colossus",
                "2026-07-11T00:00:00Z",
                None,
                signing_key.to_bytes(),
            )
            .expect_err("linked payload must fail");
        assert!(
            error.to_string().contains("symlink is forbidden"),
            "{error}"
        );
    }

    #[test]
    fn offline_bundle_rejects_the_legacy_parent_traversal_shape() {
        let parent = TempDir::new().expect("parent");
        let source = parent.path().join("bundle");
        fs::create_dir(&source).expect("bundle");
        fs::write(parent.path().join("outside.bin"), b"outside").expect("outside");
        let manifest = BundleManifest {
            format_version: 1,
            name: "colossus-offline".into(),
            version: "0.6.0".into(),
            publisher: "colossus".into(),
            created_at: "2026-07-11T00:00:00Z".into(),
            source_revision: None,
            files: vec![BundleFileEntry {
                path: "../outside.bin".into(),
                sha256: hex::encode(Sha256::digest(b"outside")),
                size: Some(7),
            }],
            signatures: Vec::new(),
        };
        fs::write(
            source.join(BUNDLE_MANIFEST),
            serde_json::to_vec(&manifest).expect("serialize bundle"),
        )
        .expect("manifest");
        let (_, repository) = repository();
        let service = PackService::new(repository, parent.path().join("installed"));
        assert!(matches!(
            service.verify_bundle(&source),
            Err(PackError::Invalid(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn offline_bundle_rejects_symlink_payloads() {
        use std::os::unix::fs::symlink;

        let parent = TempDir::new().expect("parent");
        let source = parent.path().join("bundle");
        fs::create_dir(&source).expect("bundle");
        fs::write(parent.path().join("outside.bin"), b"outside").expect("outside");
        symlink("../outside.bin", source.join("artifact.bin")).expect("symlink");
        let manifest = BundleManifest {
            format_version: 1,
            name: "colossus-offline".into(),
            version: "0.6.0".into(),
            publisher: "colossus".into(),
            created_at: "2026-07-11T00:00:00Z".into(),
            source_revision: None,
            files: vec![BundleFileEntry {
                path: "artifact.bin".into(),
                sha256: hex::encode(Sha256::digest(b"outside")),
                size: Some(7),
            }],
            signatures: Vec::new(),
        };
        fs::write(
            source.join(BUNDLE_MANIFEST),
            serde_json::to_vec(&manifest).expect("serialize bundle"),
        )
        .expect("manifest");
        let (_, repository) = repository();
        let service = PackService::new(repository, parent.path().join("installed"));
        assert!(matches!(
            service.verify_bundle(&source),
            Err(PackError::Invalid(_))
        ));
    }
}
