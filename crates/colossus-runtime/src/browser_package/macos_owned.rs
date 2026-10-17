//! Signed Debug package binding breaks the Core/native-parent code-hash cycle.
//!
//! Core embeds only the independent public trust anchor and a fixed private
//! package root. Native creator/keeper binaries subsequently seal that completed
//! Core CDHash. The final package descriptor is then signed outside Core, so its
//! newly built broker bytes never change Core's independently sealed identity.

use super::{BrowserPackageError, macos_diagnostic::Seal};
use colossus_bundles::{BundleService, BundleTrustStore};
use colossus_contracts::BundleManifest;
use colossus_home::ConfinedRoot;
use colossus_sandbox::{MacosBrowserOwnedProcessConfig, MacosBrowserTlsConfig};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::PathBuf,
};

const PURPOSE: &str = "colossus-macos-owned-process-development";
const PUBLISHER: &str = "colossus-macos-development";
const MANIFEST: &str = "macos-browser-development.bundle.json";
const DESCRIPTOR: &str = "owned-process-host.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Trust {
    schema_version: u32,
    purpose: String,
    package_root: PathBuf,
    publisher: String,
    key_id: String,
    public_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema_version: u32,
    purpose: String,
    production_accepted: bool,
    allocation_root: PathBuf,
    app_bundle: PathBuf,
    manifest_sha256: String,
    front_cdhash: String,
    body_cdhash: String,
    policy_sha256: String,
    fixture_origin: String,
    broker_cdhash: String,
    helper_cdhashes: [String; 4],
    tls: Option<TlsDescriptor>,
    custody_denials_sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsDescriptor {
    broker_cdhash: String,
    destinations: Vec<std::net::SocketAddr>,
    public_roots: Vec<Vec<u8>>,
    pki_registry_sha256: Option<String>,
}

pub(super) fn verify(
    compiler_trust: &[u8],
) -> Result<
    (
        Seal,
        MacosBrowserOwnedProcessConfig,
        Option<MacosBrowserTlsConfig>,
    ),
    BrowserPackageError,
> {
    let trust: Trust =
        serde_json::from_slice(compiler_trust).map_err(|_| BrowserPackageError::Invalid)?;
    let keys = trust_store(&trust)?;
    let root = ConfinedRoot::bind(&trust.package_root).map_err(|_| BrowserPackageError::Invalid)?;
    // Exactly two fixed relative payload names. No manifest-selected path is
    // opened before publisher authentication; the component inventory itself
    // is independently verified by the native supervisor.
    let manifest_bytes = read(&root, MANIFEST, 16 * 1024)?;
    let descriptor_bytes = read(&root, DESCRIPTOR, 16 * 1024)?;
    authenticate(&manifest_bytes, &descriptor_bytes, keys)?;
    let descriptor: Descriptor =
        serde_json::from_slice(&descriptor_bytes).map_err(|_| BrowserPackageError::Invalid)?;
    if descriptor.schema_version != 1
        || descriptor.purpose != PURPOSE
        || descriptor.production_accepted
        || descriptor.allocation_root != root.path()
        || !descriptor.app_bundle.starts_with(root.path())
        || descriptor
            .custody_denials_sha256
            .as_deref()
            .is_some_and(|value| !super::manifest::digest(value))
    {
        return Err(BrowserPackageError::Invalid);
    }
    root.revalidate()
        .map_err(|_| BrowserPackageError::Invalid)?;
    let owned = MacosBrowserOwnedProcessConfig {
        broker_cdhash: digest(&descriptor.broker_cdhash)?,
        helper_cdhashes: [
            digest(&descriptor.helper_cdhashes[0])?,
            digest(&descriptor.helper_cdhashes[1])?,
            digest(&descriptor.helper_cdhashes[2])?,
            digest(&descriptor.helper_cdhashes[3])?,
        ],
    };
    let tls = descriptor
        .tls
        .map(|tls| {
            let origin = colossus_contracts::BrowserOrigin::parse(&descriptor.fixture_origin)
                .map_err(|_| BrowserPackageError::Invalid)?;
            let port = origin
                .as_str()
                .strip_prefix("https://127.0.0.1:")
                .and_then(|port| port.parse::<u16>().ok())
                .filter(|port| !matches!(port, 0 | 443))
                .ok_or(BrowserPackageError::Invalid)?;
            if tls.destinations.as_slice() != [std::net::SocketAddr::from(([127, 0, 0, 1], port))]
                || tls.public_roots.is_empty()
                || tls.public_roots.len() > 4
                || tls
                    .public_roots
                    .iter()
                    .any(|root| root.is_empty() || root.len() > 8192)
                || tls.public_roots.iter().map(Vec::len).sum::<usize>() > 16 * 1024
            {
                return Err(BrowserPackageError::Invalid);
            }
            Ok(MacosBrowserTlsConfig {
                broker_cdhash: digest(&tls.broker_cdhash)?,
                origin,
                destinations: tls.destinations,
                public_roots: tls.public_roots,
                pki_registry_sha256: tls
                    .pki_registry_sha256
                    .as_deref()
                    .map(registry_digest)
                    .transpose()?,
            })
        })
        .transpose()?;
    if descriptor.fixture_origin.starts_with("https://") != tls.is_some() {
        return Err(BrowserPackageError::Invalid);
    }
    Ok((
        Seal {
            allocation_root: descriptor.allocation_root,
            app_bundle: descriptor.app_bundle,
            manifest_sha256: descriptor.manifest_sha256,
            front_cdhash: descriptor.front_cdhash,
            body_cdhash: descriptor.body_cdhash,
            policy_sha256: descriptor.policy_sha256,
            fixture_origin: descriptor.fixture_origin,
        },
        owned,
        tls,
    ))
}

