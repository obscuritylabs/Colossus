//! Encrypted, hash-chained redb event journal.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use colossus_contracts::{
    EncryptedPayload, EventEnvelope, NewEvent, ProjectionBatch, ProjectionMutation,
    ProjectionWorkItem, SignedCheckpoint,
};
use colossus_ports::{
    CheckpointSigner, EventJournal, KeyProvider, ProjectionStore, StoreError, VerificationReport,
};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use fs4::fs_std::FileExt as _;
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
const STREAM_VERSIONS: TableDefinition<&str, u64> = TableDefinition::new("stream_versions");
const METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");
const OUTBOX: TableDefinition<u64, &[u8]> = TableDefinition::new("projection_outbox");
const PROJECTION_POSITIONS: TableDefinition<&str, u64> =
    TableDefinition::new("projection_positions");
const PROJECTION_RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("projection_records");
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const CHECKPOINT_INTERVAL: u64 = 100;
const CHECKPOINT_MAX_AGE: Duration = Duration::from_secs(60);

fn adapter_error(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn utc_now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(adapter_error)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn projection_record_key(projection: &str, key: &str) -> Result<String, StoreError> {
    if projection.is_empty() || projection.contains('\0') {
        return Err(StoreError::Adapter(
            "projection name must be nonempty and contain no NUL".into(),
        ));
    }
    if key.is_empty() || key.contains('\0') {
        return Err(StoreError::Adapter(
            "projection key must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(format!("{projection}\0{key}"))
}

fn projection_prefix(projection: &str) -> Result<String, StoreError> {
    if projection.is_empty() || projection.contains('\0') {
        return Err(StoreError::Adapter(
            "projection name must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(format!("{projection}\0"))
}

/// Exclusive process-level lease for the canonical redb writer.
pub struct RedbWriterLease {
    file: File,
    path: PathBuf,
}

impl RedbWriterLease {
    /// Acquire the non-blocking writer lease associated with a redb state path.
    pub fn acquire(state_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let state_path = state_path.as_ref();
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).map_err(adapter_error)?;
        }
        let mut lock_name = state_path.as_os_str().to_os_string();
        lock_name.push(".writer.lock");
        let path = PathBuf::from(lock_name);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(adapter_error)?;
        if !file.try_lock_exclusive().map_err(adapter_error)? {
            return Err(StoreError::Adapter(format!(
                "redb writer lease is already held: {}",
                path.display()
            )));
        }
        Ok(Self { file, path })
    }

    /// Lock file used to coordinate embedded and worker writers.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RedbWriterLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Explicit in-memory key provider for tests and embedded applications.
pub struct StaticKeyProvider {
    active_id: Mutex<String>,
    keys: Mutex<BTreeMap<String, [u8; 32]>>,
    anchor: Mutex<Option<(u64, String)>>,
}

impl StaticKeyProvider {
    /// Create a provider with one active key.
    pub fn new(key_id: impl Into<String>, key: [u8; 32]) -> Self {
        let active_id = key_id.into();
        let mut keys = BTreeMap::new();
        keys.insert(active_id.clone(), key);
        Self {
            active_id: Mutex::new(active_id),
            keys: Mutex::new(keys),
            anchor: Mutex::new(None),
        }
    }

    /// Add a new key and atomically make it active while retaining historical keys.
    pub fn rotate(&self, key_id: impl Into<String>, key: [u8; 32]) -> Result<(), StoreError> {
        let key_id = key_id.into();
        self.keys
            .lock()
            .map_err(adapter_error)?
            .insert(key_id.clone(), key);
        *self.active_id.lock().map_err(adapter_error)? = key_id;
        Ok(())
    }
}

impl KeyProvider for StaticKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        let active_id = self.active_id.lock().map_err(adapter_error)?.clone();
        let key = self
            .keys
            .lock()
            .map_err(adapter_error)?
            .get(&active_id)
            .copied()
            .ok_or_else(|| StoreError::KeyUnavailable(active_id.clone()))?;
        Ok((active_id, key))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        self.keys
            .lock()
            .map_err(adapter_error)?
            .get(key_id)
            .copied()
            .ok_or_else(|| StoreError::KeyUnavailable(key_id.to_owned()))
    }

    fn store_anchor(&self, sequence: u64, hash: &str) -> Result<(), StoreError> {
        *self.anchor.lock().map_err(adapter_error)? = Some((sequence, hash.to_owned()));
        Ok(())
    }

    fn load_anchor(&self) -> Result<Option<(u64, String)>, StoreError> {
        Ok(self.anchor.lock().map_err(adapter_error)?.clone())
    }
}

/// Environment-backed encryption key with a separate local secure-anchor file.
pub struct EnvironmentKeyProvider {
    variable: String,
    key_id: String,
    anchor_path: PathBuf,
}

impl EnvironmentKeyProvider {
    /// Configure an explicit environment variable and anchor path.
    pub fn new(
        variable: impl Into<String>,
        key_id: impl Into<String>,
        anchor_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            variable: variable.into(),
            key_id: key_id.into(),
            anchor_path: anchor_path.into(),
        }
    }

    fn read_key(&self) -> Result<[u8; 32], StoreError> {
        let encoded = std::env::var(&self.variable).map_err(|_| {
            StoreError::KeyUnavailable(format!("environment variable {} is unset", self.variable))
        })?;
        let bytes = hex::decode(&encoded)
            .or_else(|_| BASE64.decode(&encoded))
            .map_err(|_| {
                StoreError::KeyUnavailable(format!(
                    "{} must contain 32 bytes encoded as hex or base64",
                    self.variable
                ))
            })?;
        bytes.try_into().map_err(|_| {
            StoreError::KeyUnavailable(format!("{} must decode to exactly 32 bytes", self.variable))
        })
    }
}

impl KeyProvider for EnvironmentKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok((self.key_id.clone(), self.read_key()?))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        if key_id != self.key_id {
            return Err(StoreError::KeyUnavailable(format!(
                "historical key {key_id} is not configured"
            )));
        }
        self.read_key()
    }

    fn store_anchor(&self, sequence: u64, hash: &str) -> Result<(), StoreError> {
        if let Some(parent) = self.anchor_path.parent() {
            fs::create_dir_all(parent).map_err(adapter_error)?;
        }
        let temporary = self.anchor_path.with_extension("tmp");
        let body = serde_json::to_vec(&json!({"sequence": sequence, "hash": hash}))
            .map_err(adapter_error)?;
        fs::write(&temporary, body).map_err(adapter_error)?;
        fs::rename(temporary, &self.anchor_path).map_err(adapter_error)
    }

    fn load_anchor(&self) -> Result<Option<(u64, String)>, StoreError> {
        if !self.anchor_path.exists() {
            return Ok(None);
        }
        let value: Value =
            serde_json::from_slice(&fs::read(&self.anchor_path).map_err(adapter_error)?)
                .map_err(adapter_error)?;
        let sequence = value
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| StoreError::Verification("secure anchor has no sequence".into()))?;
        let hash = value
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Verification("secure anchor has no hash".into()))?;
        Ok(Some((sequence, hash.to_owned())))
    }
}

