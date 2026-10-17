use super::*;
use colossus_contracts::{
    BrowserDocumentId, BrowserMode, BrowserOpenOptions, BrowserScope, BrowserSessionId,
    BrowserTabId,
};
use std::os::unix::fs::PermissionsExt as _;

fn owner() -> BrowserSessionBinding {
    BrowserSessionBinding {
        runtime_id: "runtime".into(),
        workspace_id: "workspace".into(),
        application_id: "application".into(),
        scope: BrowserScope::Conversation {
            id: "conversation".into(),
        },
    }
}
fn request() -> BrowserDriverOpenRequest {
    BrowserDriverOpenRequest {
        binding: owner(),
        run_id: Some("run".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", "a".repeat(32))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", "b".repeat(32))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", "c".repeat(32))).unwrap(),
        options: BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Headless,
            allowed_origins: vec![BrowserOrigin::parse("https://identity.example").unwrap()],
            initial_url: None,
        },
    }
}
fn enrollment() -> OciBrowserPki {
    OciBrowserPki::new(
        owner(),
        Vec::new(),
        vec![
            OciBrowserIdentity::new(
                Zeroizing::new(b"synthetic-encrypted-pkcs12".to_vec()),
                HostSecret::new("synthetic-private-password").unwrap(),
            )
            .unwrap(),
        ],
        vec![OciClientIdentityBinding {
            origin: BrowserOrigin::parse("https://identity.example").unwrap(),
            fingerprint_sha256: "a".repeat(64),
        }],
    )
    .unwrap()
}

#[test]
fn enrollment_stages_only_native_private_paths_and_never_clobbers_material() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(temporary.path()).unwrap();
    let pki = enrollment();
    let mut staged = pki.stage(&root, &request()).unwrap();
    let serialized = serde_json::to_string(staged.bootstrap()).unwrap();
    assert!(!serialized.contains("synthetic-private-password"));
    assert!(!serialized.contains("synthetic-encrypted-pkcs12"));
    assert!(serialized.contains("\"require_source_retirement\":true"));
    assert!(!serialized.contains(temporary.path().to_str().unwrap()));
    assert!(serialized.contains("/run/colossus-browser-control/pki/identity-0.pfx"));
    let password = temporary.path().join("pki/password-0");
    assert_eq!(
        std::fs::read(&password).unwrap(),
        b"synthetic-private-password"
    );
    assert!(pki.stage(&root, &request()).is_err());
    assert_eq!(
        std::fs::read(password).unwrap(),
        b"synthetic-private-password"
    );
    staged.release_inputs().unwrap();
    assert!(!temporary.path().join("pki/identity-0.pfx").exists());
    assert!(!temporary.path().join("pki/password-0").exists());
    staged.release_inputs().unwrap();
}

#[test]
fn retirement_preserves_replaced_unknown_inputs_and_withholds_acknowledgement() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(temporary.path()).unwrap();
    let mut staged = enrollment().stage(&root, &request()).unwrap();
    let original = temporary.path().join("pki/identity-0.pfx");
    let retained = temporary.path().join("pki/retained-original");
    std::fs::rename(&original, &retained).unwrap();
    std::fs::write(&original, b"unknown-replacement-must-survive").unwrap();
    assert_eq!(
        staged.release_inputs(),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(retained.exists());
    assert!(temporary.path().join("pki/password-0").exists());
    let quarantine = std::fs::read_dir(temporary.path().join("pki"))
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".retired-input-")
        })
        .unwrap();
    assert_eq!(
        std::fs::read(quarantine.path()).unwrap(),
        b"unknown-replacement-must-survive"
    );
    assert_eq!(
        staged.release_inputs(),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(quarantine.path().exists());
}

#[test]
fn ownership_and_narrow_origin_envelopes_withhold_private_keys() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(temporary.path()).unwrap();
    let pki = enrollment();
    let mut foreign = request();
    foreign.binding.application_id = "other-app".into();
    assert!(matches!(
        pki.stage(&root, &foreign),
        Err(BrowserDriverError::Denied)
    ));
    assert!(!temporary.path().join("pki").exists());
    let mut narrow = request();
    narrow.options.allowed_origins = vec![BrowserOrigin::parse("https://public.example").unwrap()];
    let staged = pki.stage(&root, &narrow).unwrap();
    assert!(staged.bootstrap().identities.is_empty());
    assert!(staged.bootstrap().bindings.is_empty());
    assert_eq!(
        std::fs::read_dir(temporary.path().join("pki"))
            .unwrap()
            .count(),
        0
    );
    let mut different = enrollment();
    different.owner.workspace_id = "other-workspace".into();
    assert_ne!(pki.policy_digest(), different.policy_digest());
}

#[test]
fn enrollment_rejects_non_ca_inputs_and_ambiguous_native_password_formats() {
    assert!(
        OciBrowserPki::new(owner(), vec![b"not-a-ca".to_vec()], Vec::new(), Vec::new()).is_err()
    );
    for password in ["line\nend", "nonasciié", &"a".repeat(129)] {
        assert!(
            OciBrowserIdentity::new(
                Zeroizing::new(b"synthetic-pfx".to_vec()),
                HostSecret::new(password).unwrap()
            )
            .is_err()
        );
    }
    assert!(
        OciBrowserIdentity::new(Zeroizing::new(vec![]), HostSecret::new("password").unwrap())
            .is_err()
    );
}
