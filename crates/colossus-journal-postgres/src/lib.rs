//! Encrypted, hash-chained PostgreSQL event journal and projection store.

#![allow(clippy::missing_errors_doc)]

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
use postgres::{
    Client, Config as PgConfig, NoTls,
    config::{Host, SslMode},
};
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, pem::PemObject},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio_postgres_rustls::MakeRustlsConnect;
use uuid::Uuid;

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const CHECKPOINT_INTERVAL: u64 = 100;
const CHECKPOINT_MAX_AGE: Duration = Duration::from_secs(60);
const DEFAULT_STATEMENT_TIMEOUT_MS: u64 = 30_000;

const TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS journal_metadata (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    last_sequence BIGINT NOT NULL CHECK (last_sequence >= 0),
    last_hash TEXT NOT NULL,
    latest_checkpoint BYTEA NULL
);
INSERT INTO journal_metadata (singleton, last_sequence, last_hash)
VALUES (TRUE, 0, '0000000000000000000000000000000000000000000000000000000000000000')
ON CONFLICT (singleton) DO NOTHING;

CREATE TABLE IF NOT EXISTS journal_events (
    global_sequence BIGINT PRIMARY KEY CHECK (global_sequence > 0),
    event_id TEXT NOT NULL UNIQUE,
    stream_id TEXT NOT NULL,
    stream_version BIGINT NOT NULL CHECK (stream_version > 0),
    envelope BYTEA NOT NULL,
    UNIQUE (stream_id, stream_version)
);
CREATE INDEX IF NOT EXISTS journal_events_stream_idx
ON journal_events (stream_id, stream_version);

CREATE TABLE IF NOT EXISTS journal_stream_versions (
    stream_id TEXT PRIMARY KEY,
    stream_version BIGINT NOT NULL CHECK (stream_version > 0)
);

CREATE TABLE IF NOT EXISTS projection_outbox (
    global_sequence BIGINT PRIMARY KEY REFERENCES journal_events(global_sequence),
    event_id TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS projection_positions (
    projection TEXT PRIMARY KEY,
    position BIGINT NOT NULL CHECK (position >= 0)
);

CREATE TABLE IF NOT EXISTS projection_records (
    projection TEXT NOT NULL,
    record_key TEXT NOT NULL,
    value BYTEA NOT NULL,
    PRIMARY KEY (projection, record_key)
);
"#;

fn adapter_error(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn database_error(error: postgres::Error) -> StoreError {
    if let Some(db) = error.as_db_error() {
        StoreError::Adapter(format!(
            "PostgreSQL rejected operation ({})",
            db.code().code()
        ))
    } else {
        StoreError::Adapter("PostgreSQL is unavailable".into())
    }
}

fn commit_error(_error: postgres::Error) -> StoreError {
    StoreError::OutcomeUnknown("PostgreSQL commit outcome is unknown".into())
}

fn utc_now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(adapter_error)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn to_i64(value: u64, label: &str) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Adapter(format!("{label} exceeds PostgreSQL BIGINT")))
}

fn to_u64(value: i64, label: &str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Verification(format!("{label} is negative")))
}

