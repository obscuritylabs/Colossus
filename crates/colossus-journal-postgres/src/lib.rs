//! Mode-locked PostgreSQL event journal with plaintext and encrypted payloads.

#![allow(clippy::missing_errors_doc)]

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use colossus_contracts::{
    ENCRYPTED_PAYLOAD_ALGORITHM, EncryptedPayload, EventEnvelope, NewEvent,
    PLAINTEXT_PAYLOAD_ALGORITHM, ProjectionBatch, ProjectionMutation, ProjectionWorkItem,
    SecureAnchor, SecureAnchorStatus, SignedCheckpoint, StartupVerificationMode,
    StartupVerificationReport,
};
use colossus_network::AdditionalRootCertificates;
use colossus_ports::{
    CheckpointSigner, EventJournal, JournalPayloadProtection, KeyProvider, MAX_STREAM_LIST_BATCH,
    MAX_STREAM_READ_BATCH, ProjectionStore, StoreError, VerificationReport,
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

mod common;
use common::*;

mod config;
pub use config::*;

mod crypto;
use crypto::*;

mod journal;
pub use journal::PostgresEventJournal;

#[cfg(test)]
mod tests;