/// OS keychain/DPAPI/Secret Service key provider with a separately protected anchor.
pub struct PlatformKeyProvider {
    service: String,
    key_id: String,
}

impl PlatformKeyProvider {
    /// Load or create the active journal key in the platform credential store.
    pub fn new(service: impl Into<String>, key_id: impl Into<String>) -> Result<Self, StoreError> {
        let provider = Self {
            service: service.into(),
            key_id: key_id.into(),
        };
        platform_secret(
            &provider.service,
            &format!("journal-key:{}", provider.key_id),
        )?;
        Ok(provider)
    }

    fn key_account(&self, key_id: &str) -> String {
        format!("journal-key:{key_id}")
    }

    fn anchor_account(&self) -> String {
        format!("journal-anchor:{}", self.key_id)
    }
}

/// Load or create exactly 32 random bytes in the configured platform credential store.
pub fn platform_secret(service: &str, account: &str) -> Result<[u8; 32], StoreError> {
    let entry = keyring::Entry::new(service, account).map_err(adapter_error)?;
    let secret = match entry.get_secret() {
        Ok(secret) => secret,
        Err(keyring::Error::NoEntry) => {
            let mut secret = [0_u8; 32];
            getrandom::fill(&mut secret).map_err(adapter_error)?;
            entry.set_secret(&secret).map_err(adapter_error)?;
            secret.to_vec()
        }
        Err(error) => return Err(adapter_error(error)),
    };
    secret.try_into().map_err(|_| {
        StoreError::KeyUnavailable(format!(
            "platform credential {service}/{account} is not 32 bytes"
        ))
    })
}

fn platform_existing_secret(service: &str, account: &str) -> Result<[u8; 32], StoreError> {
    let entry = keyring::Entry::new(service, account).map_err(adapter_error)?;
    let secret = entry.get_secret().map_err(|error| match error {
        keyring::Error::NoEntry => {
            StoreError::KeyUnavailable(format!("platform credential {service}/{account} is absent"))
        }
        other => adapter_error(other),
    })?;
    secret.try_into().map_err(|_| {
        StoreError::KeyUnavailable(format!(
            "platform credential {service}/{account} is not 32 bytes"
        ))
    })
}

impl KeyProvider for PlatformKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok((
            self.key_id.clone(),
            platform_secret(&self.service, &self.key_account(&self.key_id))?,
        ))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        platform_existing_secret(&self.service, &self.key_account(key_id))
    }

    fn store_anchor(&self, sequence: u64, hash: &str) -> Result<(), StoreError> {
        let entry =
            keyring::Entry::new(&self.service, &self.anchor_account()).map_err(adapter_error)?;
        let body = serde_json::to_vec(&json!({"sequence": sequence, "hash": hash}))
            .map_err(adapter_error)?;
        entry.set_secret(&body).map_err(adapter_error)
    }

    fn load_anchor(&self) -> Result<Option<(u64, String)>, StoreError> {
        let entry =
            keyring::Entry::new(&self.service, &self.anchor_account()).map_err(adapter_error)?;
        let body = match entry.get_secret() {
            Ok(body) => body,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(adapter_error(error)),
        };
        let value: Value = serde_json::from_slice(&body).map_err(adapter_error)?;
        let sequence = value
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| StoreError::Verification("secure anchor has no sequence".into()))?;
        let hash = value
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Verification("secure anchor has no hash".into()))?;
        Ok(Some((sequence, hash.into())))
    }
}