fn bounded_limit(limit: usize) -> Result<i64, StoreError> {
    Ok(i64::try_from(limit).unwrap_or(i64::MAX))
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && value.len() <= 63
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn validate_projection(projection: &str) -> Result<(), StoreError> {
    if projection.is_empty() || projection.contains('\0') {
        return Err(StoreError::Adapter(
            "projection name must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(())
}

fn validate_record_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty() || key.contains('\0') {
        return Err(StoreError::Adapter(
            "projection key must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(())
}

/// PostgreSQL TLS policy. Disabling TLS is intended only for isolated local acceptance.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PostgresTlsConfig {
    /// Require TLS with the pinned Mozilla WebPKI root set.
    #[default]
    WebpkiRoots,
    /// Require TLS and trust only the certificates in one PEM CA bundle.
    CustomCa {
        /// PEM CA-bundle path read only by the adapter.
        ca_pem_path: PathBuf,
    },
    /// Disable TLS explicitly for isolated loopback development and CI.
    Disabled,
}

/// Credential-reference-only PostgreSQL journal configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostgresJournalConfig {
    /// Environment variable containing a libpq-style URL or key/value connection string.
    pub connection_variable: String,
    /// Dedicated PostgreSQL schema owned by this Colossus instance.
    pub schema: String,
    /// TLS verification policy.
    #[serde(default)]
    pub tls: PostgresTlsConfig,
    /// Per-connection statement and lock timeout.
    #[serde(default = "default_statement_timeout_ms")]
    pub statement_timeout_ms: u64,
}

const fn default_statement_timeout_ms() -> u64 {
    DEFAULT_STATEMENT_TIMEOUT_MS
}

impl PostgresJournalConfig {
    /// Construct and validate a PostgreSQL adapter configuration.
    pub fn new(
        connection_variable: impl Into<String>,
        schema: impl Into<String>,
        tls: PostgresTlsConfig,
    ) -> Result<Self, StoreError> {
        let config = Self {
            connection_variable: connection_variable.into(),
            schema: schema.into(),
            tls,
            statement_timeout_ms: DEFAULT_STATEMENT_TIMEOUT_MS,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate identifiers and bounded timeout values without resolving credentials.
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.connection_variable.is_empty()
            || !self
                .connection_variable
                .bytes()
                .enumerate()
                .all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                })
        {
            return Err(StoreError::Adapter(
                "PostgreSQL connection variable must be a POSIX-style environment name".into(),
            ));
        }
        if !valid_identifier(&self.schema) {
            return Err(StoreError::Adapter(
                "PostgreSQL schema must be a 1-63 byte ASCII identifier".into(),
            ));
        }
        if !(100..=300_000).contains(&self.statement_timeout_ms) {
            return Err(StoreError::Adapter(
                "PostgreSQL statement timeout must be between 100 and 300000 ms".into(),
            ));
        }
        Ok(())
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

// Preserve the exact nested JSON used for historical authenticated data and hashes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEventEnvelope {
    schema_version: u16,
    event_version: u16,
    event_id: String,
    global_sequence: u64,
    stream_id: String,
    stream_version: u64,
    classification: colossus_contracts::EventClassification,
    event_type: String,
    actor: Box<RawValue>,
    context: Box<RawValue>,
    occurred_at: String,
    payload: Box<RawValue>,
    previous_hash: String,
    record_hash: String,
}

#[derive(Serialize)]
struct PersistedAssociatedData<'a> {
    schema_version: u16,
    event_version: u16,
    event_id: &'a str,
    global_sequence: u64,
    stream_id: &'a str,
    stream_version: u64,
    classification: &'a colossus_contracts::EventClassification,
    event_type: &'a str,
    actor: &'a RawValue,
    context: &'a RawValue,
    occurred_at: &'a str,
}

#[derive(Serialize)]
struct PersistedRecordHashInput<'a> {
    associated_data: PersistedAssociatedData<'a>,
    payload: &'a RawValue,
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

fn persisted_associated_data(envelope: &PersistedEventEnvelope) -> PersistedAssociatedData<'_> {
    PersistedAssociatedData {
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
    Ok(sha256_hex(
        &serde_json::to_vec(&RecordHashInput {
            associated_data: associated_data(envelope),
            payload: &envelope.payload,
            previous_hash: &envelope.previous_hash,
        })
        .map_err(adapter_error)?,
    ))
}

fn persisted_record_hash(envelope: &PersistedEventEnvelope) -> Result<String, StoreError> {
    Ok(sha256_hex(
        &serde_json::to_vec(&PersistedRecordHashInput {
            associated_data: persisted_associated_data(envelope),
            payload: &envelope.payload,
            previous_hash: &envelope.previous_hash,
        })
        .map_err(adapter_error)?,
    ))
}

fn checkpoint_message(sequence: u64, hash: &str) -> Vec<u8> {
    format!("colossus-checkpoint-v1\n{sequence}\n{hash}\n").into_bytes()
}

#[cfg(test)]
fn crash_at_test_fault(point: &str) {
    if std::env::var("COLOSSUS_POSTGRES_TEST_CRASH_POINT").as_deref() == Ok(point) {
        std::process::abort();
    }
}

/// Canonical PostgreSQL journal adapter.
pub struct PostgresEventJournal {
    config: PostgresJournalConfig,
    tls_connector: Option<MakeRustlsConnect>,
    keys: Arc<dyn KeyProvider>,
    signer: Arc<dyn CheckpointSigner>,
    last_checkpoint: Mutex<Instant>,
    recovery_mode: AtomicBool,
    recovery_reason: Mutex<Option<String>>,
}

impl PostgresEventJournal {
    /// Connect, create the isolated schema, migrate idempotently, and verify before writes.
    pub fn open(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        config.validate()?;
        let tls_connector = match &config.tls {
            PostgresTlsConfig::Disabled => None,
            tls => Some(Self::build_tls_connector(tls)?),
        };
        let journal = Self {
            config,
            tls_connector,
            keys,
            signer,
            last_checkpoint: Mutex::new(Instant::now()),
            recovery_mode: AtomicBool::new(false),
            recovery_reason: Mutex::new(None),
        };
        journal.migrate()?;
        let startup = journal.verify_inner().and_then(|report| {
            let checkpoint_sequence = report
                .checkpoint
                .as_ref()
                .map_or(0, |checkpoint| checkpoint.global_sequence);
            if report.last_sequence.saturating_sub(checkpoint_sequence) >= CHECKPOINT_INTERVAL {
                journal.checkpoint()?;
            }
            Ok(())
        });
        if let Err(error) = startup {
            journal.recovery_mode.store(true, Ordering::Release);
            *journal.recovery_reason.lock().map_err(adapter_error)? = Some(error.to_string());
        }
        Ok(journal)
    }

    /// Bounded reason startup entered read-only recovery mode.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        Ok(self.recovery_reason.lock().map_err(adapter_error)?.clone())
    }

    /// Stable adapter diagnostic without a connection string or credential value.
    #[must_use]
    pub fn diagnostic(&self) -> Value {
        serde_json::json!({
            "adapter": "postgresql",
            "connection_variable": self.config.connection_variable,
            "schema": self.config.schema,
            "tls": match &self.config.tls {
                PostgresTlsConfig::WebpkiRoots => "webpki_roots",
                PostgresTlsConfig::CustomCa { .. } => "custom_ca",
                PostgresTlsConfig::Disabled => "disabled",
            },
            "statement_timeout_ms": self.config.statement_timeout_ms,
        })
    }

    fn pg_config(&self) -> Result<PgConfig, StoreError> {
        let value = std::env::var(&self.config.connection_variable).map_err(|_| {
            StoreError::KeyUnavailable(format!(
                "PostgreSQL connection variable {} is unset",
                self.config.connection_variable
            ))
        })?;
        let mut config = PgConfig::from_str(&value).map_err(|_| {
            StoreError::Adapter(format!(
                "PostgreSQL connection variable {} is invalid",
                self.config.connection_variable
            ))
        })?;
        config.application_name("colossus-journal");
        match &self.config.tls {
            PostgresTlsConfig::Disabled => {
                config.ssl_mode(SslMode::Disable);
                let is_local = config.get_hosts().iter().all(|host| match host {
                    Host::Tcp(host) => {
                        host.eq_ignore_ascii_case("localhost")
                            || host
                                .parse::<std::net::IpAddr>()
                                .is_ok_and(|address| address.is_loopback())
                    }
                    #[cfg(unix)]
                    Host::Unix(_) => true,
                });
                if !is_local {
                    return Err(StoreError::Adapter(
                        "disabling PostgreSQL TLS is restricted to loopback or Unix-socket connections"
                            .into(),
                    ));
                }
            }
            PostgresTlsConfig::WebpkiRoots | PostgresTlsConfig::CustomCa { .. } => {
                config.ssl_mode(SslMode::Require);
            }
        }
        Ok(config)
    }

    fn connect(&self) -> Result<Client, StoreError> {
        let config = self.pg_config()?;
        let mut client = match &self.config.tls {
            PostgresTlsConfig::Disabled => config.connect(NoTls).map_err(database_error)?,
            PostgresTlsConfig::WebpkiRoots | PostgresTlsConfig::CustomCa { .. } => config
                .connect(self.required_tls_connector()?)
                .map_err(database_error)?,
        };
        let timeout = self.config.statement_timeout_ms.to_string();
        client
            .execute(
                "SELECT set_config('statement_timeout', $1, false), set_config('lock_timeout', $1, false)",
                &[&timeout],
            )
            .map_err(database_error)?;
        client
            .batch_execute(&format!("SET search_path TO \"{}\"", self.config.schema))
            .map_err(database_error)?;
        Ok(client)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let mut client = self.pg_config().and_then(|config| match &self.config.tls {
            PostgresTlsConfig::Disabled => config.connect(NoTls).map_err(database_error),
            PostgresTlsConfig::WebpkiRoots | PostgresTlsConfig::CustomCa { .. } => config
                .connect(self.required_tls_connector()?)
                .map_err(database_error),
        })?;
        client
            .batch_execute(&format!(
                "CREATE SCHEMA IF NOT EXISTS \"{}\"; SET search_path TO \"{}\"; {TABLES}",
                self.config.schema, self.config.schema
            ))
            .map_err(database_error)
    }

    fn required_tls_connector(&self) -> Result<MakeRustlsConnect, StoreError> {
        self.tls_connector
            .clone()
            .ok_or_else(|| StoreError::Adapter("PostgreSQL TLS connector is not configured".into()))
    }

    fn build_tls_connector(tls: &PostgresTlsConfig) -> Result<MakeRustlsConnect, StoreError> {
        let roots = match tls {
            PostgresTlsConfig::WebpkiRoots => RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            },
            PostgresTlsConfig::CustomCa { ca_pem_path } => {
                let bytes = fs::read(ca_pem_path).map_err(|_| {
                    StoreError::Adapter("PostgreSQL CA bundle is unreadable".into())
                })?;
                let mut roots = RootCertStore::empty();
                let mut count = 0_u64;
                for certificate in CertificateDer::pem_slice_iter(&bytes) {
                    roots
                        .add(certificate.map_err(|_| {
                            StoreError::Adapter("PostgreSQL CA bundle is invalid".into())
                        })?)
                        .map_err(|_| {
                            StoreError::Adapter("PostgreSQL CA bundle is invalid".into())
                        })?;
                    count += 1;
                }
                if count == 0 {
                    return Err(StoreError::Adapter(
                        "PostgreSQL CA bundle contains no certificates".into(),
                    ));
                }
                roots
            }
            PostgresTlsConfig::Disabled => {
                return Err(StoreError::Adapter(
                    "cannot build a PostgreSQL TLS connector while TLS is disabled".into(),
                ));
            }
        };
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| {
                    StoreError::Adapter("PostgreSQL TLS protocol configuration failed".into())
                })?
                .with_root_certificates(roots)
                .with_no_client_auth();
        Ok(MakeRustlsConnect::new(config))
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
        let ciphertext = XChaCha20Poly1305::new((&key).into())
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

    fn decrypt_persisted(
        &self,
        event: &EventEnvelope,
        persisted: &PersistedEventEnvelope,
    ) -> Result<Vec<u8>, StoreError> {
        if event.payload.algorithm != "XChaCha20-Poly1305" {
            return Err(StoreError::Verification(format!(
                "unsupported payload algorithm {}",
                event.payload.algorithm
            )));
        }
        let key = self.keys.key_by_id(&event.payload.key_id)?;
        let nonce: [u8; 24] = hex::decode(&event.payload.nonce)
            .map_err(adapter_error)?
            .try_into()
            .map_err(|_| StoreError::Verification("invalid XChaCha20 nonce length".into()))?;
        let ciphertext = hex::decode(&event.payload.ciphertext).map_err(adapter_error)?;
        let aad =
            serde_json::to_vec(&persisted_associated_data(persisted)).map_err(adapter_error)?;
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

    fn append_locked(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        let head = transaction
            .query_one(
                "SELECT last_sequence, last_hash FROM journal_metadata WHERE singleton = TRUE FOR UPDATE",
                &[],
            )
            .map_err(database_error)?;
        let mut sequence = to_u64(head.get::<_, i64>(0), "journal head")?;
        let mut previous_hash = head.get::<_, String>(1);
        let mut batch_versions = BTreeMap::<String, u64>::new();
        let mut persisted = Vec::with_capacity(events.len());

        for event in events {
            let durable_version = if let Some(version) = batch_versions.get(&event.stream_id) {
                *version
            } else {
                transaction
                    .query_opt(
                        "SELECT stream_version FROM journal_stream_versions WHERE stream_id = $1",
                        &[&event.stream_id],
                    )
                    .map_err(database_error)?
                    .map_or(Ok(0), |row| to_u64(row.get::<_, i64>(0), "stream version"))?
            };
            if event.expected_stream_version != durable_version {
                return Err(StoreError::Conflict {
                    stream_id: event.stream_id,
                    expected: event.expected_stream_version,
                    actual: durable_version,
                });
            }
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| StoreError::Adapter("global sequence overflow".into()))?;
            let stream_version = durable_version
                .checked_add(1)
                .ok_or_else(|| StoreError::Adapter("stream version overflow".into()))?;
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
            transaction
                .execute(
                    "INSERT INTO journal_events (global_sequence, event_id, stream_id, stream_version, envelope) VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &to_i64(sequence, "global sequence")?,
                        &envelope.event_id,
                        &envelope.stream_id,
                        &to_i64(stream_version, "stream version")?,
                        &encoded,
                    ],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO journal_stream_versions (stream_id, stream_version) VALUES ($1, $2) ON CONFLICT (stream_id) DO UPDATE SET stream_version = EXCLUDED.stream_version",
                    &[&envelope.stream_id, &to_i64(stream_version, "stream version")?],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO projection_outbox (global_sequence, event_id) VALUES ($1, $2)",
                    &[&to_i64(sequence, "global sequence")?, &envelope.event_id],
                )
                .map_err(database_error)?;
            batch_versions.insert(envelope.stream_id.clone(), stream_version);
            persisted.push(envelope);
        }
        transaction
            .execute(
                "UPDATE journal_metadata SET last_sequence = $1, last_hash = $2 WHERE singleton = TRUE",
                &[&to_i64(sequence, "global sequence")?, &previous_hash],
            )
            .map_err(database_error)?;
        #[cfg(test)]
        crash_at_test_fault("before_commit");
        transaction.commit().map_err(commit_error)?;
        #[cfg(test)]
        crash_at_test_fault("after_commit");
        Ok(persisted)
    }

    fn load_persisted(&self, event: &EventEnvelope) -> Result<PersistedEventEnvelope, StoreError> {
        let mut client = self.connect()?;
        let bytes = client
            .query_opt(
                "SELECT envelope FROM journal_events WHERE global_sequence = $1",
                &[&to_i64(event.global_sequence, "global sequence")?],
            )
            .map_err(database_error)?
            .ok_or_else(|| {
                StoreError::Verification(format!(
                    "event {} is absent from the journal",
                    event.event_id
                ))
            })?
            .get::<_, Vec<u8>>(0);
        let stored: EventEnvelope = serde_json::from_slice(&bytes).map_err(adapter_error)?;
        if stored != *event {
            return Err(StoreError::Verification(format!(
                "event {} does not match its persisted envelope",
                event.event_id
            )));
        }
        serde_json::from_slice(&bytes).map_err(adapter_error)
    }

    fn checkpoint_sequence(&self) -> Result<u64, StoreError> {
        let mut client = self.connect()?;
        client
            .query_one(
                "SELECT latest_checkpoint FROM journal_metadata WHERE singleton = TRUE",
                &[],
            )
            .map_err(database_error)?
            .get::<_, Option<Vec<u8>>>(0)
            .map(|bytes| {
                serde_json::from_slice::<SignedCheckpoint>(&bytes)
                    .map(|checkpoint| checkpoint.global_sequence)
                    .map_err(adapter_error)
            })
            .transpose()
            .map(|sequence| sequence.unwrap_or(0))
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
        self.signer.verify(
            &checkpoint_message(checkpoint.global_sequence, &checkpoint.record_hash),
            &hex::decode(&checkpoint.signature).map_err(adapter_error)?,
        )
    }

    fn verify_inner(&self) -> Result<VerificationReport, StoreError> {
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        let metadata = transaction
            .query_one(
                "SELECT last_sequence, last_hash, latest_checkpoint FROM journal_metadata WHERE singleton = TRUE FOR SHARE",
                &[],
            )
            .map_err(database_error)?;
        let metadata_sequence = to_u64(metadata.get::<_, i64>(0), "journal head")?;
        let metadata_hash = metadata.get::<_, String>(1);
        let checkpoint_bytes = metadata.get::<_, Option<Vec<u8>>>(2);
        let mut expected_sequence = 1_u64;
        let mut previous_hash = ZERO_HASH.to_owned();
        let mut stream_versions = BTreeMap::<String, u64>::new();
        let mut event_hashes = BTreeMap::<u64, String>::new();

        for row in transaction
            .query(
                "SELECT global_sequence, envelope FROM journal_events ORDER BY global_sequence",
                &[],
            )
            .map_err(database_error)?
        {
            let sequence = to_u64(row.get::<_, i64>(0), "global sequence")?;
            if sequence != expected_sequence {
                return Err(StoreError::Verification(format!(
                    "global sequence gap: expected {expected_sequence}, got {sequence}"
                )));
            }
            let bytes = row.get::<_, Vec<u8>>(1);
            let persisted: PersistedEventEnvelope =
                serde_json::from_slice(&bytes).map_err(adapter_error)?;
            let envelope: EventEnvelope = serde_json::from_slice(&bytes).map_err(adapter_error)?;
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
            if persisted_record_hash(&persisted)? != persisted.record_hash
                || envelope.record_hash != persisted.record_hash
            {
                return Err(StoreError::Verification(format!(
                    "event {} record hash mismatch",
                    envelope.event_id
                )));
            }
            let plaintext = self.decrypt_persisted(&envelope, &persisted)?;
            if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
                return Err(StoreError::Verification(format!(
                    "event {} plaintext hash mismatch",
                    envelope.event_id
                )));
            }
            serde_json::from_slice::<Value>(&plaintext).map_err(adapter_error)?;
            let queued = transaction
                .query_opt(
                    "SELECT event_id FROM projection_outbox WHERE global_sequence = $1",
                    &[&to_i64(sequence, "global sequence")?],
                )
                .map_err(database_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "projection outbox record {sequence} is absent"
                    ))
                })?;
            if queued.get::<_, String>(0) != envelope.event_id {
                return Err(StoreError::Verification(format!(
                    "projection outbox record {sequence} targets a different event"
                )));
            }
            previous_hash.clone_from(&envelope.record_hash);
            event_hashes.insert(sequence, envelope.record_hash);
            stream_versions.insert(envelope.stream_id, envelope.stream_version);
            expected_sequence = expected_sequence.saturating_add(1);
        }

        let last_sequence = expected_sequence.saturating_sub(1);
        if metadata_sequence != last_sequence || metadata_hash != previous_hash {
            return Err(StoreError::Verification(
                "journal head metadata does not match event chain".into(),
            ));
        }
        let outbox_count = to_u64(
            transaction
                .query_one("SELECT COUNT(*) FROM projection_outbox", &[])
                .map_err(database_error)?
                .get::<_, i64>(0),
            "projection outbox count",
        )?;
        if outbox_count != last_sequence {
            return Err(StoreError::Verification(
                "projection outbox position does not match journal head".into(),
            ));
        }
        let mut durable_stream_versions = BTreeMap::new();
        for row in transaction
            .query(
                "SELECT stream_id, stream_version FROM journal_stream_versions ORDER BY stream_id",
                &[],
            )
            .map_err(database_error)?
        {
            durable_stream_versions.insert(
                row.get::<_, String>(0),
                to_u64(row.get::<_, i64>(1), "stream version")?,
            );
        }
        if durable_stream_versions != stream_versions {
            return Err(StoreError::Verification(
                "durable stream versions do not match journal replay".into(),
            ));
        }
        for row in transaction
            .query("SELECT projection, position FROM projection_positions", &[])
            .map_err(database_error)?
        {
            let projection = row.get::<_, String>(0);
            validate_projection(&projection)?;
            let position = to_u64(row.get::<_, i64>(1), "projection position")?;
            if position > last_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {projection} position {position} is ahead of journal head {last_sequence}"
                )));
            }
        }
        for row in transaction
            .query(
                "SELECT projection, record_key, value FROM projection_records",
                &[],
            )
            .map_err(database_error)?
        {
            let projection = row.get::<_, String>(0);
            let key = row.get::<_, String>(1);
            validate_projection(&projection)?;
            validate_record_key(&key)?;
            serde_json::from_slice::<Value>(&row.get::<_, Vec<u8>>(2)).map_err(|error| {
                StoreError::Verification(format!(
                    "projection record {projection}/{key} is invalid JSON: {error}"
                ))
            })?;
        }
        let checkpoint = checkpoint_bytes
            .map(|bytes| serde_json::from_slice(&bytes).map_err(adapter_error))
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
        transaction.commit().map_err(database_error)?;
        Ok(VerificationReport {
            event_count: last_sequence,
            last_sequence,
            last_hash: previous_hash,
            checkpoint,
        })
    }
}

