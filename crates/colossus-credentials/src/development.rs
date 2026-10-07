//! Explicit development-only custody; selection never discovers or migrates keys.

use crate::PlatformKeyStore;
use crate::{EnvironmentKeyStore, crypto};
use colossus_contracts::CredentialError;
use colossus_contracts::VaultRecord;
use colossus_home::{ConfinedFile, ConfinedRoot};
use colossus_ports::{CredentialKey, CredentialVault};
use fs4::fs_std::FileExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::{
    fmt,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

/// Nonsecret explicit authority-path selector. Never carries a key or credential.
pub const DEVELOPMENT_AUTHORITY_VARIABLE: &str = "COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY";
const DIRECTORY: &str = "development-credentials";
const KEY_FILE: &str = ".authority-key.env";
const MANIFEST: &str = "authority.json";
const LOCK: &str = "authority.lock";

/// Closed native storage scopes prevent a renderer from choosing arbitrary roots.
#[derive(Clone, Copy, Debug)]
pub enum DevelopmentStoreScope {
    /// Native Desktop provider/client/enrollment vault master envelopes.
    DesktopVault,
    /// Standalone Control Plane enrollment vault master envelopes.
    ControlPlaneVault,
    /// Fixed original public API key and application bearer selectors.
    PublicApi,
    /// Canonical journal keys, checkpoint seeds and protected anchor records.
    Journal,
    /// Runtime-owned OAuth vault master envelopes, retaining their original scope.
    RuntimeOAuthVault,
}
impl DevelopmentStoreScope {
    fn directory(self) -> &'static str {
        match self {
            Self::DesktopVault => "desktop-vault",
            Self::ControlPlaneVault => "control-plane-vault",
            Self::PublicApi => "public-api",
            Self::Journal => "journal",
            Self::RuntimeOAuthVault => "runtime-oauth-vault",
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u16,
    home_fingerprint: String,
    active: bool,
}
/// Read-only public development custody state; never contains key material.
#[derive(Debug, Clone, Serialize)]
pub struct DevelopmentAuthorityMetadata {
    /// Exact existing private authority directory beneath the selected home.
    pub authority_path: PathBuf,
    /// Whether explicit offline preparation/rewrap completed activation.
    pub active: bool,
}

/// Private zeroizing authority opened only by trusted native composition.
pub struct DevelopmentAuthority {
    root: ConfinedRoot,
    key_file: ConfinedFile,
    key: Zeroizing<[u8; 32]>,
}
impl fmt::Debug for DevelopmentAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevelopmentAuthority")
            .finish_non_exhaustive()
    }
}
impl DevelopmentAuthority {
    /// Inspect only existing nonsecret metadata without reading a wrapping key.
    pub fn metadata(
        home: &ConfinedRoot,
        path: &Path,
        workspaces: &[PathBuf],
    ) -> Result<DevelopmentAuthorityMetadata, CredentialError> {
        let (root, manifest) = validated_manifest(home, path, workspaces)?;
        Ok(DevelopmentAuthorityMetadata {
            authority_path: root.path().to_owned(),
            active: manifest.active,
        })
    }
    /// Exact home-owned path; no arbitrary environment-selected root is accepted.
    pub fn path_for_home(home: &ConfinedRoot) -> PathBuf {
        home.path().join(DIRECTORY)
    }

