//! Lazy protected-vault lifecycle. Reads never initialize a missing vault or key.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_contracts::{CredentialError, VaultRecord};
use colossus_home::{ConfinedRoot, HomeError};
use colossus_ports::{CredentialKey, CredentialVault};
use fs4::fs_std::FileExt as _;
use redb::{Database, Durability, ReadableDatabase as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    io::ErrorKind,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};
use subtle::ConstantTimeEq as _;

use crate::{
    crypto,
    database::{DATABASE_FILE, Initialization, LEASE_FILE, Metadata, OpenedVault, RECORDS},
    platform::{PlatformKeyStore, SystemKeyStore},
};

const VERIFICATION: &[u8] = b"colossus-native-credential-vault-key-v1";

/// Nonsecret exact vault identity for an explicitly reviewed offline rewrap.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultKeyMetadata {
    /// Original versioned vault identity, retained during rewrap.
    pub vault_id: String,
    /// Original master-key identity, retained during rewrap.
    pub key_id: String,
    /// Original native owner-purpose binding, never a caller authorization claim.
    pub owner_scope_hash: String,
    /// Exact fixed-service platform envelope selector derived from the identity.
    pub account: String,
}
impl From<&Metadata> for VaultKeyMetadata {
    fn from(value: &Metadata) -> Self {
        Self {
            vault_id: value.vault_id.clone(),
            key_id: value.key_id.clone(),
            owner_scope_hash: value.owner_scope_hash.clone(),
            account: value.account(),
        }
    }
}
/// Ciphertext-only identity of one exact existing credential record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultRecordMetadata {
    /// Exact original vault identity; its master is never exported for record copy.
    pub vault: VaultKeyMetadata,
    /// SHA-256 of the selected ciphertext, binding reviewed source content.
    pub ciphertext_sha256: String,
}

/// Opaque source-read-only observation retained through an offline custody apply.
/// Holds writer exclusion and identity/hash proof, never a key or recovered database.
pub struct VaultSourceGuard {
    root: ConfinedRoot,
    observation: crate::metadata_view::SourceObservation,
}
impl fmt::Debug for VaultSourceGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultSourceGuard")
            .finish_non_exhaustive()
    }
}
impl VaultSourceGuard {
    /// Recheck the retained original source immediately before activating custody.
    /// A replaced, modified or unsafe source fails without reading any credential.
    pub fn revalidate(&self) -> Result<(), CredentialError> {
        self.observation.revalidate(&self.root)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InitializationCheckpoint {
    DatabaseOpened,
    PendingCommitted,
    PendingDirectorySynced,
    KeyStored,
    KeyReadBack,
    ReadyCommitted,
    BeforeRecordCommit,
    RecordCommitted,
}

/// A native, independently keyed credential vault confined to an explicit private root.
///
/// Construction is read-only. The first write creates the vault and platform key.
/// Once opened, the process retains an exclusive lease until this object is dropped.
/// Its verified master key is cached in zeroizing memory for that same lifetime.
/// Reopening rechecks platform key availability; there is no process-global key cache.
/// The adapter does not perform consumer authorization or expose renderer commands.
pub struct PlatformCredentialVault {
    root: ConfinedRoot,
    owner_scope_hash: String,
    keys: Arc<dyn PlatformKeyStore>,
    opened: Mutex<Option<OpenedVault>>,
    #[cfg(test)]
    failure: Mutex<Option<InitializationCheckpoint>>,
}

impl fmt::Debug for PlatformCredentialVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformCredentialVault")
            .finish_non_exhaustive()
    }
}

impl PlatformCredentialVault {
    /// Retain a read-only source lease/hash through a multi-source offline apply.
    /// Validates existing public metadata only. No source file or key is created,
    /// repaired or read from a platform store; dropping the guard releases writers.
    pub fn source_guard(&self) -> Result<VaultSourceGuard, CredentialError> {
        let view = crate::metadata_view::MetadataView::open_existing(&self.root)?
            .ok_or(CredentialError::MissingKey)?;
        view.metadata(&self.owner_scope_hash)?;
        view.revalidate(&self.root)?;
        Ok(VaultSourceGuard {
            root: self.root.clone(),
            observation: view.into_observation(),
        })
    }