impl EventJournal for PostgresEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.append_batch(vec![event])?
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let persisted = self.append_locked(events)?;
        let checkpoint_sequence = if persisted.is_empty() {
            0
        } else {
            self.checkpoint_sequence().map_err(|_| {
                StoreError::OutcomeUnknown(
                    "event append committed but checkpoint status is unavailable".into(),
                )
            })?
        };
        let count_due = persisted.last().is_some_and(|event| {
            event.global_sequence.saturating_sub(checkpoint_sequence) >= CHECKPOINT_INTERVAL
        });
        let age_due = self
            .last_checkpoint
            .lock()
            .map_err(adapter_error)?
            .elapsed()
            >= CHECKPOINT_MAX_AGE;
        if count_due || age_due {
            self.checkpoint().map_err(|error| {
                if persisted.is_empty() {
                    error
                } else {
                    StoreError::OutcomeUnknown(
                        "event append committed but checkpoint advancement failed".into(),
                    )
                }
            })?;
        }
        Ok(persisted)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut client = self.connect()?;
        client
            .query(
                "SELECT envelope FROM journal_events WHERE stream_id = $1 ORDER BY stream_version",
                &[&stream_id],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .collect()
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut client = self.connect()?;
        client
            .query(
                "SELECT envelope FROM journal_events WHERE global_sequence >= $1 ORDER BY global_sequence LIMIT $2",
                &[
                    &to_i64(from_sequence, "global sequence")?,
                    &bounded_limit(limit)?,
                ],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .collect()
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        let mut client = self.connect()?;
        client
            .query(
                "SELECT global_sequence, event_id FROM projection_outbox WHERE global_sequence >= $1 ORDER BY global_sequence LIMIT $2",
                &[
                    &to_i64(from_sequence, "global sequence")?,
                    &bounded_limit(limit)?,
                ],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| {
                Ok(ProjectionWorkItem {
                    global_sequence: to_u64(row.get::<_, i64>(0), "global sequence")?,
                    event_id: row.get(1),
                })
            })
            .collect()
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        let mut client = self.connect()?;
        let row = client
            .query_one(
                "SELECT last_sequence, last_hash FROM journal_metadata WHERE singleton = TRUE",
                &[],
            )
            .map_err(database_error)?;
        Ok((to_u64(row.get::<_, i64>(0), "journal head")?, row.get(1)))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        let persisted = self.load_persisted(event)?;
        serde_json::from_slice(&self.decrypt_persisted(event, &persisted)?).map_err(adapter_error)
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
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        let row = transaction
            .query_one(
                "SELECT last_sequence, last_hash FROM journal_metadata WHERE singleton = TRUE FOR UPDATE",
                &[],
            )
            .map_err(database_error)?;
        let sequence = to_u64(row.get::<_, i64>(0), "journal head")?;
        if sequence == 0 {
            transaction.commit().map_err(database_error)?;
            return Ok(None);
        }
        let hash = row.get::<_, String>(1);
        let checkpoint = SignedCheckpoint {
            global_sequence: sequence,
            record_hash: hash.clone(),
            key_id: self.signer.key_id().to_owned(),
            algorithm: "Ed25519".into(),
            signature: hex::encode(self.signer.sign(&checkpoint_message(sequence, &hash))?),
            created_at: utc_now()?,
        };
        self.keys.store_anchor(sequence, &hash)?;
        #[cfg(test)]
        crash_at_test_fault("after_anchor_before_checkpoint_commit");
        transaction
            .execute(
                "UPDATE journal_metadata SET latest_checkpoint = $1 WHERE singleton = TRUE",
                &[&serde_json::to_vec(&checkpoint).map_err(adapter_error)?],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(commit_error)?;
        *self.last_checkpoint.lock().map_err(adapter_error)? = Instant::now();
        Ok(Some(checkpoint))
    }
}

impl ProjectionStore for PostgresEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        validate_projection(projection)?;
        let mut client = self.connect()?;
        client
            .query_opt(
                "SELECT position FROM projection_positions WHERE projection = $1",
                &[&projection],
            )
            .map_err(database_error)?
            .map_or(Ok(0), |row| {
                to_u64(row.get::<_, i64>(0), "projection position")
            })
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        validate_projection(projection)?;
        validate_record_key(key)?;
        let mut client = self.connect()?;
        client
            .query_opt(
                "SELECT value FROM projection_records WHERE projection = $1 AND record_key = $2",
                &[&projection, &key],
            )
            .map_err(database_error)?
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .transpose()
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        validate_projection(projection)?;
        if key_prefix.contains('\0') {
            return Err(StoreError::Adapter(
                "projection key prefix may not contain NUL".into(),
            ));
        }
        let mut client = self.connect()?;
        client
            .query(
                "SELECT record_key, value FROM projection_records WHERE projection = $1 AND left(record_key, char_length($2)) = $2 ORDER BY record_key LIMIT $3",
                &[&projection, &key_prefix, &bounded_limit(limit)?],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| {
                Ok((
                    row.get(0),
                    serde_json::from_slice(&row.get::<_, Vec<u8>>(1)).map_err(adapter_error)?,
                ))
            })
            .collect()
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        validate_projection(&batch.projection)?;
        if batch.through_sequence <= batch.expected_position {
            return Err(StoreError::Adapter(
                "projection position must advance".into(),
            ));
        }
        for mutation in &batch.mutations {
            match mutation {
                ProjectionMutation::Upsert { key, .. } | ProjectionMutation::Delete { key } => {
                    validate_record_key(key)?;
                }
            }
        }
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO projection_positions (projection, position) VALUES ($1, 0) ON CONFLICT (projection) DO NOTHING",
                &[&batch.projection],
            )
            .map_err(database_error)?;
        let actual = to_u64(
            transaction
                .query_one(
                    "SELECT position FROM projection_positions WHERE projection = $1 FOR UPDATE",
                    &[&batch.projection],
                )
                .map_err(database_error)?
                .get::<_, i64>(0),
            "projection position",
        )?;
        if actual != batch.expected_position {
            return Err(StoreError::Conflict {
                stream_id: format!("projection:{}", batch.projection),
                expected: batch.expected_position,
                actual,
            });
        }
        for mutation in batch.mutations {
            match mutation {
                ProjectionMutation::Upsert { key, value } => {
                    transaction
                        .execute(
                            "INSERT INTO projection_records (projection, record_key, value) VALUES ($1, $2, $3) ON CONFLICT (projection, record_key) DO UPDATE SET value = EXCLUDED.value",
                            &[
                                &batch.projection,
                                &key,
                                &serde_json::to_vec(&value).map_err(adapter_error)?,
                            ],
                        )
                        .map_err(database_error)?;
                }
                ProjectionMutation::Delete { key } => {
                    transaction
                        .execute(
                            "DELETE FROM projection_records WHERE projection = $1 AND record_key = $2",
                            &[&batch.projection, &key],
                        )
                        .map_err(database_error)?;
                }
            }
        }
        transaction
            .execute(
                "UPDATE projection_positions SET position = $2 WHERE projection = $1",
                &[
                    &batch.projection,
                    &to_i64(batch.through_sequence, "projection position")?,
                ],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(commit_error)
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        validate_projection(projection)?;
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM projection_records WHERE projection = $1",
                &[&projection],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM projection_positions WHERE projection = $1",
                &[&projection],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(commit_error)
    }
}