/// Ed25519 checkpoint signer created from explicit secret key bytes.
pub struct Ed25519CheckpointSigner {
    key_id: String,
    signing_key: SigningKey,
}

impl Ed25519CheckpointSigner {
    /// Construct a signer from a 32-byte Ed25519 secret key.
    pub fn new(key_id: impl Into<String>, secret: [u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            signing_key: SigningKey::from_bytes(&secret),
        }
    }

    /// Public verification key bytes for external anchor verification.
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}

impl CheckpointSigner for Ed25519CheckpointSigner {
    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, StoreError> {
        Ok(self.signing_key.sign(message).to_bytes().to_vec())
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), StoreError> {
        let signature = Signature::from_slice(signature)
            .map_err(|error| StoreError::Verification(error.to_string()))?;
        let key: VerifyingKey = self.signing_key.verifying_key();
        key.verify(message, &signature)
            .map_err(|error| StoreError::Verification(error.to_string()))
    }
}

#[derive(Serialize)]
struct AssociatedData<'a> {
    schema_version: u16,
    event_version: u16,
    event_id: &'a str,
    global_sequence: u64,
    stream_id: &'a str,
    stream_version: u64,
    classification: &'a colossus_contracts::EventClassification,
    event_type: &'a str,
    actor: &'a colossus_contracts::Actor,
    context: &'a colossus_contracts::ExecutionContext,
    occurred_at: &'a str,
}

#[derive(Serialize)]
struct RecordHashInput<'a> {
    associated_data: AssociatedData<'a>,
    payload: &'a EncryptedPayload,
    previous_hash: &'a str,
}

fn associated_data(envelope: &EventEnvelope) -> AssociatedData<'_> {
    AssociatedData {
        schema_version: envelope.schema_version,
        event_version: envelope.event_version,
        event_id: &envelope.event_id,
        global_sequence: envelope.global_sequence,
        stream_id: &envelope.stream_id,
        stream_version: envelope.stream_version,
        classification: &envelope.classification,
        event_type: &envelope.event_type,
        actor: &envelope.actor,
        context: &envelope.context,
        occurred_at: &envelope.occurred_at,
    }
}

fn record_hash(envelope: &EventEnvelope) -> Result<String, StoreError> {
    let input = RecordHashInput {
        associated_data: associated_data(envelope),
        payload: &envelope.payload,
        previous_hash: &envelope.previous_hash,
    };
    Ok(sha256_hex(
        &serde_json::to_vec(&input).map_err(adapter_error)?,
    ))
}

fn checkpoint_message(sequence: u64, hash: &str) -> Vec<u8> {
    format!("colossus-checkpoint-v1\n{sequence}\n{hash}\n").into_bytes()
}

/// Canonical redb journal adapter.
pub struct RedbEventJournal {
    database: Database,
    keys: Arc<dyn KeyProvider>,
    signer: Arc<dyn CheckpointSigner>,
    writer: Mutex<()>,
    last_checkpoint: Mutex<Instant>,
    recovery_mode: AtomicBool,
    recovery_reason: Mutex<Option<String>>,
}

impl RedbEventJournal {
    /// Open or create a journal, then verify it before enabling writes.
    pub fn open(
        path: impl AsRef<Path>,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        let database = Database::create(path).map_err(adapter_error)?;
        let write = database.begin_write().map_err(adapter_error)?;
        write.open_table(EVENTS).map_err(adapter_error)?;
        write.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        write.open_table(METADATA).map_err(adapter_error)?;
        write.open_table(OUTBOX).map_err(adapter_error)?;
        write
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        write
            .open_table(PROJECTION_RECORDS)
            .map_err(adapter_error)?;
        write.commit().map_err(adapter_error)?;
        let journal = Self {
            database,
            keys,
            signer,
            writer: Mutex::new(()),
            last_checkpoint: Mutex::new(Instant::now()),
            recovery_mode: AtomicBool::new(false),
            recovery_reason: Mutex::new(None),
        };
        if let Err(error) = journal.verify_inner() {
            journal.recovery_mode.store(true, Ordering::Release);
            *journal.recovery_reason.lock().map_err(adapter_error)? = Some(error.to_string());
        }
        Ok(journal)
    }

