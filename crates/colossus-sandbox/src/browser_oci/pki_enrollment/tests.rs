use super::*;
use colossus_contracts::{
    BrowserDocumentId, BrowserMode, BrowserOpenOptions, BrowserSessionBinding, BrowserSessionId,
    BrowserTabId, HostSecret,
};
use colossus_home::ConfinedRoot;
use std::os::unix::fs::PermissionsExt as _;
use zeroize::Zeroizing;

fn origin() -> BrowserOrigin {
    BrowserOrigin::parse("https://identity.example").unwrap()
}
fn request() -> BrowserDriverOpenRequest {
    BrowserDriverOpenRequest {
        binding: BrowserSessionBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: "application".into(),
            scope: BrowserScope::Conversation {
                id: "conversation".into(),
            },
        },
        run_id: Some("run".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", "a".repeat(32))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", "b".repeat(32))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", "c".repeat(32))).unwrap(),
        options: BrowserOpenOptions {
            mode: BrowserMode::Headless,
            allowed_origins: vec![origin()],
            initial_url: None,
            profile: Default::default(),
        },
    }
}
fn registration(scope: OciBrowserPkiScopeAuthorization, ca: bool) -> OciBrowserPkiRegistration {
    let ca = if ca {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        vec![
            params
                .self_signed(&rcgen::KeyPair::generate().unwrap())
                .unwrap()
                .der()
                .to_vec(),
        ]
    } else {
        Vec::new()
    };
    OciBrowserPkiRegistration::new(
        OciBrowserPkiAuthorization {
            workspace_id: "workspace".into(),
            application_id: "application".into(),
            scope,
            origins: vec![origin()],
        },
        ca,
        vec![
            OciBrowserIdentity::new(
                Zeroizing::new(b"synthetic-encrypted-pfx".to_vec()),
                HostSecret::new("synthetic-password").unwrap(),
            )
            .unwrap(),
        ],
        vec![OciClientIdentityBinding {
            origin: origin(),
            fingerprint_sha256: "a".repeat(64),
        }],
    )
    .unwrap()
}

