use super::*;
use colossus_contracts::{
    BrowserDocumentId, BrowserMode, BrowserOpenOptions, BrowserSessionId, BrowserTabId,
};
fn id(byte: char) -> MacosBrowserPkiId {
    MacosBrowserPkiId::new(byte.to_string().repeat(64)).unwrap()
}
fn origin() -> BrowserOrigin {
    BrowserOrigin::parse("https://reviewed.example").unwrap()
}
fn request() -> BrowserDriverOpenRequest {
    BrowserDriverOpenRequest {
        binding: BrowserSessionBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: "app".into(),
            scope: BrowserScope::Conversation {
                id: "conversation".into(),
            },
        },
        run_id: Some("run".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", "a".repeat(32))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", "b".repeat(32))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", "c".repeat(32))).unwrap(),
        options: BrowserOpenOptions {
            mode: BrowserMode::Embedded,
            allowed_origins: vec![origin()],
            initial_url: None,
            profile: Default::default(),
        },
    }
}
fn registration(scope: MacosBrowserPkiScopeAuthorization) -> MacosBrowserPkiRegistration {
    MacosBrowserPkiRegistration::new(
        MacosBrowserPkiAuthorization {
            workspace_id: "workspace".into(),
            application_id: "app".into(),
            scope,
            origins: vec![origin()],
        },
        vec![id('a')],
        vec![MacosBrowserClientIdentity {
            origin: origin(),
            identity_id: id('b'),
            fingerprint_sha256: [7; 32],
        }],
    )
    .unwrap()
}
fn catalog() -> NativeCatalog {
    NativeCatalog {
        cas: BTreeMap::from([(id('a'), vec![origin()])]),
        identities: BTreeMap::from([(
            id('b'),
            NativeIdentity {
                fingerprint: [7; 32],
                origins: vec![origin()],
            },
        )]),
    }
}
#[test]
fn references_are_bound_to_signed_catalog_then_each_actual_core_owner() {
    let registry = MacosBrowserPkiRegistry::new(vec![registration(
        MacosBrowserPkiScopeAuthorization::FutureConversations,
    )])
    .unwrap()
    .bind(&catalog())
    .unwrap();
    let first = request();
    let selected = registry.enroll(&first).unwrap().unwrap();
    assert_eq!(selected.binding, first.binding);
    assert_eq!(selected.cas, vec![id('a')]);
    assert_eq!(selected.identities[0].fingerprint_sha256, [7; 32]);
    let mut second = request();
    second.binding.runtime_id = "second-runtime".into();
    second.binding.scope = BrowserScope::Conversation {
        id: "future".into(),
    };
    let next = registry.enroll(&second).unwrap().unwrap();
    assert_eq!(next.binding, second.binding);
    assert_ne!(next.binding, selected.binding);
}
#[test]
fn foreign_owner_scope_and_origin_never_receive_native_selections() {
    let registry = MacosBrowserPkiRegistry::new(vec![registration(
        MacosBrowserPkiScopeAuthorization::Exact(request().binding.scope),
    )])
    .unwrap()
    .bind(&catalog())
    .unwrap();
    for changed in 0..4 {
        let mut foreign = request();
        match changed {
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
        };
        assert!(registry.enroll(&foreign).unwrap().is_none());
    }
    let mut mixed = request();
    mixed
        .options
        .allowed_origins
        .push(BrowserOrigin::parse("https://unreviewed.example").unwrap());
    assert!(registry.enroll(&mixed).is_err());
}
#[test]
fn catalog_mismatch_fingerprint_substitution_and_ambiguous_review_fail_before_enrollment() {
    let mut wrong = catalog();
    wrong.identities.insert(
        id('b'),
        NativeIdentity {
            fingerprint: [8; 32],
            origins: vec![origin()],
        },
    );
    assert!(
        MacosBrowserPkiRegistry::new(vec![registration(
            MacosBrowserPkiScopeAuthorization::FutureConversations
        )])
        .unwrap()
        .bind(&wrong)
        .is_err()
    );
    let mut missing = catalog();
    missing.cas.clear();
    assert!(
        MacosBrowserPkiRegistry::new(vec![registration(
            MacosBrowserPkiScopeAuthorization::FutureConversations
        )])
        .unwrap()
        .bind(&missing)
        .is_err()
    );
    let registry = MacosBrowserPkiRegistry::new(vec![
        registration(MacosBrowserPkiScopeAuthorization::FutureConversations),
        registration(MacosBrowserPkiScopeAuthorization::Exact(
            request().binding.scope,
        )),
    ])
    .unwrap()
    .bind(&catalog())
    .unwrap();
    assert!(registry.enroll(&request()).is_err());
    let mut narrowed = catalog();
    narrowed.identities.get_mut(&id('b')).unwrap().origins =
        vec![BrowserOrigin::parse("https://different-native.example").unwrap()];
    assert!(
        MacosBrowserPkiRegistry::new(vec![registration(
            MacosBrowserPkiScopeAuthorization::FutureConversations
        )])
        .unwrap()
        .bind(&narrowed)
        .is_err()
    );
    let mut narrowed_ca = catalog();
    narrowed_ca.cas.insert(
        id('a'),
        vec![BrowserOrigin::parse("https://different-native.example").unwrap()],
    );
    assert!(
        MacosBrowserPkiRegistry::new(vec![registration(
            MacosBrowserPkiScopeAuthorization::FutureConversations
        )])
        .unwrap()
        .bind(&narrowed_ca)
        .is_err()
    );
}
#[test]
fn ids_paths_malformed_scope_and_unreviewed_future_workflows_are_denied() {
    for value in [
        "0".repeat(64),
        "../identity".into(),
        "A".repeat(64),
        "a".repeat(63),
    ] {
        assert!(MacosBrowserPkiId::new(value).is_err());
    }
    let registry = MacosBrowserPkiRegistry::new(vec![registration(
        MacosBrowserPkiScopeAuthorization::FutureConversations,
    )])
    .unwrap()
    .bind(&catalog())
    .unwrap();
    let mut workflow = request();
    workflow.binding.scope = BrowserScope::Workflow {
        id: "workflow".into(),
    };
    assert!(registry.enroll(&workflow).unwrap().is_none());
    workflow.binding.scope = BrowserScope::Conversation { id: "".into() };
    assert!(registry.enroll(&workflow).is_err());
}
