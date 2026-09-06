//! Signed offline release-bundle verification, materialization, and installation.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    BundleFileEntry, BundleInstallation, BundleManifest, BundleMaterialization, BundleSignature,
    BundleSigningKeyInfo, BundleVerification, EffectRequest, QuarantinedEffectResult,
    ResourceAuthority,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

mod executor;
pub use executor::BundleExecutor;

mod service;
pub use service::BundleService;

mod types;
pub use types::{BundleError, BundleOperation, BundleTrustStore, current_release_target};

const BUNDLE_MANIFEST: &str = "manifest.json";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 10_000;

/// Deterministic bytes authenticated by release-bundle signatures.
pub fn canonical_bundle_signing_bytes(manifest: &BundleManifest) -> Result<Vec<u8>, BundleError> {
    let mut unsigned = manifest.clone();
    unsigned.signatures.clear();
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            value => value,
        }
    }
    Ok(serde_json::to_vec(&sorted(serde_json::to_value(
        unsigned,
    )?))?)
}

/// Derive the public signing-key identity for an offline release-bundle seed.
#[must_use]
pub fn bundle_signing_key_info(seed: [u8; 32]) -> BundleSigningKeyInfo {
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    BundleSigningKeyInfo {
        key_id: hex::encode(Sha256::digest(public)),
        public_key: BASE64.encode(public),
    }
}

#[cfg(test)]
mod tests;
