//! Dedicated no-CEF signed native fixture. Never a production/CEF receipt.
use super::{PUBLISHER, Package, TRUST, Trust, inventory, platform, proof};
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
const PURPOSE: &str = "colossus-macos-native-custody-acceptance";
const DESCRIPTOR: &str = "native-custody-acceptance.json";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema_version: u32,
    purpose: String,
    production_accepted: bool,
    cef_artifact_receipt: bool,
    package_root: PathBuf,
    app_bundle: PathBuf,
    broker_cdhash: String,
    policy_sha256: String,
    prepared_sha256: String,
    native_result_sha256: String,
    inputs_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    schema_version: u32,
    purpose: String,
    production_accepted: bool,
    cef_artifact_receipt: bool,
    package: PathBuf,
    policy_sha256: String,
    controller_sha256: String,
    controller_cdhash: String,
    controller_source_sha256: String,
    native_source_sha256: BTreeMap<String, String>,
    roles: Vec<Role>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Role {
    role: u8,
    front_sha256: String,
    front_cdhash: String,
    body_sha256: String,
    command: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Results {
    schema_version: u32,
    purpose: String,
    prepared_sha256: String,
    production_accepted: bool,
    cef_artifact_receipt: bool,
    roles: Vec<ResultRow>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultRow {
    role: u8,
    front_cdhash: String,
    front_sha256: String,
    body_sha256: String,
    exit_code: u32,
    accepted: bool,
    receipt: Receipt,
    stderr: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    role: u8,
    pre_body_denials: u16,
    direct_child_reaped: bool,
    exit_code: u32,
    cef_loaded: bool,
    production_accepted: bool,
}
pub(super) fn admit(executable: &Path, owner: u32) -> Result<Package, Status> {
    let trust: Trust = serde_json::from_slice(TRUST).map_err(|_| Status::Denied)?;
    if trust.schema_version != 1
        || trust.purpose != super::PURPOSE
        || trust.publisher != PUBLISHER
        || proof::digest::<32>(&trust.key_id).is_err()
    {
        return Err(Status::Denied);
    }
    let root = ConfinedRoot::bind(&trust.package_root).map_err(|_| Status::Denied)?;
    if root.path() != trust.package_root
        || executable != root.path().join("colossus-native-custody-acceptance")
    {
        return Err(Status::Denied);
    }
    let (bundle_file, bundle_bytes) = inventory::HeldFile::read(
        &root.path().join("native-custody-acceptance.bundle.json"),
        16384,
        true,
        owner,
    )?;
    let (descriptor_file, descriptor_bytes) =
        inventory::HeldFile::read(&root.path().join(DESCRIPTOR), 16384, true, owner)?;
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
    let descriptor: Descriptor =
        serde_json::from_slice(&descriptor_bytes).map_err(|_| Status::Denied)?;
    let app = root.path().join("allocation/Colossus Custody Probe.app");
    if descriptor.schema_version != 1
        || descriptor.purpose != PURPOSE
        || descriptor.production_accepted
        || descriptor.cef_artifact_receipt
        || descriptor.package_root != root.path()
        || descriptor.app_bundle != app
    {
        return Err(Status::Denied);
    }
    let broker_code_hash = proof::digest(&descriptor.broker_cdhash)?;
    platform::verify_code(executable, &broker_code_hash)?;
    let (prepared_file, prepared_bytes) =
        inventory::HeldFile::read(&root.path().join("prepared.json"), 32768, true, owner)?;
    let (result_file, result_bytes) =
        inventory::HeldFile::read(&root.path().join("native-result.json"), 32768, true, owner)?;
    if Sha256::digest(&prepared_bytes).as_slice()
        != proof::digest::<32>(&descriptor.prepared_sha256)?
        || Sha256::digest(&result_bytes).as_slice()
            != proof::digest::<32>(&descriptor.native_result_sha256)?
    {
        return Err(Status::Denied);
    }
    let prepared: Prepared = serde_json::from_slice(&prepared_bytes).map_err(|_| Status::Denied)?;
    let results: Results = serde_json::from_slice(&result_bytes).map_err(|_| Status::Denied)?;
    let sources: BTreeMap<String, String> =
        serde_json::from_slice(super::SOURCES).map_err(|_| Status::Denied)?;
    if prepared.schema_version != 1
        || prepared.purpose != "colossus-native-front-custody-probe"
        || prepared.production_accepted
        || prepared.cef_artifact_receipt
        || prepared.package != app
        || prepared.policy_sha256 != descriptor.policy_sha256
        || prepared.roles.len() != 5
        || prepared.controller_source_sha256
            != include_str!(concat!(
                env!("OUT_DIR"),
                "/custody-controller-source.sha256"
            ))
        || results.schema_version != 1
        || results.purpose != "colossus-native-front-custody-probe-result"
        || results.production_accepted
        || results.cef_artifact_receipt
        || results.prepared_sha256 != descriptor.prepared_sha256
        || results.roles.len() != 5
        || sources
            .iter()
            .any(|(path, hash)| prepared.native_source_sha256.get(path) != Some(hash))
    {
        return Err(Status::Denied);
    }
    let mut files = vec![bundle_file, descriptor_file, prepared_file, result_file];
    let (controller_file, controller) = inventory::HeldFile::read(
        &root.path().join("colossus-custody-front-controller"),
        16 * 1024 * 1024,
        false,
        owner,
    )?;
    if Sha256::digest(&controller).as_slice() != proof::digest::<32>(&prepared.controller_sha256)? {
        return Err(Status::Denied);
    }
    platform::verify_code(
        &root.path().join("colossus-custody-front-controller"),
        &proof::digest(&prepared.controller_cdhash)?,
    )?;
    files.push(controller_file);
    for (index, (role, result)) in prepared.roles.iter().zip(&results.roles).enumerate() {
        let expected = index as u8;
        let r = &result.receipt;
        if role.role != expected
            || result.role != expected
            || result.front_cdhash != role.front_cdhash
            || result.front_sha256 != role.front_sha256
            || result.body_sha256 != role.body_sha256
            || result.exit_code != 0
            || !result.accepted
            || !result.stderr.is_empty()
            || r.schema_version != 1
            || r.role != expected
            || r.pre_body_denials != 511
            || !r.direct_child_reaped
            || r.exit_code != 0
            || r.cef_loaded
            || r.production_accepted
            || role.command.len() != 7
        {
            return Err(Status::Denied);
        }
        let path = if index == 0 {
            app.join("Contents/MacOS/colossus-native-browser-host")
        } else {
            let suffix = [" (Renderer)", " (GPU)", "", ""][index - 1];
            let name = format!("Colossus Browser Helper{suffix}");
            app.join(format!(
                "Contents/Frameworks/{name}.app/Contents/MacOS/{name}"
            ))
        };
        platform::verify_code(&path, &proof::digest(&role.front_cdhash)?)?;
        let (front_file, front) = inventory::HeldFile::read(&path, 16 * 1024 * 1024, false, owner)?;
        let body_path = path.with_file_name(format!(
            "{} Body.dylib",
            path.file_name()
                .and_then(|n| n.to_str())
                .ok_or(Status::Denied)?
        ));
        let (body_file, body) =
            inventory::HeldFile::read(&body_path, 16 * 1024 * 1024, false, owner)?;
        if Sha256::digest(&front).as_slice() != proof::digest::<32>(&role.front_sha256)?
            || Sha256::digest(&body).as_slice() != proof::digest::<32>(&role.body_sha256)?
        {
            return Err(Status::Denied);
        }
        files.extend([front_file, body_file]);
    }
    platform::verify_code(&app, &proof::digest(&prepared.roles[0].front_cdhash)?)?;
    let (policy_file, policy_bytes) = inventory::HeldFile::read(
        &app.join("Contents/Resources/colossus-network-envelope.policy"),
        32768,
        false,
        owner,
    )?;
    if Sha256::digest(&policy_bytes).as_slice() != proof::digest::<32>(&descriptor.policy_sha256)? {
        return Err(Status::Denied);
    }
    let custody_root = proof::policy_root(&policy_bytes, &root.path().join("allocation"), &app)?;
    let (inputs_file, inputs_bytes) = inventory::HeldFile::read(
        &custody_root.join("acceptance-inputs/identity.json"),
        16384,
        true,
        owner,
    )?;
    if Sha256::digest(&inputs_bytes).as_slice() != proof::digest::<32>(&descriptor.inputs_sha256)? {
        return Err(Status::Denied);
    }
    files.extend([policy_file, inputs_file]);
    let package = Package {
        root,
        files,
        custody_root,
        broker_code_hash,
        record_binding: Package::record_binding(&descriptor_bytes),
        registry_digest: None,
        tls_scope: None,
    };
    package.revalidate()?;
    Ok(package)
}
