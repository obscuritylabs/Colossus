//! Verified private repository-configuration inspection contracts.

use colossus_runtime::RuntimeConfig;
use colossus_sdk::{Sha256Digest, VerifiedExecutable, inspect_sidecar_configuration};
use sha2::{Digest as _, Sha256};

fn sidecar() -> VerifiedExecutable {
    let path = std::path::PathBuf::from(env!("CARGO_BIN_EXE_colossus-sidecar"));
    let source = std::fs::read(&path).expect("read sidecar");
    let digest: [u8; 32] = Sha256::digest(source).into();
    VerifiedExecutable::new(path, Sha256Digest::from_bytes(digest)).expect("verified sidecar")
}

#[tokio::test]
async fn verified_sidecar_inspects_canonical_configuration_without_opening_runtime_state() {
    let yaml = RuntimeConfig::offline_template("state.redb")
        .to_yaml()
        .expect("runtime YAML");
    let response = inspect_sidecar_configuration(&sidecar(), yaml)
        .await
        .expect("inspection response");

    assert!(response.canonical_config.is_some());
    assert!(response.error_code.is_none());
    assert!(
        response
            .explicit_field_ids
            .iter()
            .any(|field| field == "schemaVersion")
    );
}

#[tokio::test]
async fn verified_sidecar_returns_only_a_sanitized_validation_code() {
    let response = inspect_sidecar_configuration(
        &sidecar(),
        "schemaVersion: 2\nunknownSecret: never-return-this\n".into(),
    )
    .await
    .expect("inspection response");

    assert!(response.canonical_config.is_none());
    assert_eq!(
        response.error_code.as_deref(),
        Some("invalid_configuration")
    );
    assert!(!format!("{response:?}").contains("never-return-this"));
}
