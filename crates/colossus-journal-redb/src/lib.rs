//! Mode-locked, hash-chained redb event journal with plaintext and encrypted payloads.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    ENCRYPTED_PAYLOAD_ALGORITHM, EventEnvelope, NewEvent, PLAINTEXT_PAYLOAD_ALGORITHM,
    ProjectionBatch, ProjectionMutation, ProjectionWorkItem, SecureAnchor, SecureAnchorStatus,
    SignedCheckpoint, StartupVerificationMode, StartupVerificationReport,
};
use colossus_journal_format::{
    GENESIS_HASH as ZERO_HASH, PersistedEnvelope as PersistedEventEnvelope, checkpoint_message,
    seal_event, utc_now, verify_payload,
};
use colossus_ports::{
    CheckpointSigner, EventJournal, JournalPayloadProtection, KeyProvider, MAX_STREAM_LIST_BATCH,
    MAX_STREAM_READ_BATCH, ProjectionStore, StoreError, VerificationReport,
};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use fs4::fs_std::FileExt as _;
use redb::{
    Database, ReadTransaction, ReadableDatabase, ReadableTable, ReadableTableMetadata,
    TableDefinition, TableError,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

mod common;
pub use common::RedbWriterLease;
use common::*;

mod keys;
#[cfg(test)]
use keys::cached_platform_secret;
pub use keys::{
    EnvironmentKeyProvider, PlaintextKeyProvider, PlatformKeyProvider, StaticKeyProvider,
    decode_secure_anchor, platform_secret,
};

mod crypto;
#[cfg(test)]
use crypto::crash_at_test_fault;
pub use crypto::{DisabledCheckpointSigner, Ed25519CheckpointSigner};

mod journal;
pub use journal::RedbEventJournal;
mod key_metadata;
pub use key_metadata::{JournalKeyMetadata, journal_key_metadata};

#[cfg(test)]
mod tests;