#[test]
fn future_scope_native_consent_mints_each_exact_owner_after_core_runtime_identity_exists() {
    let registry = OciBrowserPkiRegistry::new(vec![registration(
        OciBrowserPkiScopeAuthorization::FutureConversations,
        false,
    )])
    .unwrap();
    let first = request();
    let first_pki = registry.enroll(&first).unwrap().unwrap();
    let mut second = request();
    second.binding.runtime_id = "new-runtime".into();
    second.binding.scope = BrowserScope::Conversation {
        id: "future-conversation".into(),
    };
    let second_pki = registry.enroll(&second).unwrap().unwrap();
    assert_ne!(first_pki.policy_digest(), second_pki.policy_digest());
    let allocation = tempfile::tempdir().unwrap();
    std::fs::set_permissions(allocation.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(allocation.path()).unwrap();
    assert!(first_pki.stage(&root, &second).is_err());
    let mut staged = second_pki.stage(&root, &second).unwrap();
    staged.release_inputs().unwrap();
}

#[test]
fn foreign_workspace_application_scope_and_unapproved_origins_receive_no_secret_material() {
    let registry = OciBrowserPkiRegistry::new(vec![registration(
        OciBrowserPkiScopeAuthorization::Exact(request().binding.scope),
        false,
    )])
    .unwrap();
    for field in 0..4 {
        let mut foreign = request();
        match field {
            0 => foreign.binding.workspace_id = "foreign".into(),
            1 => foreign.binding.application_id = "foreign".into(),
            2 => {
                foreign.binding.scope = BrowserScope::Conversation {
                    id: "foreign".into(),
                }
            }
            _ => {
                foreign.options.allowed_origins =
                    vec![BrowserOrigin::parse("https://foreign.example").unwrap()]
            }
        }
        assert!(registry.enroll(&foreign).unwrap().is_none());
    }
}

#[test]
fn profile_global_ca_trust_cannot_extend_to_an_unapproved_origin_in_the_same_envelope() {
    let registry = OciBrowserPkiRegistry::new(vec![registration(
        OciBrowserPkiScopeAuthorization::FutureConversationsAndWorkflows,
        true,
    )])
    .unwrap();
    assert!(registry.has_private_ca());
    assert!(registry.has_client_identities());
    let mut mixed = request();
    mixed
        .options
        .allowed_origins
        .push(BrowserOrigin::parse("https://unapproved.example").unwrap());
    assert!(matches!(
        registry.enroll(&mixed),
        Err(BrowserDriverError::Denied)
    ));
    let mut approved = request();
    approved.binding.scope = BrowserScope::Workflow {
        id: "future-workflow".into(),
    };
    assert!(registry.enroll(&approved).unwrap().is_some());
}

#[test]
fn conversations_only_consent_and_ambiguous_imports_fail_closed() {
    let registry = OciBrowserPkiRegistry::new(vec![registration(
        OciBrowserPkiScopeAuthorization::FutureConversations,
        false,
    )])
    .unwrap();
    let mut workflow = request();
    workflow.binding.scope = BrowserScope::Workflow {
        id: "workflow".into(),
    };
    assert!(registry.enroll(&workflow).unwrap().is_none());
    let ambiguous = OciBrowserPkiRegistry::new(vec![
        registration(OciBrowserPkiScopeAuthorization::FutureConversations, false),
        registration(
            OciBrowserPkiScopeAuthorization::FutureConversationsAndWorkflows,
            false,
        ),
    ])
    .unwrap();
    assert!(matches!(
        ambiguous.enroll(&request()),
        Err(BrowserDriverError::Denied)
    ));
}

#[test]
fn approved_ca_only_origin_receives_no_identity_capability_or_staged_secret_files() {
    let public = BrowserOrigin::parse("https://public.example").unwrap();
    let mut imported = registration(OciBrowserPkiScopeAuthorization::FutureConversations, true);
    imported.authorization.origins.push(public.clone());
    let registry = OciBrowserPkiRegistry::new(vec![imported]).unwrap();
    let mut public_request = request();
    public_request.options.allowed_origins = vec![public];
    let material = registry.enroll(&public_request).unwrap().unwrap();
    assert!(material.has_private_ca());
    assert!(!material.has_client_identities());
    assert!(!material.has_client_identities_for(&public_request.options.allowed_origins));
    let allocation = tempfile::tempdir().unwrap();
    std::fs::set_permissions(allocation.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = ConfinedRoot::bind(allocation.path()).unwrap();
    let mut staged = material.stage(&root, &public_request).unwrap();
    let bootstrap = serde_json::to_value(staged.bootstrap()).unwrap();
    assert!(bootstrap["identities"].as_array().unwrap().is_empty());
    assert!(bootstrap["bindings"].as_array().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(allocation.path().join("pki"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from("ca-0.der")]
    );
    staged.release_inputs().unwrap();

    let imported = registration(OciBrowserPkiScopeAuthorization::FutureConversations, false);
    let fixed = OciBrowserPki::new(
        request().binding,
        imported.cas,
        imported.identities,
        imported.bindings,
    )
    .unwrap();
    assert!(fixed.has_client_identities());
    assert!(!fixed.has_client_identities_for(&public_request.options.allowed_origins));
    assert!(fixed.has_client_identities_for(&request().options.allowed_origins));
}

#[test]
fn native_policy_digest_covers_public_consent_and_leaf_pins_without_secret_bytes() {
    let first = OciBrowserPkiRegistry::new(vec![registration(
        OciBrowserPkiScopeAuthorization::FutureConversations,
        false,
    )])
    .unwrap();
    let mut different_secret =
        registration(OciBrowserPkiScopeAuthorization::FutureConversations, false);
    different_secret.identities = vec![
        OciBrowserIdentity::new(
            Zeroizing::new(b"different-encrypted-pfx".to_vec()),
            HostSecret::new("different-password").unwrap(),
        )
        .unwrap(),
    ];
    let same_policy = OciBrowserPkiRegistry::new(vec![different_secret]).unwrap();
    assert_eq!(first.policy_digest(), same_policy.policy_digest());
    let mut different_pin =
        registration(OciBrowserPkiScopeAuthorization::FutureConversations, false);
    different_pin.bindings[0].fingerprint_sha256 = "b".repeat(64);
    assert_ne!(
        first.policy_digest(),
        OciBrowserPkiRegistry::new(vec![different_pin])
            .unwrap()
            .policy_digest()
    );
    let workflows = OciBrowserPkiRegistry::new(vec![registration(
        OciBrowserPkiScopeAuthorization::FutureWorkflows,
        false,
    )])
    .unwrap();
    assert_ne!(first.policy_digest(), workflows.policy_digest());
}