fn digest(value: &str) -> Result<[u8; 20], BrowserPackageError> {
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(BrowserPackageError::Invalid);
    }
    let bytes: [u8; 20] = hex::decode(value)
        .map_err(|_| BrowserPackageError::Invalid)?
        .try_into()
        .map_err(|_| BrowserPackageError::Invalid)?;
    if bytes == [0; 20] {
        return Err(BrowserPackageError::Invalid);
    }
    Ok(bytes)
}
fn registry_digest(value: &str) -> Result<[u8; 32], BrowserPackageError> {
    if !super::manifest::digest(value) {
        return Err(BrowserPackageError::Invalid);
    }
    let digest = hex::decode(value)
        .map_err(|_| BrowserPackageError::Invalid)?
        .try_into()
        .map_err(|_| BrowserPackageError::Invalid)?;
    if digest == [0; 32] {
        return Err(BrowserPackageError::Invalid);
    }
    Ok(digest)
}

fn trust_store(trust: &Trust) -> Result<BundleTrustStore, BrowserPackageError> {
    if trust.schema_version != 1
        || trust.purpose != PURPOSE
        || trust.publisher != PUBLISHER
        || !trust.package_root.is_absolute()
        || !super::manifest::digest(&trust.key_id)
        || trust.public_key.len() > 64
    {
        return Err(BrowserPackageError::Invalid);
    }
    Ok(BTreeMap::from([(
        PUBLISHER.into(),
        BTreeMap::from([(trust.key_id.clone(), trust.public_key.clone())]),
    )]))
}

fn authenticate(
    manifest_bytes: &[u8],
    payload: &[u8],
    trust: BundleTrustStore,
) -> Result<(), BrowserPackageError> {
    if manifest_bytes.len() > 16 * 1024 || payload.len() > 16 * 1024 || payload.is_empty() {
        return Err(BrowserPackageError::Invalid);
    }
    let manifest: BundleManifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| BrowserPackageError::Invalid)?;
    if manifest.name != PURPOSE
        || manifest.publisher != PUBLISHER
        || manifest.files.len() != 1
        || manifest.files[0].path != DESCRIPTOR
        || manifest.files[0].size != Some(payload.len() as u64)
        || manifest.files[0].sha256 != hex::encode(Sha256::digest(payload))
    {
        return Err(BrowserPackageError::Invalid);
    }
    BundleService::new(trust)
        .verify_manifest_signature(&manifest)
        .map_err(|_| BrowserPackageError::Invalid)?;
    Ok(())
}

fn read(root: &ConfinedRoot, name: &str, ceiling: u64) -> Result<Vec<u8>, BrowserPackageError> {
    root.revalidate()
        .map_err(|_| BrowserPackageError::Invalid)?;
    let path = root.path().join(name);
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&path)
        .map_err(|_| BrowserPackageError::Invalid)?;
    let before = file.metadata().map_err(|_| BrowserPackageError::Invalid)?;
    if !before.is_file()
        || before.nlink() != 1
        || before.mode() & 0o077 != 0
        || before.len() > ceiling
    {
        return Err(BrowserPackageError::Invalid);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(ceiling + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BrowserPackageError::Invalid)?;
    let after = std::fs::symlink_metadata(&path).map_err(|_| BrowserPackageError::Invalid)?;
    if bytes.len() as u64 != before.len()
        || after.file_type().is_symlink()
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err(BrowserPackageError::Invalid);
    }
    root.revalidate()
        .map_err(|_| BrowserPackageError::Invalid)?;
    Ok(bytes)
}

#[cfg(test)]
#[path = "macos_owned_tests.rs"]
mod tests;