    /// Read one existing ciphertext identity only, without initializing any source
    /// or reading an OS key. Missing records fail rather than selecting another.
    pub fn record_metadata(
        root: &ConfinedRoot,
        owner_scope: &str,
        key: &CredentialKey,
    ) -> Result<Option<VaultRecordMetadata>, CredentialError> {
        let Some(database) = crate::metadata_view::MetadataView::open_existing(root)? else {
            return Ok(None);
        };
        let metadata = database.metadata(&hex::encode(Sha256::digest(owner_scope.as_bytes())))?;
        let read = database.begin_read()?;
        let table = read
            .open_table(RECORDS)
            .map_err(|_| CredentialError::Corrupt)?;
        let id = record_id(key);
        let Some(value) = table.get(id.as_str()).map_err(|_| CredentialError::Io)? else {
            database.revalidate(root)?;
            return Ok(None);
        };
        if value.value().len() > crypto::MAX_CIPHERTEXT_BYTES {
            return Err(CredentialError::Oversized);
        }
        let ciphertext_sha256 = hex::encode(Sha256::digest(value.value()));
        database.revalidate(root)?;
        Ok(Some(VaultRecordMetadata {
            vault: VaultKeyMetadata::from(&metadata),
            ciphertext_sha256,
        }))
    }

    /// Copy only one approved record into an independently keyed target vault.
    /// No source master, sibling record, grant or enrollment is exported/rotated.
    pub fn copy_existing_record(
        &self,
        target: &dyn CredentialVault,
        key: &CredentialKey,
        expected: &VaultRecordMetadata,
    ) -> Result<bool, CredentialError> {
        if std::ptr::addr_eq(self as &dyn CredentialVault, target) {
            return Err(CredentialError::InvalidInput);
        }
        let source = crate::metadata_view::MetadataView::open_existing(&self.root)?
            .ok_or(CredentialError::MissingKey)?;
        let metadata = source.metadata(&self.owner_scope_hash)?;
        if VaultKeyMetadata::from(&metadata) != expected.vault {
            return Err(CredentialError::Corrupt);
        }
        let id = record_id(key);
        let ciphertext = {
            let read = source.begin_read()?;
            let table = read
                .open_table(RECORDS)
                .map_err(|_| CredentialError::Corrupt)?;
            let value = table
                .get(id.as_str())
                .map_err(|_| CredentialError::Io)?
                .ok_or(CredentialError::MissingKey)?;
            if value.value().len() > crypto::MAX_CIPHERTEXT_BYTES {
                return Err(CredentialError::Oversized);
            }
            if hex::encode(Sha256::digest(value.value())) != expected.ciphertext_sha256 {
                return Err(CredentialError::Corrupt);
            }
            value.value().to_vec()
        };
        source.revalidate(&self.root)?;
        let envelope = self
            .keys
            .read(&metadata.account())?
            .ok_or(CredentialError::MissingKey)?;
        let master = verified_source_key(&metadata, &envelope)?;
        let plaintext = crypto::decrypt(&master, &metadata.aad(&id), &ciphertext)?;
        source.revalidate(&self.root)?;
        let copied = crate::development::seal_existing_record(target, key, &plaintext)?;
        source.revalidate(&self.root)?;
        Ok(copied)
    }
    /// Inspect existing public metadata only. Missing sources stay absent; no
    /// lease file, vault, platform entry or key is created. An active writer fails.
    pub fn key_metadata(
        root: &ConfinedRoot,
        owner_scope: &str,
    ) -> Result<Option<VaultKeyMetadata>, CredentialError> {
        let Some(database) = crate::metadata_view::MetadataView::open_existing(root)? else {
            return Ok(None);
        };
        let metadata = database.metadata(&hex::encode(Sha256::digest(owner_scope.as_bytes())))?;
        database.revalidate(root)?;
        Ok(Some(VaultKeyMetadata::from(&metadata)))
    }