    /// Initialize only an empty authority. A missing/partial/invalid existing key
    /// is never regenerated, and no platform key or credential is read or moved.
    pub fn initialize(
        home: &ConfinedRoot,
        workspaces: &[PathBuf],
    ) -> Result<Self, CredentialError> {
        validate_workspaces(&Self::path_for_home(home), workspaces)?;
        let path = home
            .prepare_directory(Path::new(DIRECTORY))
            .map_err(invalid)?;
        let root = ConfinedRoot::bind(path).map_err(invalid)?;
        let lock = root.open_file(Path::new(LOCK)).map_err(invalid)?;
        if !lock
            .file()
            .try_lock_exclusive()
            .map_err(|_| CredentialError::Io)?
        {
            return Err(CredentialError::Busy);
        }
        let _lock = crate::database::VaultLease(lock);
        let has_key = root
            .path()
            .join(KEY_FILE)
            .try_exists()
            .map_err(|_| CredentialError::Io)?;
        let has_manifest = root
            .path()
            .join(MANIFEST)
            .try_exists()
            .map_err(|_| CredentialError::Io)?;
        if has_key || has_manifest {
            return Self::open(home, root.path(), workspaces);
        }
        if std::fs::read_dir(root.path())
            .map_err(|_| CredentialError::Io)?
            .any(|entry| entry.map(|item| item.file_name() != LOCK).unwrap_or(true))
        {
            return Err(CredentialError::Corrupt);
        }
        let key = crypto::new_key()?;
        let mut encoded = Zeroizing::new([0_u8; 65]);
        hex::encode_to_slice(key.as_ref(), &mut encoded[..64])
            .map_err(|_| CredentialError::Corrupt)?;
        encoded[64] = b'\n';
        write_new(&root, KEY_FILE, encoded.as_ref())?;
        let metadata = serde_json::to_vec(&Manifest {
            schema_version: 1,
            home_fingerprint: home_fingerprint(home)?,
            active: false,
        })
        .map_err(|_| CredentialError::Io)?;
        for scope in [
            DevelopmentStoreScope::DesktopVault,
            DevelopmentStoreScope::ControlPlaneVault,
            DevelopmentStoreScope::PublicApi,
            DevelopmentStoreScope::Journal,
            DevelopmentStoreScope::RuntimeOAuthVault,
        ] {
            root.prepare_directory(Path::new(scope.directory()))
                .map_err(invalid)?;
        }
        write_new(&root, MANIFEST, &metadata)?;
        root.sync_directory().map_err(invalid)?;
        Self::open(home, root.path(), workspaces)
    }

    /// Open an exact initialized home-bound authority. This never falls back to a
    /// platform store or creates missing files, even when a protected vault exists.
    pub fn open(
        home: &ConfinedRoot,
        path: &Path,
        workspaces: &[PathBuf],
    ) -> Result<Self, CredentialError> {
        let (root, _) = validated_manifest(home, path, workspaces)?;
        let manifest_file = root
            .open_existing_file(Path::new(MANIFEST))
            .map_err(invalid)?;
        let mut metadata = Vec::new();
        manifest_file
            .file()
            .take(1025)
            .read_to_end(&mut metadata)
            .map_err(|_| CredentialError::Io)?;
        if metadata.len() > 1024 {
            return Err(CredentialError::Corrupt);
        }
        let manifest: Manifest =
            serde_json::from_slice(&metadata).map_err(|_| CredentialError::Corrupt)?;
        if manifest.schema_version != 1 || manifest.home_fingerprint != home_fingerprint(home)? {
            return Err(CredentialError::Corrupt);
        }
        manifest_file.revalidate(&root).map_err(invalid)?;
        let key_file = root
            .open_existing_file(Path::new(KEY_FILE))
            .map_err(invalid)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if key_file
                .file()
                .metadata()
                .map_err(invalid)?
                .permissions()
                .mode()
                & 0o777
                != 0o600
            {
                return Err(CredentialError::InvalidInput);
            }
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(66));
        key_file
            .file()
            .take(66)
            .read_to_end(&mut bytes)
            .map_err(|_| CredentialError::Io)?;
        let content = match bytes.as_slice() {
            bytes if bytes.len() == 64 => bytes,
            bytes if bytes.len() == 65 && bytes[64] == b'\n' => &bytes[..64],
            _ => return Err(CredentialError::Corrupt),
        };
        let mut key = Zeroizing::new([0_u8; 32]);
        hex::decode_to_slice(content, key.as_mut()).map_err(|_| CredentialError::Corrupt)?;
        key_file.revalidate(&root).map_err(invalid)?;
        Ok(Self {
            root,
            key_file,
            key,
        })
    }

    /// Resolve only the nonsecret explicit selector and marker, before any key or
    /// runtime acquisition. Release builds reject this development opt-in.
    pub fn selected_root(
        home: &ConfinedRoot,
        workspaces: &[PathBuf],
    ) -> Result<Option<ConfinedRoot>, CredentialError> {
        let path = std::env::var_os(DEVELOPMENT_AUTHORITY_VARIABLE);
        selected_root(
            home,
            path.as_deref().map(Path::new),
            workspaces,
            cfg!(debug_assertions),
        )
    }

    /// Open only an active explicitly selected development authority.
    pub fn selected(
        home: &ConfinedRoot,
        workspaces: &[PathBuf],
    ) -> Result<Option<Self>, CredentialError> {
        Self::selected_root(home, workspaces)?
            .map(|root| Self::open(home, root.path(), workspaces))
            .transpose()
    }

    /// Explicit offline completion boundary. Call only after fresh-home checks or
    /// every approved source selector was sealed and verified; preparation alone
    /// never enables runtime use of incomplete migrated custody.
    pub fn activate(&self, home: &ConfinedRoot) -> Result<(), CredentialError> {
        let lock = self.root.open_file(Path::new(LOCK)).map_err(invalid)?;
        if !lock
            .file()
            .try_lock_exclusive()
            .map_err(|_| CredentialError::Io)?
        {
            return Err(CredentialError::Busy);
        }
        let lock = crate::database::VaultLease(lock);
        lock.revalidate(&self.root).map_err(invalid)?;
        let (_, mut manifest) = validated_manifest(home, self.root.path(), &[])?;
        manifest.active = true;
        let bytes = serde_json::to_vec(&manifest).map_err(|_| CredentialError::Io)?;
        let file = self
            .root
            .open_file(Path::new("authority.next.json"))
            .map_err(invalid)?;
        file.file()
            .write_all(&bytes)
            .map_err(|_| CredentialError::Io)?;
        file.file()
            .set_len(bytes.len() as u64)
            .map_err(|_| CredentialError::Io)?;
        file.file().sync_all().map_err(|_| CredentialError::Io)?;
        file.revalidate(&self.root).map_err(invalid)?;
        lock.revalidate(&self.root).map_err(invalid)?;
        let current = self
            .root
            .open_existing_file(Path::new(MANIFEST))
            .map_err(invalid)?;
        current.revalidate(&self.root).map_err(invalid)?;
        self.key_file.revalidate(&self.root).map_err(invalid)?;
        std::fs::rename(file.path(), self.root.path().join(MANIFEST))
            .map_err(|_| CredentialError::Io)?;
        self.root.sync_directory().map_err(invalid)?;
        let metadata = Self::metadata(home, self.root.path(), &[])?;
        if !metadata.active {
            return Err(CredentialError::Corrupt);
        }
        Ok(())
    }

    /// Select only an existing fixed private store; no secret enters an environment.
    pub fn store(
        &self,
        scope: DevelopmentStoreScope,
    ) -> Result<EnvironmentKeyStore, CredentialError> {
        self.key_file.revalidate(&self.root).map_err(invalid)?;
        let path = self.root.path().join(scope.directory());
        self.root.revalidate_directory(&path).map_err(invalid)?;
        let root = ConfinedRoot::bind(path).map_err(invalid)?;
        Ok(EnvironmentKeyStore::with_wrapping_key(
            root,
            Zeroizing::new(*self.key),
        ))
    }

    /// Larger journal anchor/checkpoint records use the existing bounded encrypted
    /// vault; its small random master envelope uses this same authority.
    pub fn journal_vault(&self) -> Result<Arc<crate::PlatformCredentialVault>, CredentialError> {
        let keys = self.store(DevelopmentStoreScope::Journal)?;
        let root = ConfinedRoot::bind(
            self.root
                .path()
                .join(DevelopmentStoreScope::Journal.directory()),
        )
        .map_err(invalid)?;
        crate::PlatformCredentialVault::with_key_store(root, "development-journal", Arc::new(keys))
            .map(Arc::new)
    }

    /// Validate each active workspace before granting its tools access to a runtime.
    pub fn validate_workspaces(&self, workspaces: &[PathBuf]) -> Result<(), CredentialError> {
        validate_workspaces(self.root.path(), workspaces)
    }
}