    /// Bounded reason startup entered recovery mode.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        Ok(self.recovery_reason.lock().map_err(adapter_error)?.clone())
    }

    fn encrypt_payload(
        &self,
        envelope: &EventEnvelope,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, StoreError> {
        let (key_id, key) = self.keys.active_key()?;
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).map_err(adapter_error)?;
        let aad = serde_json::to_vec(&associated_data(envelope)).map_err(adapter_error)?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(adapter_error)?;
        Ok(EncryptedPayload {
            key_id,
            algorithm: "XChaCha20-Poly1305".into(),
            nonce: hex::encode(nonce),
            ciphertext: hex::encode(ciphertext),
            plaintext_hash: sha256_hex(plaintext),
        })
    }

    fn append_locked(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let write = self.database.begin_write().map_err(adapter_error)?;
        let mut persisted = Vec::with_capacity(events.len());
        {
            let mut event_table = write.open_table(EVENTS).map_err(adapter_error)?;
            let mut stream_table = write.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            let mut outbox = write.open_table(OUTBOX).map_err(adapter_error)?;
            let mut sequence = metadata
                .get("last_sequence")
                .map_err(adapter_error)?
                .map_or(Ok(0_u64), |value| {
                    serde_json::from_slice(value.value()).map_err(adapter_error)
                })?;
            let mut previous_hash = metadata
                .get("last_hash")
                .map_err(adapter_error)?
                .map_or_else(
                    || Ok::<String, StoreError>(ZERO_HASH.into()),
                    |value| serde_json::from_slice(value.value()).map_err(adapter_error),
                )?;
            let mut batch_versions = BTreeMap::<String, u64>::new();

            for event in events {
                let durable_version = if let Some(version) = batch_versions.get(&event.stream_id) {
                    *version
                } else {
                    stream_table
                        .get(event.stream_id.as_str())
                        .map_err(adapter_error)?
                        .map_or(0, |value| value.value())
                };
                if event.expected_stream_version != durable_version {
                    return Err(StoreError::Conflict {
                        stream_id: event.stream_id,
                        expected: event.expected_stream_version,
                        actual: durable_version,
                    });
                }

                sequence = sequence.saturating_add(1);
                let stream_version = durable_version.saturating_add(1);
                let mut envelope = EventEnvelope {
                    schema_version: 1,
                    event_version: event.event_version,
                    event_id: Uuid::now_v7().to_string(),
                    global_sequence: sequence,
                    stream_id: event.stream_id,
                    stream_version,
                    classification: event.classification,
                    event_type: event.event_type,
                    actor: event.actor,
                    context: event.context,
                    occurred_at: utc_now()?,
                    payload: EncryptedPayload {
                        key_id: String::new(),
                        algorithm: String::new(),
                        nonce: String::new(),
                        ciphertext: String::new(),
                        plaintext_hash: String::new(),
                    },
                    previous_hash: previous_hash.clone(),
                    record_hash: String::new(),
                };
                let plaintext = serde_json::to_vec(&event.payload).map_err(adapter_error)?;
                envelope.payload = self.encrypt_payload(&envelope, &plaintext)?;
                envelope.record_hash = record_hash(&envelope)?;
                previous_hash.clone_from(&envelope.record_hash);
                let encoded = serde_json::to_vec(&envelope).map_err(adapter_error)?;
                event_table
                    .insert(sequence, encoded.as_slice())
                    .map_err(adapter_error)?;
                stream_table
                    .insert(envelope.stream_id.as_str(), stream_version)
                    .map_err(adapter_error)?;
                let outbox_record = serde_json::to_vec(&json!({
                    "event_id": envelope.event_id,
                    "global_sequence": sequence,
                    "status": "pending"
                }))
                .map_err(adapter_error)?;
                outbox
                    .insert(sequence, outbox_record.as_slice())
                    .map_err(adapter_error)?;
                batch_versions.insert(envelope.stream_id.clone(), stream_version);
                persisted.push(envelope);
            }
            let sequence_bytes = serde_json::to_vec(&sequence).map_err(adapter_error)?;
            let hash_bytes = serde_json::to_vec(&previous_hash).map_err(adapter_error)?;
            metadata
                .insert("last_sequence", sequence_bytes.as_slice())
                .map_err(adapter_error)?;
            metadata
                .insert("last_hash", hash_bytes.as_slice())
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)?;
        Ok(persisted)
    }

    fn decrypt(&self, event: &EventEnvelope) -> Result<Vec<u8>, StoreError> {
        if event.payload.algorithm != "XChaCha20-Poly1305" {
            return Err(StoreError::Verification(format!(
                "unsupported payload algorithm {}",
                event.payload.algorithm
            )));
        }
        let key = self.keys.key_by_id(&event.payload.key_id)?;
        let nonce = hex::decode(&event.payload.nonce).map_err(adapter_error)?;
        let nonce: [u8; 24] = nonce
            .try_into()
            .map_err(|_| StoreError::Verification("invalid XChaCha20 nonce length".into()))?;
        let ciphertext = hex::decode(&event.payload.ciphertext).map_err(adapter_error)?;
        let aad = serde_json::to_vec(&associated_data(event)).map_err(adapter_error)?;
        XChaCha20Poly1305::new((&key).into())
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| {
                StoreError::Verification(format!(
                    "event {} payload authentication failed",
                    event.event_id
                ))
            })
    }

    fn verify_checkpoint(
        &self,
        checkpoint: &SignedCheckpoint,
        event_hashes: &BTreeMap<u64, String>,
    ) -> Result<(), StoreError> {
        if checkpoint.algorithm != "Ed25519" || checkpoint.key_id != self.signer.key_id() {
            return Err(StoreError::Verification(
                "checkpoint signer identity or algorithm mismatch".into(),
            ));
        }
        if event_hashes.get(&checkpoint.global_sequence) != Some(&checkpoint.record_hash) {
            return Err(StoreError::Verification(
                "checkpoint does not match journal record".into(),
            ));
        }
        let signature = hex::decode(&checkpoint.signature).map_err(adapter_error)?;
        self.signer.verify(
            &checkpoint_message(checkpoint.global_sequence, &checkpoint.record_hash),
            &signature,
        )
    }

    fn verify_inner(&self) -> Result<VerificationReport, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let event_table = read.open_table(EVENTS).map_err(adapter_error)?;
        let durable_stream_table = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let outbox = read.open_table(OUTBOX).map_err(adapter_error)?;
        let projection_positions = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        let projection_records = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        let mut expected_sequence = 1_u64;
        let mut previous_hash = ZERO_HASH.to_owned();
        let mut stream_versions = BTreeMap::<String, u64>::new();
        let mut event_hashes = BTreeMap::<u64, String>::new();

        for entry in event_table.iter().map_err(adapter_error)? {
            let (key, value) = entry.map_err(adapter_error)?;
            let sequence = key.value();
            if sequence != expected_sequence {
                return Err(StoreError::Verification(format!(
                    "global sequence gap: expected {expected_sequence}, got {sequence}"
                )));
            }
            let envelope: EventEnvelope =
                serde_json::from_slice(value.value()).map_err(adapter_error)?;
            if envelope.global_sequence != sequence || envelope.previous_hash != previous_hash {
                return Err(StoreError::Verification(format!(
                    "event {} sequence or previous hash mismatch",
                    envelope.event_id
                )));
            }
            let expected_stream = stream_versions
                .get(&envelope.stream_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            if envelope.stream_version != expected_stream {
                return Err(StoreError::Verification(format!(
                    "stream {} version mismatch",
                    envelope.stream_id
                )));
            }
            let computed_hash = record_hash(&envelope)?;
            if computed_hash != envelope.record_hash {
                return Err(StoreError::Verification(format!(
                    "event {} record hash mismatch",
                    envelope.event_id
                )));
            }
            let plaintext = self.decrypt(&envelope)?;
            if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
                return Err(StoreError::Verification(format!(
                    "event {} plaintext hash mismatch",
                    envelope.event_id
                )));
            }
            serde_json::from_slice::<Value>(&plaintext).map_err(adapter_error)?;
            previous_hash.clone_from(&envelope.record_hash);
            event_hashes.insert(sequence, envelope.record_hash);
            let queued = outbox
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "projection outbox record {sequence} is absent"
                    ))
                })?;
            let queued: Value = serde_json::from_slice(queued.value()).map_err(adapter_error)?;
            if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id) {
                return Err(StoreError::Verification(format!(
                    "projection outbox record {sequence} targets a different event"
                )));
            }
            stream_versions.insert(envelope.stream_id, envelope.stream_version);
            expected_sequence = expected_sequence.saturating_add(1);
        }

        let last_sequence = expected_sequence.saturating_sub(1);
        let metadata_sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let metadata_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| serde_json::from_slice(value.value()).map_err(adapter_error),
            )?;
        if metadata_sequence != last_sequence || metadata_hash != previous_hash {
            return Err(StoreError::Verification(
                "journal head metadata does not match event chain".into(),
            ));
        }
        if outbox.len().map_err(adapter_error)? != last_sequence {
            return Err(StoreError::Verification(
                "projection outbox position does not match journal head".into(),
            ));
        }
        let mut durable_stream_versions = BTreeMap::new();
        for entry in durable_stream_table.iter().map_err(adapter_error)? {
            let (stream_id, version) = entry.map_err(adapter_error)?;
            durable_stream_versions.insert(stream_id.value().to_owned(), version.value());
        }
        if durable_stream_versions != stream_versions {
            return Err(StoreError::Verification(
                "durable stream versions do not match journal replay".into(),
            ));
        }
        for entry in projection_positions.iter().map_err(adapter_error)? {
            let (projection, position) = entry.map_err(adapter_error)?;
            projection_prefix(projection.value())?;
            if position.value() > last_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {} position {} is ahead of journal head {last_sequence}",
                    projection.value(),
                    position.value()
                )));
            }
        }
        for entry in projection_records.iter().map_err(adapter_error)? {
            let (key, value) = entry.map_err(adapter_error)?;
            let Some((projection, record_key)) = key.value().split_once('\0') else {
                return Err(StoreError::Verification(
                    "projection record key has no namespace delimiter".into(),
                ));
            };
            projection_record_key(projection, record_key)?;
            serde_json::from_slice::<Value>(value.value()).map_err(|error| {
                StoreError::Verification(format!(
                    "projection record {} is invalid JSON: {error}",
                    key.value()
                ))
            })?;
        }

        let checkpoint = metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()?;
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, &event_hashes)?;
        }
        if let Some((anchor_sequence, anchor_hash)) = self.keys.load_anchor()?
            && event_hashes.get(&anchor_sequence) != Some(&anchor_hash)
        {
            return Err(StoreError::Verification(
                "secure anchor is missing or differs from journal".into(),
            ));
        }
        Ok(VerificationReport {
            event_count: last_sequence,
            last_sequence,
            last_hash: previous_hash,
            checkpoint,
        })
    }
}

