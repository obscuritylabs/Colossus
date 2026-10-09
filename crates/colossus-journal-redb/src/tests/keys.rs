use super::*;
use crate::{EnvironmentKeyProvider, decode_secure_anchor};

#[test]
fn rotation_cannot_replace_historical_key_material() {
    let keys = StaticKeyProvider::new("first", [1; 32]);
    keys.rotate("second", [2; 32]).expect("rotate");
    assert!(matches!(
        keys.rotate("first", [3; 32]),
        Err(StoreError::Adapter(_))
    ));
    assert_eq!(keys.key_by_id("first").expect("historical key"), [1; 32]);
    assert_eq!(
        keys.active_key().expect("active key"),
        ("second".into(), [2; 32])
    );
    keys.rotate("first", [1; 32])
        .expect("reactivate retained key");
    assert_eq!(
        keys.active_key().expect("reactivated key"),
        ("first".into(), [1; 32])
    );
}

#[test]
fn environment_anchor_replaces_durably_without_reusing_a_fixed_temporary_path() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("anchor.json");
    let old_temporary = path.with_extension("tmp");
    std::fs::write(&old_temporary, b"unrelated file").expect("seed neighboring file");
    let provider = EnvironmentKeyProvider::new("UNUSED_REDB_TEST_KEY", "key", &path);
    assert_eq!(provider.load_anchor().expect("missing anchor"), None);
    for sequence in [1, 2] {
        let anchor = SecureAnchor {
            format_version: 2,
            sequence,
            hash: "ab".repeat(32),
            verification_profile: Some("full-journal-v1".into()),
            status: SecureAnchorStatus::Verified,
        };
        provider.store_anchor(&anchor).expect("persist anchor");
        assert_eq!(provider.load_anchor().expect("load anchor"), Some(anchor));
    }
    assert_eq!(
        std::fs::read(&old_temporary).expect("neighbor"),
        b"unrelated file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    std::fs::write(&path, vec![b' '; 8193]).expect("oversized anchor");
    assert!(matches!(
        provider.load_anchor(),
        Err(StoreError::Verification(_))
    ));
    std::fs::write(&path, b"{").expect("malformed anchor");
    assert!(matches!(
        provider.load_anchor(),
        Err(StoreError::Verification(_))
    ));
}

#[test]
fn secure_anchor_decoder_retains_legacy_fields_and_rejects_malformed_metadata() {
    let legacy = decode_secure_anchor(br#"{"sequence":1,"hash":"abc"}"#).expect("legacy anchor");
    assert_eq!(legacy.format_version, 1);
    assert_eq!(legacy.sequence, 1);
    assert_eq!(legacy.hash, "abc");
    assert_eq!(legacy.status, SecureAnchorStatus::Verified);
    for extra in [
        json!({"format_version": "bad"}),
        json!({"format_version": 65536}),
        json!({"status": "bad"}),
    ] {
        let mut anchor = json!({"sequence": 1, "hash": "abc"});
        anchor
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("metadata").clone());
        assert!(matches!(
            decode_secure_anchor(&serde_json::to_vec(&anchor).expect("json")),
            Err(StoreError::Verification(_))
        ));
    }
}
