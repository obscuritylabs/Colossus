use super::*;
use crate::service::bundle_artifact_path;
use tempfile::TempDir;

fn fixture() -> (TempDir, BundleService, [u8; 32], String, String) {
    let root = TempDir::new().expect("temp");
    let seed = [7; 32];
    let info = bundle_signing_key_info(seed);
    let trust = BTreeMap::from([(
        "colossus".into(),
        BTreeMap::from([(info.key_id.clone(), info.public_key)]),
    )]);
    (
        root,
        BundleService::new(trust),
        seed,
        info.key_id,
        "colossus".into(),
    )
}

#[test]
fn signed_bundle_round_trip_rejects_tampering() {
    let (root, service, seed, key_id, publisher) = fixture();
    let staged = root.path().join("staged");
    let artifact = staged.join(bundle_artifact_path(
        current_release_target().expect("target"),
    ));
    fs::create_dir_all(artifact.parent().expect("parent")).expect("dirs");
    fs::write(&artifact, b"release").expect("artifact");
    let destination = root.path().join("bundle");
    let built = service
        .build(
            &staged,
            &destination,
            "colossus",
            "1.0.0",
            &publisher,
            "2026-01-01T00:00:00Z",
            None,
            seed,
        )
        .expect("build");
    assert_eq!(built.signing_key_id, key_id);
    assert_eq!(
        service.verify(&destination).expect("verify"),
        built.verification
    );
    fs::write(destination.join(&built.verification.name), b"undeclared").expect("tamper");
    assert!(service.verify(&destination).is_err());
}

#[test]
fn untrusted_signer_is_rejected() {
    let root = TempDir::new().expect("temp");
    let staged = root.path().join("staged");
    let artifact = staged.join(bundle_artifact_path(
        current_release_target().expect("target"),
    ));
    fs::create_dir_all(artifact.parent().expect("parent")).expect("dirs");
    fs::write(artifact, b"release").expect("artifact");
    assert!(
        BundleService::new(BTreeMap::new())
            .build(
                &staged,
                &root.path().join("bundle"),
                "colossus",
                "1.0.0",
                "colossus",
                "2026-01-01T00:00:00Z",
                None,
                [8; 32],
            )
            .is_err()
    );
}
