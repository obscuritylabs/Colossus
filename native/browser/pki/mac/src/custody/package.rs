#[cfg(feature = "macos-custody-native-acceptance")]
mod acceptance;
mod inventory;
mod proof;

use super::platform;
use crate::Status;
use colossus_bundles::BundleService;
use colossus_contracts::BundleManifest;
use colossus_home::ConfinedRoot;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const PURPOSE: &str = "colossus-macos-owned-process-development";
const PUBLISHER: &str = "colossus-macos-development";
const BUNDLE: &str = "macos-browser-development.bundle.json";
const DESCRIPTOR: &str = "owned-process-host.json";
const DENIALS: &str = "owned-process-custody-denials.json";
const TLS: &str = "Contents/MacOS/colossus-native-browser-tls-broker";
const TRUST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/custody-package-trust.json"));
const SOURCES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/custody-policy-sources.json"));

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
    tls: Option<Tls>,
    custody_denials_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tls {
    broker_cdhash: String,
    destinations: Vec<std::net::SocketAddr>,
    public_roots: Vec<Vec<u8>>,
    pki_registry_sha256: Option<String>,
}

pub(super) struct Package {
    root: ConfinedRoot,
    files: Vec<inventory::HeldFile>,
    custody_root: PathBuf,
    broker_code_hash: [u8; 20],
    record_binding: [u8; 32],
    registry_digest: Option<[u8; 32]>,
    tls_scope: Option<(String, Vec<std::net::SocketAddr>)>,
}
impl Package {
    #[cfg(feature = "macos-custody-native-acceptance")]
    pub fn admit_acceptance(executable: &Path, owner: u32) -> Result<Self, Status> {
        acceptance::admit(executable, owner)
    }
    pub fn admit(executable: &Path, owner: u32) -> Result<Self, Status> {
        let trust: Trust = serde_json::from_slice(TRUST).map_err(|_| Status::Denied)?;
        if trust.schema_version != 1
            || trust.purpose != PURPOSE
            || trust.publisher != PUBLISHER
            || !trust.package_root.is_absolute()
            || proof::digest::<32>(&trust.key_id).is_err()
            || trust.public_key.len() > 64
        {
            return Err(Status::Denied);
        }
        let root = ConfinedRoot::bind(&trust.package_root).map_err(|_| Status::Denied)?;
        if root.path() != trust.package_root {
            return Err(Status::Denied);
        }
        let (bundle_file, bundle_bytes) =
            inventory::HeldFile::read(&root.path().join(BUNDLE), 16 * 1024, true, owner)?;
        let (descriptor_file, descriptor_bytes) =
            inventory::HeldFile::read(&root.path().join(DESCRIPTOR), 16 * 1024, true, owner)?;
        let bundle: BundleManifest =
            serde_json::from_slice(&bundle_bytes).map_err(|_| Status::Denied)?;
        if bundle.name != PURPOSE
            || bundle.publisher != PUBLISHER
            || bundle.files.len() != 1
            || bundle.files[0].path != DESCRIPTOR
            || bundle.files[0].size != Some(descriptor_bytes.len() as u64)
            || bundle.files[0].sha256 != hex::encode(Sha256::digest(&descriptor_bytes))
        {
            return Err(Status::Denied);
        }
        BundleService::new(BTreeMap::from([(
            PUBLISHER.into(),
            BTreeMap::from([(trust.key_id, trust.public_key)]),
        )]))
        .verify_manifest_signature(&bundle)
        .map_err(|_| Status::Denied)?;
        // Parse descriptor only after independent publisher authentication.
        let descriptor: Descriptor =
            serde_json::from_slice(&descriptor_bytes).map_err(|_| Status::Denied)?;
        if descriptor.schema_version != 1
            || descriptor.purpose != PURPOSE
            || descriptor.production_accepted
            || descriptor.allocation_root != root.path()
            || !descriptor.app_bundle.starts_with(root.path())
            || descriptor
                .app_bundle
                .canonicalize()
                .map_err(|_| Status::Denied)?
                != descriptor.app_bundle
            || executable != descriptor.app_bundle.join(TLS)
        {
            return Err(Status::Denied);
        }
        let tls = descriptor.tls.as_ref().ok_or(Status::Denied)?;
        let port = descriptor
            .fixture_origin
            .strip_prefix("https://127.0.0.1:")
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|port| !matches!(port, 0 | 443))
            .ok_or(Status::Denied)?;
        if tls.destinations != [std::net::SocketAddr::from(([127, 0, 0, 1], port))]
            || tls.public_roots.is_empty()
            || tls.public_roots.len() > 4
            || tls
                .public_roots
                .iter()
                .any(|der| der.is_empty() || der.len() > 8192)
            || tls.public_roots.iter().map(Vec::len).sum::<usize>() > 16 * 1024
        {
            return Err(Status::Denied);
        }
        let broker_code_hash = proof::digest(&tls.broker_cdhash)?;
        platform::verify_code(
            &descriptor.app_bundle,
            &proof::digest(&descriptor.front_cdhash)?,
        )?;
        platform::verify_code(executable, &broker_code_hash)?;
        platform::verify_code(
            &descriptor
                .app_bundle
                .join("Contents/MacOS/colossus-native-browser-host Body.dylib"),
            &proof::digest(&descriptor.body_cdhash)?,
        )?;
        platform::verify_code(
            &descriptor
                .app_bundle
                .join("Contents/MacOS/colossus-mac-browser-process-broker"),
            &proof::digest(&descriptor.broker_cdhash)?,
        )?;
        let mut roles = vec![(0, proof::digest::<20>(&descriptor.front_cdhash)?)];
        for (index, suffix) in [" (Renderer)", " (GPU)", "", ""].iter().enumerate() {
            let name = format!("Colossus Browser Helper{suffix}");
            let path = descriptor
                .app_bundle
                .join(format!("Contents/Frameworks/{name}.app"));
            let hash = proof::digest(&descriptor.helper_cdhashes[index])?;
            platform::verify_code(&path, &hash)?;
            roles.push((index as u8 + 1, hash));
        }
        let policy_path = descriptor
            .app_bundle
            .join("Contents/Resources/colossus-network-envelope.policy");
        let (policy_file, policy_bytes) =
            inventory::HeldFile::read(&policy_path, 32 * 1024, false, owner)?;
        if Sha256::digest(&policy_bytes).as_slice()
            != proof::digest::<32>(&descriptor.policy_sha256)?
        {
            return Err(Status::Denied);
        }
        let custody_root = proof::policy_root(&policy_bytes, root.path(), &descriptor.app_bundle)?;
        let manifest_path = descriptor
            .app_bundle
            .with_extension("app.browser-component.json");
        let (component_file, component_bytes) =
            inventory::HeldFile::read(&manifest_path, 8 * 1024 * 1024, false, owner)?;
        let component_hash = proof::digest::<32>(&descriptor.manifest_sha256)?;
        if Sha256::digest(&component_bytes).as_slice() != component_hash {
            return Err(Status::Denied);
        }
        let sources: BTreeMap<String, String> =
            serde_json::from_slice(SOURCES).map_err(|_| Status::Denied)?;
        let mut files =
            inventory::verify(&descriptor.app_bundle, &component_bytes, &sources, owner)?;
        let denial_hash = proof::digest::<32>(
            descriptor
                .custody_denials_sha256
                .as_deref()
                .ok_or(Status::Unavailable)?,
        )?;
        let (denial_file, denial_bytes) =
            inventory::HeldFile::read(&root.path().join(DENIALS), 32 * 1024, true, owner)?;
        if Sha256::digest(&denial_bytes).as_slice() != denial_hash {
            return Err(Status::Denied);
        }
        proof::denials(
            &denial_bytes,
            component_hash,
            proof::digest(&descriptor.policy_sha256)?,
            &roles,
            &sources,
        )?;
        files.extend([
            bundle_file,
            descriptor_file,
            policy_file,
            component_file,
            denial_file,
        ]);
        let registry_digest = tls
            .pki_registry_sha256
            .as_deref()
            .map(proof::digest::<32>)
            .transpose()?;
        if let Some(expected) = registry_digest {
            let (registry_file, bytes) = inventory::HeldFile::read(
                &custody_root.join("owned-pki-registry.json"),
                128 * 1024,
                true,
                owner,
            )?;
            if Sha256::digest(&bytes).as_slice() != expected {
                return Err(Status::Denied);
            }
            files.push(registry_file);
        }
        let result = Self {
            root,
            files,
            custody_root,
            broker_code_hash,
            record_binding: Self::record_binding(&descriptor_bytes),
            registry_digest,
            tls_scope: Some((descriptor.fixture_origin, tls.destinations.clone())),
        };
        result.revalidate()?;
        Ok(result)
    }
    pub fn revalidate(&self) -> Result<(), Status> {
        self.root.revalidate().map_err(|_| Status::Denied)?;
        for file in &self.files {
            file.revalidate()?;
        }
        Ok(())
    }
    fn record_binding(descriptor: &[u8]) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"colossus-macos-custody-package-binding-v1\0");
        digest.update(TRUST);
        digest.update(SOURCES);
        digest.update(descriptor);
        digest.finalize().into()
    }
    pub fn custody_record_binding(&self) -> [u8; 32] {
        self.record_binding
    }
    pub fn registry_digest(&self) -> Option<[u8; 32]> {
        self.registry_digest
    }
    pub fn admits_tls(&self, origin: &str, destinations: &[std::net::SocketAddr]) -> bool {
        self.tls_scope
            .as_ref()
            .is_some_and(|(signed_origin, signed_destinations)| {
                signed_origin == origin && signed_destinations == destinations
            })
    }
    pub fn custody_root(&self) -> &Path {
        &self.custody_root
    }
    pub fn broker_code_hash(&self) -> [u8; 20] {
        self.broker_code_hash
    }
}