impl EventJournal for RedbEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        let mut events = self.append_batch(vec![event])?;
        events
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let persisted = {
            let _guard = self.writer.lock().map_err(adapter_error)?;
            self.append_locked(events)?
        };
        let count_due = persisted
            .last()
            .is_some_and(|event| event.global_sequence % CHECKPOINT_INTERVAL == 0);
        let age_due = self
            .last_checkpoint
            .lock()
            .map_err(adapter_error)?
            .elapsed()
            >= CHECKPOINT_MAX_AGE;
        if count_due || age_due {
            self.checkpoint()?;
        }
        Ok(persisted)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(EVENTS).map_err(adapter_error)?;
        let mut events = Vec::new();
        for entry in table.iter().map_err(adapter_error)? {
            let (_, value) = entry.map_err(adapter_error)?;
            let event: EventEnvelope =
                serde_json::from_slice(value.value()).map_err(adapter_error)?;
            if event.stream_id == stream_id {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(EVENTS).map_err(adapter_error)?;
        let mut events = Vec::with_capacity(limit.min(1024));
        for entry in table.range(from_sequence..).map_err(adapter_error)? {
            if events.len() >= limit {
                break;
            }
            let (_, value) = entry.map_err(adapter_error)?;
            events.push(serde_json::from_slice(value.value()).map_err(adapter_error)?);
        }
        Ok(events)
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(OUTBOX).map_err(adapter_error)?;
        let mut work = Vec::with_capacity(limit.min(1024));
        for entry in table.range(from_sequence..).map_err(adapter_error)? {
            if work.len() >= limit {
                break;
            }
            let (sequence, value) = entry.map_err(adapter_error)?;
            let record: Value = serde_json::from_slice(value.value()).map_err(adapter_error)?;
            let event_id = record
                .get("event_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "projection outbox record {} has no event_id",
                        sequence.value()
                    ))
                })?;
            if record.get("global_sequence").and_then(Value::as_u64) != Some(sequence.value()) {
                return Err(StoreError::Verification(format!(
                    "projection outbox record {} has a mismatched sequence",
                    sequence.value()
                )));
            }
            work.push(ProjectionWorkItem {
                global_sequence: sequence.value(),
                event_id: event_id.to_owned(),
            });
        }
        Ok(work)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| serde_json::from_slice(value.value()).map_err(adapter_error),
            )?;
        Ok((sequence, hash))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        serde_json::from_slice(&self.decrypt(event)?).map_err(adapter_error)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.verify_inner()
    }

    fn is_recovery_mode(&self) -> bool {
        self.recovery_mode.load(Ordering::Acquire)
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        if sequence == 0 {
            return Ok(None);
        }
        let hash: String = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()?
            .ok_or_else(|| StoreError::Verification("journal head hash is absent".into()))?;
        drop(metadata);
        drop(read);
        let signature = self.signer.sign(&checkpoint_message(sequence, &hash))?;
        let checkpoint = SignedCheckpoint {
            global_sequence: sequence,
            record_hash: hash.clone(),
            key_id: self.signer.key_id().to_owned(),
            algorithm: "Ed25519".into(),
            signature: hex::encode(signature),
            created_at: utc_now()?,
        };
        let bytes = serde_json::to_vec(&checkpoint).map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            metadata
                .insert("latest_checkpoint", bytes.as_slice())
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)?;
        self.keys.store_anchor(sequence, &hash)?;
        *self.last_checkpoint.lock().map_err(adapter_error)? = Instant::now();
        Ok(Some(checkpoint))
    }
}