fn validated_manifest(
    home: &ConfinedRoot,
    path: &Path,
    workspaces: &[PathBuf],
) -> Result<(ConfinedRoot, Manifest), CredentialError> {
    if path != DevelopmentAuthority::path_for_home(home) || !path.is_absolute() {
        return Err(CredentialError::InvalidInput);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(invalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CredentialError::InvalidInput);
    }
    let root = ConfinedRoot::bind(path).map_err(invalid)?;
    if root.path() != DevelopmentAuthority::path_for_home(home) {
        return Err(CredentialError::InvalidInput);
    }
    validate_workspaces(root.path(), workspaces)?;
    let file = root
        .open_existing_file(Path::new(MANIFEST))
        .map_err(invalid)?;
    let mut bytes = Vec::new();
    file.file()
        .take(1025)
        .read_to_end(&mut bytes)
        .map_err(|_| CredentialError::Io)?;
    if bytes.len() > 1024 {
        return Err(CredentialError::Corrupt);
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| CredentialError::Corrupt)?;
    if manifest.schema_version != 1 || manifest.home_fingerprint != home_fingerprint(home)? {
        return Err(CredentialError::Corrupt);
    }
    file.revalidate(&root).map_err(invalid)?;
    Ok((root, manifest))
}
fn selected_root(
    home: &ConfinedRoot,
    path: Option<&Path>,
    workspaces: &[PathBuf],
    development_build: bool,
) -> Result<Option<ConfinedRoot>, CredentialError> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !development_build {
        return Err(CredentialError::InvalidInput);
    }
    let (root, manifest) = validated_manifest(home, path, workspaces)?;
    if !manifest.active {
        return Err(CredentialError::Unavailable);
    }
    Ok(Some(root))
}

