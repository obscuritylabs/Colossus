use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_bundles::{bundle_signing_key_info, canonical_bundle_signing_bytes};
use colossus_contracts::{BundleFileEntry, BundleSignature};
use ed25519_dalek::{Signer as _, SigningKey};

fn signed(payload: &[u8], seed: [u8; 32]) -> (Vec<u8>, BundleTrustStore) {
    let key = bundle_signing_key_info(seed);
    let mut manifest = BundleManifest {
        format_version: 1,
        name: PURPOSE.into(),
        version: "0.1.0".into(),
        publisher: PUBLISHER.into(),
        created_at: "2026-10-11T00:00:00Z".into(),
        source_revision: None,
        files: vec![BundleFileEntry {
            path: DESCRIPTOR.into(),
            size: Some(payload.len() as u64),
            sha256: hex::encode(Sha256::digest(payload)),
        }],
        signatures: Vec::new(),
    };
    manifest.signatures.push(BundleSignature {
        algorithm: "ed25519".into(),
        key_id: key.key_id.clone(),
        signature: BASE64.encode(
            SigningKey::from_bytes(&seed)
                .sign(&canonical_bundle_signing_bytes(&manifest).unwrap())
                .to_bytes(),
        ),
    });
    (
        serde_json::to_vec(&manifest).unwrap(),
        BTreeMap::from([(
            PUBLISHER.into(),
            BTreeMap::from([(key.key_id, key.public_key)]),
        )]),
    )
}

#[test]
fn external_descriptor_requires_the_independent_compiler_publisher_key() {
    let payload = br#"{"native":"reviewed-package"}"#;
    let (manifest, trust) = signed(payload, [1; 32]);
    authenticate(&manifest, payload, trust.clone()).unwrap();
    assert!(authenticate(&manifest, br#"{"native":"changed--package"}"#, trust).is_err());
    let (_, wrong) = signed(payload, [2; 32]);
    assert!(authenticate(&manifest, payload, wrong).is_err());
}

#[test]
fn signer_cannot_expand_manifest_into_a_path_selector_or_other_package_purpose() {
    let payload = br#"{"native":"reviewed-package"}"#;
    let (manifest, trust) = signed(payload, [1; 32]);
    let mut altered: BundleManifest = serde_json::from_slice(&manifest).unwrap();
    altered.files[0].path = "../owned-process-host.json".into();
    assert!(
        authenticate(
            &serde_json::to_vec(&altered).unwrap(),
            payload,
            trust.clone()
        )
        .is_err()
    );
    altered.files[0].path = DESCRIPTOR.into();
    altered.name = super::super::manifest::MANIFEST_NAME.into();
    assert!(authenticate(&serde_json::to_vec(&altered).unwrap(), payload, trust).is_err());
}

#[test]
fn signed_loader_binds_fixed_private_files_and_rejects_changed_payload_or_links() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let key = bundle_signing_key_info([1; 32]);
    let trust = serde_json::to_vec(&serde_json::json!({"schema_version":1,"purpose":PURPOSE,
        "package_root":root,"publisher":PUBLISHER,"key_id":key.key_id,"public_key":key.public_key}))
    .unwrap();
    let payload = serde_json::to_vec(&serde_json::json!({"schema_version":1,"purpose":PURPOSE,
        "production_accepted":false,"allocation_root":root,"app_bundle":root.join("Browser.app"),
        "manifest_sha256":"1".repeat(64),"front_cdhash":"2".repeat(40),"body_cdhash":"3".repeat(40),
        "policy_sha256":"4".repeat(64),"fixture_origin":"http://127.0.0.1:60924",
        "broker_cdhash":"5".repeat(40),"helper_cdhashes":["6".repeat(40),"7".repeat(40),"8".repeat(40),"8".repeat(40)]})).unwrap();
    let (manifest, _) = signed(&payload, [1; 32]);
    for (name, bytes) in [
        (MANIFEST, manifest.as_slice()),
        (DESCRIPTOR, payload.as_slice()),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
        std::fs::set_permissions(root.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let (seal, owned, tls) = verify(&trust).unwrap();
    assert_eq!(seal.allocation_root, root);
    assert_eq!(owned.broker_cdhash, [0x55; 20]);
    assert!(tls.is_none());
    let mut https: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    https["fixture_origin"] = "https://127.0.0.1:60924".into();
    https["tls"] = serde_json::json!({"broker_cdhash":"9".repeat(40),"destinations":["127.0.0.1:60924"],"public_roots":[[1]]});
    let https_bytes = serde_json::to_vec(&https).unwrap();
    let (https_manifest, _) = signed(&https_bytes, [1; 32]);
    std::fs::write(root.join(MANIFEST), https_manifest).unwrap();
    std::fs::write(root.join(DESCRIPTOR), https_bytes).unwrap();
    let (_, _, tls) = verify(&trust).unwrap();
    assert_eq!(tls.unwrap().broker_cdhash, [0x99; 20]);
    https["tls"]["destinations"] = serde_json::json!(["127.0.0.1:60925"]);
    let invalid = serde_json::to_vec(&https).unwrap();
    let (invalid_manifest, _) = signed(&invalid, [1; 32]);
    std::fs::write(root.join(MANIFEST), invalid_manifest).unwrap();
    std::fs::write(root.join(DESCRIPTOR), invalid).unwrap();
    assert!(verify(&trust).is_err());
    let (manifest, _) = signed(&payload, [1; 32]);
    std::fs::write(root.join(MANIFEST), manifest).unwrap();
    std::fs::write(root.join(DESCRIPTOR), b"{} ").unwrap();
    assert!(verify(&trust).is_err());
    std::fs::write(root.join(DESCRIPTOR), &payload).unwrap();
    let outside = root.join("saved-descriptor");
    std::fs::rename(root.join(DESCRIPTOR), &outside).unwrap();
    symlink(&outside, root.join(DESCRIPTOR)).unwrap();
    assert!(verify(&trust).is_err());
}
