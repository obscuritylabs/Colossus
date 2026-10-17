//! Synthetic signed publisher fixtures exercise verification only; no real
//! Chromium process, installed image or production acceptance is inferred.
use super::{files, manifest};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_bundles::{BundleTrustStore, canonical_bundle_signing_bytes};
use colossus_contracts::{
    BrowserCapabilities, BrowserLimits, BundleFileEntry, BundleManifest, BundleSignature,
};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
};

fn key() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}
fn trust() -> BundleTrustStore {
    let public = key().verifying_key().to_bytes();
    BTreeMap::from([(
        "colossus".into(),
        BTreeMap::from([(files::hash(&public), BASE64.encode(public))]),
    )])
}
fn write(root: &Path, name: &str, value: &Value) {
    fs::write(
        root.join(name),
        serde_json::to_vec(value).expect("fixture JSON"),
    )
    .expect("write");
}
fn seal(root: &Path) -> Vec<u8> {
    let mut signed = BundleManifest {
        format_version: 1,
        name: manifest::MANIFEST_NAME.into(),
        publisher: "colossus".into(),
        version: "synthetic-test".into(),
        created_at: "2026-10-10T00:00:00Z".into(),
        source_revision: None,
        files: manifest::FIXED_FILES
            .into_iter()
            .map(|path| {
                let bytes = fs::read(root.join(path)).expect("fixture bytes");
                BundleFileEntry {
                    path: path.into(),
                    sha256: files::hash(&bytes),
                    size: Some(bytes.len() as u64),
                }
            })
            .collect(),
        signatures: Vec::new(),
    };
    signed.signatures.push(BundleSignature {
        algorithm: "ed25519".into(),
        key_id: files::hash(&key().verifying_key().to_bytes()),
        signature: BASE64.encode(
            key()
                .sign(&canonical_bundle_signing_bytes(&signed).expect("canonical"))
                .to_bytes(),
        ),
    });
    let bytes = serde_json::to_vec(&signed).expect("signed manifest");
    fs::write(root.join("manifest.json"), &bytes).expect("write manifest");
    bytes
}
fn fixture(root: &Path) -> Vec<u8> {
    fs::create_dir(root.join("component")).expect("component");
    fs::create_dir(root.join("component/extensions")).expect("directory");
    fs::set_permissions(
        root.join("component/extensions"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("mode");
    let host = b"synthetic fixture, never executable Chromium";
    fs::write(root.join("component/colossus-native-browser-host"), host).expect("host fixture");
    fs::set_permissions(
        root.join("component/colossus-native-browser-host"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("mode");
    let inventory = json!({"schema_version":1,"protocol_version":1,"component":"colossus-browser",
        "platform":"linux64","target":manifest::TARGET,"executable":"colossus-native-browser-host",
        "cef_version":"synthetic-cef","chromium_version":"synthetic-chromium","archive_sha256":"d".repeat(64),
        "modes":{"headless":true,"desktop":false},"files":[
            {"path":"colossus-native-browser-host","kind":"file","mode":0o755,"size":host.len(),"sha256":files::hash(host)},
            {"path":"extensions","kind":"directory","mode":0o700}]});
    write(root, "component/browser-component.json", &inventory);
    let inventory_sha =
        files::hash(&fs::read(root.join("component/browser-component.json")).expect("inventory"));
    let image = b"synthetic image, never loaded";
    fs::write(root.join("runtime-image.tar"), image).expect("image fixture");
    let cap = BrowserCapabilities {
        available: true,
        engine_version: Some("synthetic-cef".into()),
        modes: vec![colossus_contracts::BrowserMode::Headless],
        actions: vec![
            colossus_contracts::BrowserActionKind::Navigate,
            colossus_contracts::BrowserActionKind::Snapshot,
        ],
        limits: BrowserLimits::default(),
        private_ca_trust: false,
        client_identities: false,
        restrictive_egress: true,
    };
    write(
        root,
        "browser-release.json",
        &json!({"schema_version":1,"purpose":manifest::MANIFEST_NAME,
        "target":manifest::TARGET,"image_id":format!("sha256:{}", "a".repeat(64)),
        "image_archive_sha256":files::hash(image),"component_manifest_sha256":inventory_sha,"capabilities":cap}),
    );
    write(
        root,
        "acceptance.json",
        &json!({"schema_version":1,"target":manifest::TARGET,
        "image_id":format!("sha256:{}", "a".repeat(64)),"image_archive_sha256":files::hash(image),
        "component_manifest_sha256":inventory_sha,"chromium_sandbox_verified":true,"process_tree_containment_verified":true,
        "egress_denial_verified":true,"typed_actions_verified":true,"cleanup_verified":true,
        "installed_browser_package_verified":true,"production_acceptance":true}),
    );
    seal(root)
}

#[test]
fn independently_bound_payload_verifies_without_launching_or_mutating_a_daemon() {
    let root = tempfile::tempdir().expect("private test directory");
    let signed = fixture(root.path());
    let release =
        files::verify_with_trust(root.path(), &signed, trust()).expect("synthetic signed payload");
    assert_eq!(release.target, manifest::TARGET);
    assert!(manifest::signed(&signed, manifest::trust().expect("publisher trust")).is_err());
}
#[test]
fn signed_but_unaccepted_proof_or_component_modes_never_enable_a_package() {
    for field in [
        "chromium_sandbox_verified",
        "process_tree_containment_verified",
        "egress_denial_verified",
        "typed_actions_verified",
        "cleanup_verified",
        "installed_browser_package_verified",
        "production_acceptance",
    ] {
        let root = tempfile::tempdir().expect("private test directory");
        fixture(root.path());
        let mut value: Value = serde_json::from_slice(
            &fs::read(root.path().join("acceptance.json")).expect("receipt"),
        )
        .expect("JSON");
        value[field] = json!(false);
        write(root.path(), "acceptance.json", &value);
        assert!(
            files::verify_with_trust(root.path(), &seal(root.path()), trust()).is_err(),
            "{field}"
        );
    }
    let root = tempfile::tempdir().expect("private test directory");
    fixture(root.path());
    let mut value: Value = serde_json::from_slice(
        &fs::read(root.path().join("component/browser-component.json")).expect("inventory"),
    )
    .expect("JSON");
    value["modes"]["headless"] = json!(false);
    write(root.path(), "component/browser-component.json", &value);
    let inventory_digest = files::hash(
        &fs::read(root.path().join("component/browser-component.json")).expect("inventory"),
    );
    for path in ["browser-release.json", "acceptance.json"] {
        let mut bound: Value =
            serde_json::from_slice(&fs::read(root.path().join(path)).expect("binding"))
                .expect("JSON");
        bound["component_manifest_sha256"] = json!(inventory_digest);
        write(root.path(), path, &bound);
    }
    assert!(files::verify_with_trust(root.path(), &seal(root.path()), trust()).is_err());
}

#[test]
fn signed_capabilities_cannot_exceed_the_compiled_ceiling_or_enroll_unimplemented_pki() {
    for (field, value) in [
        ("available", json!(false)),
        ("restrictive_egress", json!(false)),
        ("private_ca_trust", json!(true)),
        ("client_identities", json!(true)),
        ("actions", json!(["navigate", "navigate"])),
        ("modes", json!(["embedded"])),
        (
            "limits",
            json!({"max_sessions": 0, "max_tabs": 8,"max_concurrent_actions": 1,
                "max_snapshot_nodes": 1,"max_observation_bytes": 1,"action_timeout_ms": 1,
                "navigation_timeout_ms": 1,"max_lease_ms": 1}),
        ),
    ] {
        let root = tempfile::tempdir().expect("private test directory");
        fixture(root.path());
        let mut release: Value = serde_json::from_slice(
            &fs::read(root.path().join("browser-release.json")).expect("release"),
        )
        .expect("JSON");
        release["capabilities"][field] = value;
        write(root.path(), "browser-release.json", &release);
        assert!(
            files::verify_with_trust(root.path(), &seal(root.path()), trust()).is_err(),
            "{field}"
        );
    }
}
#[test]
fn tampered_extra_linked_or_mode_changed_payload_is_rejected() {
    for mutation in 0..5 {
        let root = tempfile::tempdir().expect("private test directory");
        let signed = fixture(root.path());
        let host = root.path().join("component/colossus-native-browser-host");
        match mutation {
            0 => fs::write(&host, b"modified").expect("tamper"),
            1 => fs::write(root.path().join("component/unlisted"), b"extra").expect("extra"),
            2 => {
                fs::remove_file(&host).expect("remove");
                symlink("../runtime-image.tar", &host).expect("link");
            }
            3 => fs::set_permissions(&host, fs::Permissions::from_mode(0o777)).expect("mode"),
            _ => fs::write(root.path().join("runtime-image.tar"), b"changed image").expect("image"),
        }
        assert!(
            files::verify_with_trust(root.path(), &signed, trust()).is_err(),
            "mutation{mutation}"
        );
    }
}