/// Seal one already-validated existing small envelope, retaining the old source.
/// Existing different material is a terminal conflict, never overwritten.
pub fn seal_existing(
    target: &dyn PlatformKeyStore,
    account: &str,
    original: &[u8],
) -> Result<bool, CredentialError> {
    if original.is_empty() || original.len() > 256 {
        return Err(CredentialError::Oversized);
    }
    if let Some(existing) = target.read(account)? {
        if existing.len() != original.len() || !bool::from(existing.as_slice().ct_eq(original)) {
            return Err(CredentialError::Corrupt);
        }
        return Ok(false);
    }
    target.write(account, original)?;
    let readback = target.read(account)?.ok_or(CredentialError::MissingKey)?;
    if readback.len() != original.len() || !bool::from(readback.as_slice().ct_eq(original)) {
        return Err(CredentialError::Corrupt);
    }
    Ok(true)
}

/// Exact native public-API selector mapping shared by CLI, connector and journal
/// development custody. This never contains or derives an application bearer.
pub fn headless_credential_account(service: &str, account: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"colossus-headless-public-api-v1\0");
    hash.update((service.len() as u64).to_be_bytes());
    hash.update(service.as_bytes());
    hash.update(account.as_bytes());
    hex::encode(hash.finalize())
}

/// Shared opaque slot for original journal keys/checkpoint seeds/anchor records.
pub fn development_journal_key(
    service: &str,
    account: &str,
) -> Result<CredentialKey, CredentialError> {
    CredentialKey::new(
        "development-journal",
        &headless_credential_account(service, account),
    )
}

/// Preserve a larger existing journal record without plaintext export or overwrite.
pub fn seal_existing_record(
    target: &dyn CredentialVault,
    key: &CredentialKey,
    original: &[u8],
) -> Result<bool, CredentialError> {
    if let Some(existing) = target.read(key)? {
        if existing.expose().len() != original.len()
            || !bool::from(existing.expose().ct_eq(original))
        {
            return Err(CredentialError::Corrupt);
        }
        return Ok(false);
    }
    let record = VaultRecord::new(original.to_vec())?;
    target.write(key, &record)?;
    let readback = target.read(key)?.ok_or(CredentialError::MissingKey)?;
    if readback.expose().len() != original.len() || !bool::from(readback.expose().ct_eq(original)) {
        return Err(CredentialError::Corrupt);
    }
    Ok(true)
}

fn home_fingerprint(home: &ConfinedRoot) -> Result<String, CredentialError> {
    home.revalidate().map_err(invalid)?;
    let text = home.path().to_str().ok_or(CredentialError::InvalidInput)?;
    Ok(hex::encode(Sha256::digest(text.as_bytes())))
}
fn validate_workspaces(authority: &Path, workspaces: &[PathBuf]) -> Result<(), CredentialError> {
    if workspaces.is_empty() {
        return Ok(());
    }
    // Compare the same physical path spelling on both sides. Windows homes may
    // use ordinary drive paths while canonical workspaces use verbatim prefixes.
    // Initialization also checks overlap before the authority directory exists.
    let authority = match std::fs::canonicalize(authority) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = authority.parent().ok_or(CredentialError::InvalidInput)?;
            let leaf = authority.file_name().ok_or(CredentialError::InvalidInput)?;
            std::fs::canonicalize(parent).map_err(invalid)?.join(leaf)
        }
        Err(error) => return Err(invalid(error)),
    };
    for workspace in workspaces {
        let canonical = std::fs::canonicalize(workspace).map_err(invalid)?;
        if authority.starts_with(&canonical) || canonical.starts_with(&authority) {
            return Err(CredentialError::InvalidInput);
        }
    }
    Ok(())
}
fn write_new(root: &ConfinedRoot, name: &str, bytes: &[u8]) -> Result<(), CredentialError> {
    let file = root.open_file(Path::new(name)).map_err(invalid)?;
    if !file.was_created() {
        return Err(CredentialError::Corrupt);
    }
    file.file()
        .write_all(bytes)
        .map_err(|_| CredentialError::Io)?;
    file.file().sync_all().map_err(|_| CredentialError::Io)?;
    file.revalidate(root).map_err(invalid)?;
    root.sync_directory().map_err(invalid)
}
fn invalid(_: impl fmt::Debug) -> CredentialError {
    CredentialError::InvalidInput
}
