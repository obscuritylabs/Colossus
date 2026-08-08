//! Mode-locked, hash-chained redb event journal with plaintext and encrypted payloads.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
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
use colossus_ports::{
    CheckpointSigner, EventJournal, JournalPayloadProtection, KeyProvider, MAX_STREAM_LIST_BATCH,
    MAX_STREAM_READ_BATCH, ProjectionStore, StoreError, VerificationReport,
};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use fs4::fs_std::FileExt as _;
use redb::{
    Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition, TableHandle,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

mod common;
pub use common::RedbWriterLease;
use common::*;

mod keys;
#[cfg(test)]
use keys::cached_platform_secret;
pub use keys::{
    EnvironmentKeyProvider, PlaintextKeyProvider, PlatformKeyProvider, StaticKeyProvider,
    platform_secret,
};

mod crypto;
use crypto::*;
pub use crypto::{DisabledCheckpointSigner, Ed25519CheckpointSigner};

mod journal;
pub use journal::RedbEventJournal;

#[cfg(test)]
mod tests;