    /// Rewrap only this reviewed ready vault's verified small master envelope.
    /// Source records/metadata and the old platform entry remain unchanged. It
    /// holds read-only source leases that exclude writers and never repairs or
    /// initializes the original source database.
    pub fn rewrap_key(
        &self,
        target: &dyn PlatformKeyStore,
        expected: &VaultKeyMetadata,
    ) -> Result<bool, CredentialError> {
        let source = crate::metadata_view::MetadataView::open_existing(&self.root)?
            .ok_or(CredentialError::MissingKey)?;
        let metadata = source.metadata(&self.owner_scope_hash)?;
        if VaultKeyMetadata::from(&metadata) != *expected {
            return Err(CredentialError::Corrupt);
        }
        let envelope = self
            .keys
            .read(&metadata.account())?
            .ok_or(CredentialError::MissingKey)?;
        let _key = verified_source_key(&metadata, &envelope)?;
        source.revalidate(&self.root)?;
        let copied = crate::development::seal_existing(target, &metadata.account(), &envelope)?;
        source.revalidate(&self.root)?;
        Ok(copied)
    }
    /// Bind an existing owner-private root without creating any file or OS-store entry.
    pub fn new(
        root: ConfinedRoot,
        owner_scope: impl Into<String>,
    ) -> Result<Self, CredentialError> {
        Self::with_key_store(root, owner_scope, Arc::new(SystemKeyStore))
    }