impl ProjectionStore for RedbEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        projection_prefix(projection)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        Ok(table
            .get(projection)
            .map_err(adapter_error)?
            .map_or(0, |position| position.value()))
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        let namespaced = projection_record_key(projection, key)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        table
            .get(namespaced.as_str())
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        let namespace = projection_prefix(projection)?;
        if key_prefix.contains('\0') {
            return Err(StoreError::Adapter(
                "projection key prefix may not contain NUL".into(),
            ));
        }
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        let mut records = Vec::with_capacity(limit.min(1024));
        for entry in table.iter().map_err(adapter_error)? {
            if records.len() >= limit {
                break;
            }
            let (stored_key, value) = entry.map_err(adapter_error)?;
            let Some(key) = stored_key.value().strip_prefix(&namespace) else {
                continue;
            };
            if !key.starts_with(key_prefix) {
                continue;
            }
            records.push((
                key.to_owned(),
                serde_json::from_slice(value.value()).map_err(adapter_error)?,
            ));
        }
        Ok(records)
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        projection_prefix(&batch.projection)?;
        if batch.through_sequence <= batch.expected_position {
            return Err(StoreError::Adapter(
                "projection position must advance".into(),
            ));
        }
        let encoded = batch
            .mutations
            .into_iter()
            .map(|mutation| match mutation {
                ProjectionMutation::Upsert { key, value } => Ok((
                    projection_record_key(&batch.projection, &key)?,
                    Some(serde_json::to_vec(&value).map_err(adapter_error)?),
                )),
                ProjectionMutation::Delete { key } => {
                    Ok((projection_record_key(&batch.projection, &key)?, None))
                }
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            let actual = positions
                .get(batch.projection.as_str())
                .map_err(adapter_error)?
                .map_or(0, |position| position.value());
            if actual != batch.expected_position {
                return Err(StoreError::Conflict {
                    stream_id: format!("projection:{}", batch.projection),
                    expected: batch.expected_position,
                    actual,
                });
            }
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            for (key, value) in &encoded {
                if let Some(value) = value {
                    records
                        .insert(key.as_str(), value.as_slice())
                        .map_err(adapter_error)?;
                } else {
                    records.remove(key.as_str()).map_err(adapter_error)?;
                }
            }
            positions
                .insert(batch.projection.as_str(), batch.through_sequence)
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        let namespace = projection_prefix(projection)?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            let mut keys = Vec::new();
            for entry in records.iter().map_err(adapter_error)? {
                let (key, _) = entry.map_err(adapter_error)?;
                if key.value().starts_with(&namespace) {
                    keys.push(key.value().to_owned());
                }
            }
            for key in keys {
                records.remove(key.as_str()).map_err(adapter_error)?;
            }
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            positions.remove(projection).map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EVENTS, Ed25519CheckpointSigner, METADATA, OUTBOX, PROJECTION_POSITIONS, RedbEventJournal,
        RedbWriterLease, STREAM_VERSIONS, StaticKeyProvider, adapter_error,
    };
    use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
    use colossus_ports::{EventJournal, ProjectionStore, StoreError};
    use colossus_projection::{ProjectionWorker, default_handlers};
    use colossus_testkit::{assert_journal_conformance, assert_projection_store_conformance};
    use redb::{Database, ReadableDatabase};
    use serde_json::json;
    use std::{sync::Arc, thread};
    use tempfile::tempdir;

    fn event(stream: &str, version: u64, value: u64) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: stream.into(),
            expected_stream_version: version,
            classification: EventClassification::Domain,
            event_type: "test.recorded.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext {
                correlation_id: "correlation".into(),
                ..ExecutionContext::default()
            },
            payload: json!({"value": value}),
        }
    }

    fn journal(path: &std::path::Path) -> RedbEventJournal {
        journal_with_keys(
            path,
            Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
        )
    }

    fn journal_with_keys(path: &std::path::Path, keys: Arc<StaticKeyProvider>) -> RedbEventJournal {
        RedbEventJournal::open(
            path,
            keys,
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
        )
        .expect("open journal")
    }

    #[test]
    fn encrypted_append_round_trip_and_concurrency() {
        let directory = tempdir().expect("tempdir");
        let journal = journal(&directory.path().join("state.redb"));
        let stored = journal.append(event("stream-1", 0, 42)).expect("append");
        assert_ne!(stored.payload.ciphertext, hex::encode(br#"{"value":42}"#));
        assert_eq!(
            journal.decrypt_payload(&stored).expect("decrypt"),
            json!({"value": 42})
        );
        let conflict = journal.append(event("stream-1", 0, 43));
        assert!(matches!(
            conflict,
            Err(StoreError::Conflict { actual: 1, .. })
        ));
        let report = journal.verify().expect("verify");
        assert_eq!(report.event_count, 1);
    }

    #[test]
    fn shared_journal_conformance_suite_passes() {
        let directory = tempdir().expect("tempdir");
        let journal = journal(&directory.path().join("state.redb"));
        assert_journal_conformance(
            &journal,
            event("conformance", 0, 1),
            event("conformance", 0, 2),
        );
    }

    #[test]
    fn shared_projection_store_conformance_suite_passes() {
        let directory = tempdir().expect("tempdir");
        let journal = journal(&directory.path().join("state.redb"));
        assert_projection_store_conformance(&journal);
    }

    #[test]
    fn writer_lease_is_exclusive_and_reacquirable() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("state.redb");
        let first = RedbWriterLease::acquire(&path).expect("first lease");
        assert!(RedbWriterLease::acquire(&path).is_err());
        assert!(first.path().ends_with("state.redb.writer.lock"));
        drop(first);
        RedbWriterLease::acquire(&path).expect("reacquired lease");
    }

    #[test]
    fn projection_worker_catches_up_after_journal_only_restart() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("state.redb");
        let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
        {
            let journal = journal_with_keys(&path, Arc::clone(&keys));
            journal
                .append(NewEvent {
                    event_type: "session.created.v1".into(),
                    stream_id: "session:restarted".into(),
                    payload: json!({"title": "Recovered"}),
                    ..event("unused", 0, 1)
                })
                .expect("journal append before crash");
        }
        let journal = Arc::new(journal_with_keys(&path, keys));
        let journal_port: Arc<dyn EventJournal> = journal.clone();
        let store_port: Arc<dyn ProjectionStore> = journal.clone();
        let worker =
            ProjectionWorker::new(journal_port, store_port, default_handlers()).expect("worker");
        assert!(
            worker
                .status()
                .expect("lag")
                .iter()
                .all(|item| item.lag == 1)
        );
        worker.drain(16, 16).expect("catch up");
        assert_eq!(
            journal
                .get("sessions-v1", "restarted")
                .expect("record")
                .expect("session")["title"],
            json!("Recovered")
        );
    }

    #[test]
    fn concurrent_appends_are_serialized_without_lost_events() {
        let directory = tempdir().expect("tempdir");
        let journal = Arc::new(journal(&directory.path().join("state.redb")));
        let handles = (0_u64..8)
            .map(|index| {
                let journal = Arc::clone(&journal);
                thread::spawn(move || {
                    journal
                        .append(event(&format!("stream-{index}"), 0, index))
                        .expect("concurrent append")
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("thread");
        }
        assert_eq!(journal.verify().expect("verify").event_count, 8);
    }

    #[test]
    fn historical_keys_remain_usable_after_rotation() {
        let directory = tempdir().expect("tempdir");
        let keys = Arc::new(StaticKeyProvider::new("key-v1", [1_u8; 32]));
        let journal = journal_with_keys(&directory.path().join("state.redb"), Arc::clone(&keys));
        let first = journal.append(event("stream", 0, 1)).expect("first");
        keys.rotate("key-v2", [2_u8; 32]).expect("rotate");
        let second = journal.append(event("stream", 1, 2)).expect("second");
        assert_eq!(first.payload.key_id, "key-v1");
        assert_eq!(second.payload.key_id, "key-v2");
        assert_eq!(
            journal.decrypt_payload(&first).expect("old key"),
            json!({"value": 1})
        );
        assert_eq!(
            journal.decrypt_payload(&second).expect("new key"),
            json!({"value": 2})
        );
        journal.verify().expect("rotation verification");
    }

    #[test]
    fn signed_checkpoint_and_secure_anchor_verify() {
        let directory = tempdir().expect("tempdir");
        let journal = journal(&directory.path().join("state.redb"));
        journal.append(event("stream-1", 0, 1)).expect("append");
        let checkpoint = journal
            .checkpoint()
            .expect("checkpoint")
            .expect("nonempty checkpoint");
        assert_eq!(checkpoint.global_sequence, 1);
        assert_eq!(
            journal.verify().expect("verify").checkpoint,
            Some(checkpoint)
        );
    }

    #[test]
    fn tampering_enters_recovery_mode_on_reopen() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("state.redb");
        {
            let journal = journal(&path);
            journal.append(event("stream-1", 0, 1)).expect("append");
            journal.checkpoint().expect("checkpoint");
        }
        let database = Database::create(&path).expect("database");
        let read = database.begin_read().expect("read");
        let table = read.open_table(EVENTS).expect("events");
        let bytes = table.get(1).expect("get").expect("event").value().to_vec();
        drop(table);
        drop(read);
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        value["event_type"] = json!("tampered.v1");
        let bytes = serde_json::to_vec(&value).expect("encode");
        let write = database.begin_write().expect("write");
        {
            let mut table = write.open_table(EVENTS).expect("events");
            table
                .insert(1, bytes.as_slice())
                .map_err(adapter_error)
                .expect("insert");
        }
        write.commit().expect("commit");
        drop(database);

        let reopened = journal(&path);
        assert!(reopened.is_recovery_mode());
        assert!(matches!(
            reopened.append(event("stream-1", 1, 2)),
            Err(StoreError::RecoveryMode)
        ));
    }

    #[test]
    fn secure_anchor_detects_consistent_tail_truncation() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("state.redb");
        let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
        let first_hash;
        {
            let journal = journal_with_keys(&path, Arc::clone(&keys));
            first_hash = journal
                .append(event("stream-1", 0, 1))
                .expect("first")
                .record_hash;
            journal.append(event("stream-1", 1, 2)).expect("second");
            journal.checkpoint().expect("checkpoint");
        }
        let database = Database::create(&path).expect("database");
        let write = database.begin_write().expect("write");
        {
            let mut events = write.open_table(EVENTS).expect("events");
            events.remove(2).expect("truncate event");
            let mut streams = write.open_table(STREAM_VERSIONS).expect("streams");
            streams.insert("stream-1", 1).expect("rewind stream");
            let mut outbox = write.open_table(OUTBOX).expect("outbox");
            outbox.remove(2).expect("truncate outbox");
            let mut metadata = write.open_table(METADATA).expect("metadata");
            let one = serde_json::to_vec(&1_u64).expect("sequence");
            let hash = serde_json::to_vec(&first_hash).expect("hash");
            metadata
                .insert("last_sequence", one.as_slice())
                .expect("rewind sequence");
            metadata
                .insert("last_hash", hash.as_slice())
                .expect("rewind hash");
            metadata
                .remove("latest_checkpoint")
                .expect("remove checkpoint");
        }
        write.commit().expect("commit truncation");
        drop(database);

        let reopened = journal_with_keys(&path, keys);
        assert!(reopened.is_recovery_mode());
        assert!(
            reopened
                .recovery_reason()
                .expect("reason")
                .expect("recovery reason")
                .contains("secure anchor")
        );
    }

    #[test]
    fn projection_position_ahead_of_journal_enters_recovery_mode() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("state.redb");
        let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
        {
            let journal = journal_with_keys(&path, Arc::clone(&keys));
            journal.append(event("stream", 0, 1)).expect("append");
        }
        let database = Database::create(&path).expect("database");
        let write = database.begin_write().expect("write");
        {
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .expect("projection positions");
            positions
                .insert("sessions-v1", 2)
                .expect("corrupt position");
        }
        write.commit().expect("commit corruption");
        drop(database);

        let reopened = journal_with_keys(&path, keys);
        assert!(reopened.is_recovery_mode());
        assert!(
            reopened
                .recovery_reason()
                .expect("reason")
                .expect("recovery reason")
                .contains("ahead of journal head")
        );
    }
}
