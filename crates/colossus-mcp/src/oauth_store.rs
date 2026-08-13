use async_trait::async_trait;
use chacha20poly1305::{
    KeyInit as _, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use colossus_ports::KeyProvider;
use redb::{Database, ReadableDatabase as _, TableDefinition, backends::InMemoryBackend};
use rmcp::transport::auth::{AuthError, CredentialStore, StoredCredentials};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{fs, fs::File, path::Path, sync::Arc};
use zeroize::{Zeroize as _, Zeroizing};

pub(super) const OAUTH_RECORDS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("mcp_oauth_credentials");
const KEY_DOMAIN: &[u8] = b"colossus-mcp-oauth-encrypted-state-v1";

#[derive(Clone)]
pub(super) enum OAuthStoreFactory {
    Platform {
        service: String,
        repository_id: String,
    },
    EncryptedState {
        database: Arc<Database>,
        keys: Arc<dyn KeyProvider>,
        repository_id: String,
    },
    PlaintextState {
        database: Arc<Database>,
        repository_id: String,
    },
}

impl OAuthStoreFactory {
    pub(super) fn platform(service: String, repository_id: String) -> Self {
        Self::Platform {
            service,
            repository_id,
        }
    }

    pub(super) fn ephemeral_state(repository_id: String) -> Result<Self, AuthError> {
        let database = Database::builder()
            .create_with_backend(InMemoryBackend::new())
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        initialize_database(&database)?;
        Ok(Self::PlaintextState {
            database: Arc::new(database),
            repository_id,
        })
    }

    pub(super) fn encrypted_state(
        path: &Path,
        keys: Arc<dyn KeyProvider>,
        repository_id: String,
    ) -> Result<Self, AuthError> {
        let database =
            Database::create(path).map_err(|error| AuthError::InternalError(error.to_string()))?;
        let write = database
            .begin_write()
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        write
            .open_table(OAUTH_RECORDS)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        write
            .commit()
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        Ok(Self::EncryptedState {
            database: Arc::new(database),
            keys,
            repository_id,
        })
    }

    pub(super) fn encrypted_state_file(
        file: File,
        keys: Arc<dyn KeyProvider>,
        repository_id: String,
    ) -> Result<Self, AuthError> {
        validate_owner_private_file(&file)?;
        let database = Database::builder()
            .create_file(file)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        initialize_database(&database)?;
        Ok(Self::EncryptedState {
            database: Arc::new(database),
            keys,
            repository_id,
        })
    }

    pub(super) fn plaintext_state(path: &Path, repository_id: String) -> Result<Self, AuthError> {
        prepare_owner_private_state(path)?;
        let database =
            Database::create(path).map_err(|error| AuthError::InternalError(error.to_string()))?;
        validate_owner_private_state(path)?;
        let write = database
            .begin_write()
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        write
            .open_table(OAUTH_RECORDS)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        write
            .commit()
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        Ok(Self::PlaintextState {
            database: Arc::new(database),
            repository_id,
        })
    }

    pub(super) fn plaintext_state_file(
        file: File,
        repository_id: String,
    ) -> Result<Self, AuthError> {
        validate_owner_private_file(&file)?;
        let database = Database::builder()
            .create_file(file)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        initialize_database(&database)?;
        Ok(Self::PlaintextState {
            database: Arc::new(database),
            repository_id,
        })
    }

    pub(super) fn store(&self, server: &str, endpoint: &str) -> OAuthCredentialStore {
        let identity = identity(
            match self {
                Self::Platform { repository_id, .. }
                | Self::EncryptedState { repository_id, .. }
                | Self::PlaintextState { repository_id, .. } => repository_id,
            },
            server,
            endpoint,
        );
        match self {
            Self::Platform { service, .. } => OAuthCredentialStore::Platform {
                service: service.clone(),
                account: format!("mcp-oauth:{}", hex::encode(Sha256::digest(&identity))),
            },
            Self::EncryptedState { database, keys, .. } => OAuthCredentialStore::EncryptedState {
                database: Arc::clone(database),
                keys: Arc::clone(keys),
                identity,
            },
            Self::PlaintextState { database, .. } => OAuthCredentialStore::PlaintextState {
                database: Arc::clone(database),
                identity,
            },
        }
    }
}

#[derive(Clone)]
pub(super) enum OAuthCredentialStore {
    Platform {
        service: String,
        account: String,
    },
    EncryptedState {
        database: Arc<Database>,
        keys: Arc<dyn KeyProvider>,
        identity: String,
    },
    PlaintextState {
        database: Arc<Database>,
        identity: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptedOAuthRecord {
    schema_version: u16,
    key_id: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaintextOAuthRecord {
    schema_version: u16,
    credentials: StoredCredentials,
}

#[async_trait]
impl CredentialStore for OAuthCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self {
            Self::Platform { service, account } => {
                let entry = keyring::Entry::new(service, account)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let encoded = match entry.get_password() {
                    Ok(value) => value,
                    Err(keyring::Error::NoEntry) => return Ok(None),
                    Err(error) => return Err(AuthError::InternalError(error.to_string())),
                };
                serde_json::from_str(&encoded)
                    .map(Some)
                    .map_err(|error| AuthError::InternalError(error.to_string()))
            }
            Self::EncryptedState {
                database,
                keys,
                identity,
            } => {
                let record: EncryptedOAuthRecord = {
                    let read = database
                        .begin_read()
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    let table = read
                        .open_table(OAUTH_RECORDS)
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    let Some(record) = table
                        .get(identity.as_str())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?
                    else {
                        return Ok(None);
                    };
                    serde_json::from_slice(record.value())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?
                };
                if record.schema_version != 1 {
                    return Err(AuthError::InternalError(
                        "unsupported encrypted OAuth record".into(),
                    ));
                }
                let mut key = keys
                    .key_by_id(&record.key_id)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let mut derived = derive_key(&key);
                key.zeroize();
                let nonce: [u8; 24] = hex::decode(&record.nonce)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?
                    .try_into()
                    .map_err(|_| AuthError::InternalError("OAuth nonce is invalid".into()))?;
                let ciphertext = hex::decode(&record.ciphertext)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let mut plaintext = XChaCha20Poly1305::new((&derived).into())
                    .decrypt(
                        XNonce::from_slice(&nonce),
                        Payload {
                            msg: &ciphertext,
                            aad: associated_data(identity, &record.key_id).as_bytes(),
                        },
                    )
                    .map_err(|_| {
                        AuthError::InternalError("OAuth credential decryption failed".into())
                    })?;
                derived.zeroize();
                let credentials: StoredCredentials = serde_json::from_slice(&plaintext)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                plaintext.zeroize();
                let (active_id, mut active_key) = keys
                    .active_key()
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                active_key.zeroize();
                if active_id != record.key_id {
                    self.save(credentials.clone()).await?;
                }
                Ok(Some(credentials))
            }
            Self::PlaintextState { database, identity } => {
                let record: PlaintextOAuthRecord = {
                    let read = database
                        .begin_read()
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    let table = read
                        .open_table(OAUTH_RECORDS)
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    let Some(record) = table
                        .get(identity.as_str())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?
                    else {
                        return Ok(None);
                    };
                    serde_json::from_slice(record.value())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?
                };
                if record.schema_version != 1 {
                    return Err(AuthError::InternalError(
                        "unsupported plaintext OAuth record".into(),
                    ));
                }
                Ok(Some(record.credentials))
            }
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let mut plaintext = serde_json::to_vec(&credentials)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        match self {
            Self::Platform { service, account } => {
                let entry = keyring::Entry::new(service, account)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let mut encoded = String::from_utf8(plaintext)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let result = entry
                    .set_password(&encoded)
                    .map_err(|error| AuthError::InternalError(error.to_string()));
                encoded.zeroize();
                result
            }
            Self::EncryptedState {
                database,
                keys,
                identity,
            } => {
                let (key_id, mut key) = keys
                    .active_key()
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let mut derived = derive_key(&key);
                key.zeroize();
                let mut nonce = [0_u8; 24];
                getrandom::fill(&mut nonce)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let ciphertext = XChaCha20Poly1305::new((&derived).into())
                    .encrypt(
                        XNonce::from_slice(&nonce),
                        Payload {
                            msg: &plaintext,
                            aad: associated_data(identity, &key_id).as_bytes(),
                        },
                    )
                    .map_err(|_| {
                        AuthError::InternalError("OAuth credential encryption failed".into())
                    })?;
                derived.zeroize();
                plaintext.zeroize();
                let record = serde_json::to_vec(&EncryptedOAuthRecord {
                    schema_version: 1,
                    key_id,
                    nonce: hex::encode(nonce),
                    ciphertext: hex::encode(ciphertext),
                })
                .map_err(|error| AuthError::InternalError(error.to_string()))?;
                let write = database
                    .begin_write()
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                {
                    let mut table = write
                        .open_table(OAUTH_RECORDS)
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    table
                        .insert(identity.as_str(), record.as_slice())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                }
                write
                    .commit()
                    .map_err(|error| AuthError::InternalError(error.to_string()))
            }
            Self::PlaintextState { database, identity } => {
                let record = Zeroizing::new(
                    serde_json::to_vec(&PlaintextOAuthRecord {
                        schema_version: 1,
                        credentials,
                    })
                    .map_err(|error| AuthError::InternalError(error.to_string()))?,
                );
                plaintext.zeroize();
                let write = database
                    .begin_write()
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                {
                    let mut table = write
                        .open_table(OAUTH_RECORDS)
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    table
                        .insert(identity.as_str(), record.as_slice())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                }
                write
                    .commit()
                    .map_err(|error| AuthError::InternalError(error.to_string()))
            }
        }
    }

    async fn clear(&self) -> Result<(), AuthError> {
        match self {
            Self::Platform { service, account } => {
                let entry = keyring::Entry::new(service, account)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(error) => Err(AuthError::InternalError(error.to_string())),
                }
            }
            Self::EncryptedState {
                database, identity, ..
            }
            | Self::PlaintextState { database, identity } => {
                let write = database
                    .begin_write()
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
                {
                    let mut table = write
                        .open_table(OAUTH_RECORDS)
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                    table
                        .remove(identity.as_str())
                        .map_err(|error| AuthError::InternalError(error.to_string()))?;
                }
                write
                    .commit()
                    .map_err(|error| AuthError::InternalError(error.to_string()))
            }
        }
    }
}

fn prepare_owner_private_state(path: &Path) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| AuthError::InternalError(error.to_string()))?;
    }
    match fs::symlink_metadata(path) {
        Ok(_) => validate_owner_private_state(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                fs::OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .mode(0o600)
                    .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                    .open(path)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
            }
            #[cfg(not(unix))]
            {
                fs::OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|error| AuthError::InternalError(error.to_string()))?;
            }
            validate_owner_private_state(path)
        }
        Err(error) => Err(AuthError::InternalError(error.to_string())),
    }
}

