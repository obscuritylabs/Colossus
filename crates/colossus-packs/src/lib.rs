//! Strict capability-pack and signed offline-bundle verification and lifecycle management.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, BundleFileEntry, BundleInstallation, BundleManifest, BundleMaterialization,
    BundleSigningKeyInfo, BundleVerification, CollectionArtifactEntry, CollectionArtifactKind,
    CollectionInstallation, CollectionManifest, CollectionMaterialization, CollectionVerification,
    EffectRequest, PackFileEntry, PackInstallation, PackManifest, PackSignature, PackStatus,
    PackVerification, PublisherTrust, QuarantinedEffectResult, RegistryPullResult,
    RegistryPushResult, ResourceAuthority, SkillInstallResult, SkillValidationResult,
};
use colossus_network::AdditionalRootCertificates;
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, NetworkDestinationMatch,
    network_authority_match,
};
use colossus_ports::{ExtensionRepository, StoreError};
use colossus_skills::{copy_verified_skill, inspect_skill_directory};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use futures::StreamExt as _;
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write as _},
    net::IpAddr,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::net::lookup_host;
use tokio_util::io::ReaderStream;

mod types;
use types::*;
pub use types::{PackError, PackOperation, current_release_target};

mod gateway;
use gateway::*;

mod archives;
use archives::*;

mod verification;
use verification::*;

mod collection;
use collection::*;

mod manifest;
use manifest::*;

mod signing;
use signing::*;
pub use signing::{
    canonical_bundle_signing_bytes, canonical_collection_signing_bytes,
    canonical_pack_signing_bytes,
};

mod service;
pub use service::PackService;

mod executor;
pub use executor::PackExecutor;

#[cfg(test)]
mod tests;
