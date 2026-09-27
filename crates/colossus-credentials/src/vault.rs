//! Lazy protected-vault lifecycle. Reads never initialize a missing vault or key.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_contracts::{CredentialError, VaultRecord};
use colossus_home::{ConfinedRoot, HomeError};
use colossus_ports::{CredentialKey, CredentialVault};
use fs4::fs_std::FileExt as _;
use redb::{Database, Durability, ReadableDatabase as _};
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

fn home_error(error: HomeError) -> CredentialError {
    match error {
        HomeError::Io { .. } => CredentialError::Io,
        _ => CredentialError::Corrupt,
    }
}