fn validate_owner_private_state(path: &Path) -> Result<(), AuthError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| AuthError::InternalError(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AuthError::InternalError(
            "plaintext OAuth state must be a regular non-symlink file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o777 != 0o600
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
        {
            return Err(AuthError::InternalError(
                "plaintext OAuth state must be a current-user owner-only single-link file".into(),
            ));
        }
    }
    Ok(())
}

fn validate_owner_private_file(file: &File) -> Result<(), AuthError> {
    let metadata = file
        .metadata()
        .map_err(|error| AuthError::InternalError(error.to_string()))?;
    if !metadata.is_file() {
        return Err(AuthError::InternalError(
            "OAuth state must be a regular file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o777 != 0o600
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
        {
            return Err(AuthError::InternalError(
                "OAuth state must be a current-user owner-only single-link file".into(),
            ));
        }
    }
    Ok(())
}

fn initialize_database(database: &Database) -> Result<(), AuthError> {
    let write = database
        .begin_write()
        .map_err(|error| AuthError::InternalError(error.to_string()))?;
    write
        .open_table(OAUTH_RECORDS)
        .map_err(|error| AuthError::InternalError(error.to_string()))?;
    write
        .commit()
        .map_err(|error| AuthError::InternalError(error.to_string()))
}

fn identity(repository_id: &str, server: &str, endpoint: &str) -> String {
    format!("{repository_id}\0{server}\0{endpoint}")
}

fn associated_data(identity: &str, key_id: &str) -> String {
    format!("mcp-oauth-v1\0{identity}\0{key_id}")
}

fn derive_key(key: &[u8; 32]) -> [u8; 32] {
    Sha256::new()
        .chain_update(KEY_DOMAIN)
        .chain_update(key)
        .finalize()
        .into()
}