#[cfg(test)]
mod tests {
    use super::{PostgresEventJournal, PostgresJournalConfig, PostgresTlsConfig};
    use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
    use colossus_journal_redb::{Ed25519CheckpointSigner, StaticKeyProvider};
    use colossus_ports::{EventJournal, ProjectionStore, StoreError};
    use colossus_projection::JournalExternalWorkQueue;
    use colossus_session::EventSourcedSessionRepository;
    use colossus_testkit::{
        assert_external_work_queue_conformance, assert_session_repository_conformance,
        assert_work_repository_conformance, assert_workflow_repository_conformance,
    };
    use colossus_testkit::{assert_journal_conformance, assert_projection_store_conformance};
    use colossus_work::EventSourcedWorkRepository;
    use colossus_workflow::EventSourcedWorkflowRepository;
    use serde_json::json;
    use std::{
        fs,
        process::Command,
        sync::{Arc, Barrier},
    };
    use uuid::Uuid;

    fn event(stream: &str, version: u64, value: u64) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: stream.into(),
            expected_stream_version: version,
            classification: EventClassification::Domain,
            event_type: "test.recorded.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "postgres-test".into(),
            },
            context: ExecutionContext {
                correlation_id: "postgres-conformance".into(),
                ..ExecutionContext::default()
            },
            payload: json!({"value": value}),
        }
    }

    fn live_config() -> Option<PostgresJournalConfig> {
        std::env::var("COLOSSUS_TEST_POSTGRES_URL").ok()?;
        PostgresJournalConfig::new(
            "COLOSSUS_TEST_POSTGRES_URL",
            format!("colossus_test_{}", Uuid::now_v7().simple()),
            PostgresTlsConfig::Disabled,
        )
        .ok()
    }

    fn open(config: &PostgresJournalConfig) -> PostgresEventJournal {
        PostgresEventJournal::open(
            config.clone(),
            Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
        )
        .expect("open PostgreSQL journal")
    }

    fn with_schema(config: &PostgresJournalConfig, suffix: &str) -> PostgresJournalConfig {
        PostgresJournalConfig {
            schema: format!("{}_{suffix}", config.schema),
            ..config.clone()
        }
    }

    #[test]
    fn configuration_rejects_identifiers_and_does_not_echo_connection_values() {
        assert!(
            PostgresJournalConfig::new("bad-name", "valid", PostgresTlsConfig::Disabled).is_err()
        );
        assert!(
            PostgresJournalConfig::new("DATABASE_URL", "bad-name", PostgresTlsConfig::Disabled)
                .is_err()
        );
        let config = PostgresJournalConfig::new(
            "COLOSSUS_INTENTIONALLY_MISSING_DATABASE_URL",
            "valid_schema",
            PostgresTlsConfig::Disabled,
        )
        .expect("valid reference-only config");
        let error = match PostgresEventJournal::open(
            config,
            Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
        ) {
            Ok(_) => panic!("an unset connection variable must fail before opening"),
            Err(error) => error,
        };
        assert!(
            !error.to_string().contains("password"),
            "connection errors must not expose credential values"
        );
    }

    #[test]
    fn tls_policy_defaults_to_pinned_webpki_and_rejects_invalid_custom_bundles() {
        let config: PostgresJournalConfig = serde_json::from_value(json!({
            "connectionVariable": "DATABASE_URL",
            "schema": "colossus"
        }))
        .expect("default PostgreSQL TLS config");
        assert_eq!(config.tls, PostgresTlsConfig::WebpkiRoots);
        PostgresEventJournal::build_tls_connector(&config.tls)
            .expect("pinned WebPKI roots build a rustls connector");
        assert!(PostgresEventJournal::build_tls_connector(&PostgresTlsConfig::Disabled).is_err());

        let path = std::env::temp_dir().join(format!(
            "colossus-invalid-postgres-ca-{}.pem",
            Uuid::now_v7()
        ));
        fs::write(&path, b"not a PEM certificate").expect("write invalid CA bundle");
        let error = match PostgresEventJournal::build_tls_connector(&PostgresTlsConfig::CustomCa {
            ca_pem_path: path.clone(),
        }) {
            Ok(_) => panic!("an invalid private CA bundle must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "storage adapter failure: PostgreSQL CA bundle contains no certificates"
        );
        fs::remove_file(path).expect("remove invalid CA bundle");
    }

    #[test]
    fn live_crash_append_child() {
        let (Ok(schema), Ok(point)) = (
            std::env::var("COLOSSUS_POSTGRES_TEST_CRASH_SCHEMA"),
            std::env::var("COLOSSUS_POSTGRES_TEST_CRASH_POINT"),
        ) else {
            return;
        };
        let config = PostgresJournalConfig::new(
            "COLOSSUS_TEST_POSTGRES_URL",
            schema,
            PostgresTlsConfig::Disabled,
        )
        .expect("crash config");
        open(&config)
            .append(event("crash-stream", 0, 1))
            .expect("configured fault must terminate the process");
        panic!("PostgreSQL crash point {point} did not terminate the child");
    }

    #[test]
    fn live_kill_recovery_preserves_transaction_boundary_and_chain() {
        let Some(config) = live_config() else {
            return;
        };
        for (suffix, point, expected_events) in [
            ("before", "before_commit", 0_u64),
            ("after", "after_commit", 1_u64),
        ] {
            let crash_config = with_schema(&config, suffix);
            let child = Command::new(std::env::current_exe().expect("current test executable"))
                .args(["--exact", "tests::live_crash_append_child", "--nocapture"])
                .env("COLOSSUS_POSTGRES_TEST_CRASH_SCHEMA", &crash_config.schema)
                .env("COLOSSUS_POSTGRES_TEST_CRASH_POINT", point)
                .status()
                .expect("spawn PostgreSQL crash child");
            assert!(
                !child.success(),
                "PostgreSQL crash child unexpectedly succeeded"
            );
            let reopened = open(&crash_config);
            let report = reopened.verify().expect("verify after crash");
            assert_eq!(report.event_count, expected_events);
            assert_eq!(report.last_sequence, expected_events);
        }
    }

    #[test]
    fn live_shared_journal_and_projection_conformance() {
        let Some(config) = live_config() else {
            return;
        };
        let journal = open(&config);
        assert_journal_conformance(
            &journal,
            event("conformance", 0, 1),
            event("conformance", 0, 2),
        );
        let projection_config = PostgresJournalConfig {
            schema: format!("{}_projection", config.schema),
            ..config
        };
        assert_projection_store_conformance(&open(&projection_config));
    }

    #[test]
    fn live_concurrent_writers_preserve_one_global_chain_and_stream_conflicts() {
        let Some(config) = live_config() else {
            return;
        };
        let first = Arc::new(open(&config));
        let second = Arc::new(open(&config));
        let barrier = Arc::new(Barrier::new(2));
        let handles = [first, second]
            .into_iter()
            .enumerate()
            .map(|(index, journal)| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    journal.append(event("shared-stream", 0, index as u64))
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("writer thread"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(StoreError::Conflict { .. })))
                .count(),
            1
        );
        let reopened = open(&config);
        assert_eq!(
            reopened
                .verify()
                .expect("verify concurrent chain")
                .event_count,
            1
        );
        reopened
            .append(event("other-stream", 0, 3))
            .expect("second global writer");
        assert_eq!(
            reopened.verify().expect("verify two streams").event_count,
            2
        );
    }

    #[test]
    fn live_outage_is_sanitized_and_a_later_operation_reconnects() {
        let (Ok(_available_url), Ok(_unavailable_url)) = (
            std::env::var("COLOSSUS_TEST_POSTGRES_URL"),
            std::env::var("COLOSSUS_TEST_POSTGRES_OUTAGE_URL"),
        ) else {
            return;
        };
        let config = PostgresJournalConfig::new(
            "COLOSSUS_TEST_POSTGRES_URL",
            format!("colossus_test_{}", Uuid::now_v7().simple()),
            PostgresTlsConfig::Disabled,
        )
        .expect("outage config");
        let journal = open(&config);
        journal
            .append(event("before-outage", 0, 1))
            .expect("initial append");

        let unavailable = PostgresJournalConfig {
            connection_variable: "COLOSSUS_TEST_POSTGRES_OUTAGE_URL".into(),
            ..config.clone()
        };
        let outage = match PostgresEventJournal::open(
            unavailable,
            Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
        ) {
            Ok(_) => panic!("unavailable database must fail"),
            Err(error) => error,
        };
        assert!(!outage.to_string().contains("credential-must-not-appear"));

        journal
            .append(event("after-outage", 0, 3))
            .expect("adapter reconnects after outage");
        assert_eq!(journal.verify().expect("verify recovery").event_count, 2);
    }

    #[test]
    fn live_shared_repository_and_external_queue_conformance() {
        let Some(config) = live_config() else {
            return;
        };

        let sessions: Arc<dyn EventJournal> = Arc::new(open(&with_schema(&config, "sessions")));
        assert_session_repository_conformance(|| {
            Box::new(EventSourcedSessionRepository::new(Arc::clone(&sessions)))
        });

        let work: Arc<dyn EventJournal> = Arc::new(open(&with_schema(&config, "work")));
        assert_work_repository_conformance(|| {
            Box::new(EventSourcedWorkRepository::new(Arc::clone(&work)))
        });

        let workflows: Arc<dyn EventJournal> = Arc::new(open(&with_schema(&config, "workflows")));
        assert_workflow_repository_conformance(|| {
            Box::new(EventSourcedWorkflowRepository::new(Arc::clone(&workflows)))
        });

        let queue_journal = Arc::new(open(&with_schema(&config, "queue")));
        let journal: Arc<dyn EventJournal> = queue_journal.clone();
        let projection: Arc<dyn ProjectionStore> = queue_journal;
        let queue = JournalExternalWorkQueue::new(Arc::clone(&journal), projection);
        assert_external_work_queue_conformance(
            journal.as_ref(),
            &queue,
            event("queue-one", 0, 1),
            event("queue-two", 0, 2),
        );
    }
}