    /// Bind an explicit platform-key adapter. Native composition and conformance tests
    /// use this seam; never substitute an insecure key store in production.
    pub fn with_key_store(
        root: ConfinedRoot,
        owner_scope: impl Into<String>,
        keys: Arc<dyn PlatformKeyStore>,
    ) -> Result<Self, CredentialError> {
        let scope = owner_scope.into();
        if scope.is_empty() || scope.len() > 512 || scope.chars().any(char::is_control) {
            return Err(CredentialError::InvalidInput);
        }
        root.revalidate().map_err(home_error)?;
        Ok(Self {
            root,
            owner_scope_hash: hex::encode(Sha256::digest(scope.as_bytes())),
            keys,
            opened: Mutex::new(None),
            #[cfg(test)]
            failure: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn fail_at(&self, checkpoint: InitializationCheckpoint) {
        *self.failure.lock().unwrap() = Some(checkpoint);
    }

    /// Irreversibly remove this vault's OS key for a confirmed application uninstall.
    ///
    /// The caller must own the private root, stop its consumers, and subsequently
    /// delete its files. This retains the exclusive vault lease and metadata so a
    /// failed cleanup can be retried. No records or key material are read, and a
    /// missing vault or already removed key succeeds without creating a new key.
    pub fn delete_key_for_uninstall(&self) -> Result<(), CredentialError> {
        let guard = self.opened(false)?;
        let Some(vault) = guard.as_ref() else {
            return Ok(());
        };
        let Some(metadata) = vault.metadata()? else {
            return Ok(());
        };
        metadata.validate(&self.owner_scope_hash)?;
        self.keys.delete(&metadata.account())
    }

    #[cfg(test)]
    fn checkpoint(&self, checkpoint: InitializationCheckpoint) -> Result<(), CredentialError> {
        let mut failure = self.failure.lock().unwrap();
        if *failure == Some(checkpoint) {
            *failure = None;
            return Err(CredentialError::Io);
        }
        Ok(())
    }

    fn opened(&self, create: bool) -> Result<MutexGuard<'_, Option<OpenedVault>>, CredentialError> {
        let mut guard = self.opened.lock().map_err(|_| CredentialError::Corrupt)?;
        self.root.revalidate().map_err(home_error)?;
        if let Some(vault) = guard.as_ref() {
            vault.revalidate(&self.root)?;
            return Ok(guard);
        }
        if !create {
            match self.root.open_existing_file(Path::new(DATABASE_FILE)) {
                Ok(_) => {}
                Err(HomeError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                    return Ok(guard);
                }
                Err(error) => return Err(home_error(error)),
            }
        }
        let lease = self
            .root
            .open_file(Path::new(LEASE_FILE))
            .map_err(home_error)?;
        if !lease
            .file()
            .try_lock_exclusive()
            .map_err(|_| CredentialError::Io)?
        {
            return Err(CredentialError::Busy);
        }
        let lease = crate::database::VaultLease(lease);
        let file = if create {
            self.root.open_file(Path::new(DATABASE_FILE))
        } else {
            self.root
                .open_existing_file_read_write(Path::new(DATABASE_FILE))
        }
        .map_err(home_error)?;
        let database = Database::builder()
            .create_file(file.file().try_clone().map_err(|_| CredentialError::Io)?)
            .map_err(|error| match error {
                redb::DatabaseError::DatabaseAlreadyOpen => CredentialError::Busy,
                redb::DatabaseError::Storage(redb::StorageError::Io(_)) => CredentialError::Io,
                _ => CredentialError::Corrupt,
            })?;
        let vault = OpenedVault {
            database,
            master: None,
            file,
            lease,
        };
        vault.revalidate(&self.root)?;
        *guard = Some(vault);
        #[cfg(test)]
        self.checkpoint(InitializationCheckpoint::DatabaseOpened)?;
        Ok(guard)
    }

    fn metadata(
        &self,
        vault: &OpenedVault,
        create: bool,
    ) -> Result<Option<Metadata>, CredentialError> {
        if let Some(metadata) = vault.metadata()? {
            metadata.validate(&self.owner_scope_hash)?;
            return Ok(Some(metadata));
        }
        if !create {
            return Err(CredentialError::Corrupt);
        }
        let metadata = Metadata {
            version: 1,
            vault_id: crypto::random_id()?,
            key_id: crypto::random_id()?,
            owner_scope_hash: self.owner_scope_hash.clone(),
            state: Initialization::PendingKey,
            verification: None,
        };
        vault.save_metadata(&metadata)?;
        #[cfg(test)]
        self.checkpoint(InitializationCheckpoint::PendingCommitted)?;
        Ok(Some(metadata))
    }

    fn ready_key(
        &self,
        vault: &mut OpenedVault,
        metadata: &mut Metadata,
        create: bool,
    ) -> Result<bool, CredentialError> {
        if metadata.state == Initialization::PendingKey && !create {
            return Ok(false);
        }
        if metadata.state == Initialization::PendingKey {
            // Sync the new Unix directory entries before publishing any OS key.
            // Repeat on recovery: the previous attempt may have failed at this boundary.
            self.root.sync_directory().map_err(home_error)?;
            #[cfg(test)]
            self.checkpoint(InitializationCheckpoint::PendingDirectorySynced)?;
        }
        let account = metadata.account();
        let key = if let Some(key) = vault.master.take() {
            key
        } else {
            match self.keys.read(&account)? {
                Some(bytes) => crypto::decode_key(&bytes, &metadata.vault_id, &metadata.key_id)?,
                None if metadata.state == Initialization::PendingKey && create => {
                    let key = crypto::new_key()?;
                    let envelope = crypto::encode_key(&metadata.vault_id, &metadata.key_id, &key)?;
                    self.keys.write(&account, &envelope)?;
                    #[cfg(test)]
                    self.checkpoint(InitializationCheckpoint::KeyStored)?;
                    let readback = self
                        .keys
                        .read(&account)?
                        .ok_or(CredentialError::MissingKey)?;
                    let verified =
                        crypto::decode_key(&readback, &metadata.vault_id, &metadata.key_id)?;
                    if !bool::from(key.as_ref().ct_eq(verified.as_ref())) {
                        return Err(CredentialError::Corrupt);
                    }
                    #[cfg(test)]
                    self.checkpoint(InitializationCheckpoint::KeyReadBack)?;
                    key
                }
                None => return Err(CredentialError::MissingKey),
            }
        };
        let aad = metadata.aad("verification");
        if metadata.state == Initialization::PendingKey {
            let encrypted = crypto::encrypt(&key, &aad, VERIFICATION)?;
            metadata.verification = Some(STANDARD.encode(encrypted));
            metadata.state = Initialization::Ready;
            vault.save_metadata(metadata)?;
            #[cfg(test)]
            self.checkpoint(InitializationCheckpoint::ReadyCommitted)?;
        } else {
            let value = metadata
                .verification
                .as_deref()
                .ok_or(CredentialError::Corrupt)?;
            if value.len() > 256 {
                return Err(CredentialError::Corrupt);
            }
            let encoded = STANDARD
                .decode(value)
                .map_err(|_| CredentialError::Corrupt)?;
            if crypto::decrypt(&key, &aad, &encoded)?.as_slice() != VERIFICATION {
                return Err(CredentialError::Corrupt);
            }
        }
        vault.master = Some(key);
        Ok(true)
    }
}

impl CredentialVault for PlatformCredentialVault {
    fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        let mut opened = self.opened(false)?;
        let Some(vault) = opened.as_mut() else {
            self.root.revalidate().map_err(home_error)?;
            return Ok(None);
        };
        let mut metadata = self
            .metadata(vault, false)?
            .ok_or(CredentialError::Corrupt)?;
        if !self.ready_key(vault, &mut metadata, false)? {
            vault.revalidate(&self.root)?;
            return Ok(None);
        }
        let master = vault.master.as_ref().ok_or(CredentialError::MissingKey)?;
        let read = vault
            .database
            .begin_read()
            .map_err(|_| CredentialError::Io)?;
        let table = read
            .open_table(RECORDS)
            .map_err(|_| CredentialError::Corrupt)?;
        let record_id = record_id(key);
        let Some(record) = table
            .get(record_id.as_str())
            .map_err(|_| CredentialError::Io)?
        else {
            vault.revalidate(&self.root)?;
            return Ok(None);
        };
        let mut plaintext = crypto::decrypt(master, &metadata.aad(&record_id), record.value())?;
        let result = VaultRecord::new(std::mem::take(&mut *plaintext))
            .map_err(|_| CredentialError::Corrupt)?;
        vault.revalidate(&self.root)?;
        Ok(Some(result))
    }

    fn write(&self, key: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
        let mut opened = self.opened(true)?;
        let vault = opened.as_mut().ok_or(CredentialError::Corrupt)?;
        let mut metadata = self
            .metadata(vault, true)?
            .ok_or(CredentialError::Corrupt)?;
        self.ready_key(vault, &mut metadata, true)?;
        let master = vault.master.as_ref().ok_or(CredentialError::MissingKey)?;
        let record_id = record_id(key);
        let encrypted = crypto::encrypt(master, &metadata.aad(&record_id), record.expose())?;
        let mut transaction = vault
            .database
            .begin_write()
            .map_err(|_| CredentialError::Io)?;
        transaction
            .set_durability(Durability::Immediate)
            .map_err(|_| CredentialError::Io)?;
        transaction
            .open_table(RECORDS)
            .map_err(|_| CredentialError::Corrupt)?
            .insert(record_id.as_str(), encrypted.as_slice())
            .map_err(|_| CredentialError::Io)?;
        #[cfg(test)]
        self.checkpoint(InitializationCheckpoint::BeforeRecordCommit)?;
        transaction.commit().map_err(|_| CredentialError::Io)?;
        #[cfg(test)]
        self.checkpoint(InitializationCheckpoint::RecordCommitted)?;
        vault.revalidate(&self.root)
    }

    fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
        let mut opened = self.opened(false)?;
        let Some(vault) = opened.as_mut() else {
            self.root.revalidate().map_err(home_error)?;
            return Ok(());
        };
        let mut metadata = self
            .metadata(vault, false)?
            .ok_or(CredentialError::Corrupt)?;
        if !self.ready_key(vault, &mut metadata, false)? {
            vault.revalidate(&self.root)?;
            return Ok(());
        }
        let mut transaction = vault
            .database
            .begin_write()
            .map_err(|_| CredentialError::Io)?;
        transaction
            .set_durability(Durability::Immediate)
            .map_err(|_| CredentialError::Io)?;
        transaction
            .open_table(RECORDS)
            .map_err(|_| CredentialError::Corrupt)?
            .remove(record_id(key).as_str())
            .map_err(|_| CredentialError::Io)?;
        transaction.commit().map_err(|_| CredentialError::Io)?;
        vault.revalidate(&self.root)
    }
}

pub(crate) fn record_id(key: &CredentialKey) -> String {
    format!("{}\0{}", key.purpose(), key.id())
}

fn verified_source_key(
    metadata: &Metadata,
    envelope: &[u8],
) -> Result<crypto::MasterKey, CredentialError> {
    let key = crypto::decode_key(envelope, &metadata.vault_id, &metadata.key_id)?;
    let verification = metadata
        .verification
        .as_deref()
        .ok_or(CredentialError::Corrupt)?;
    if verification.len() > 256 {
        return Err(CredentialError::Corrupt);
    }
    let encoded = STANDARD
        .decode(verification)
        .map_err(|_| CredentialError::Corrupt)?;
    if crypto::decrypt(&key, &metadata.aad("verification"), &encoded)?.as_slice() != VERIFICATION {
        return Err(CredentialError::Corrupt);
    }
    Ok(key)
}

pub(crate) fn home_error(error: HomeError) -> CredentialError {
    match error {
        HomeError::Io { .. } => CredentialError::Io,
        _ => CredentialError::Corrupt,
    }
}
