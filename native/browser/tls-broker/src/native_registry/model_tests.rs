use super::*;
use crate::{ReviewedClientIdentity, registry_metadata::*};
fn catalog() -> RegistryMetadata {
    RegistryMetadata {
        schema_version: 1,
        purpose: "colossus-native-browser-reviewed-pki-registry".into(),
        cas: vec![RegisteredCa {
            id: hex::encode([2; 32]),
            slot: 0,
            sha256: hex::encode([3; 32]),
            origins: vec!["https://fixture.test".into()],
        }],
        identities: vec![RegisteredIdentity {
            id: hex::encode([4; 32]),
            slot: 0,
            leaf_sha256: hex::encode([5; 32]),
            pkcs12_sha256: hex::encode([6; 32]),
            password_sha256: hex::encode([7; 32]),
            ca_ids: vec![hex::encode([2; 32])],
            origins: vec!["https://fixture.test".into()],
        }],
        reviews: vec![ReviewedScope {
            workspace_id: "workspace".into(),
            application_id: "application".into(),
            scope: ScopeConsent::FutureConversations {},
            origins: vec!["https://fixture.test".into()],
            ca_ids: vec![hex::encode([2; 32])],
            client_id: Some(hex::encode([4; 32])),
        }],
    }
}
fn selection() -> PkiSelection {
    PkiSelection {
        scope_binding: [1; 32],
        ca_ids: vec![[2; 32]],
        client: Some(ReviewedClientIdentity {
            identity_id: [4; 32],
            leaf_sha256: [5; 32],
        }),
    }
}
#[test]
fn exact_reviewed_catalog_selects_only_registered_slots() {
    let plan = resolve(&catalog(), "https://fixture.test", &selection()).unwrap();
    assert_eq!(plan.ca_indices, vec![0]);
    assert_eq!(plan.identity_index, Some(0));
    // Correlation has no secret native authority: a different nonzero digest
    // does not alter the independently admitted origin/catalog decision.
    let mut next = selection();
    next.scope_binding = [8; 32];
    assert!(resolve(&catalog(), "https://fixture.test", &next).is_ok());
}
#[test]
fn foreign_origin_id_leaf_issuer_and_partial_review_fail_closed() {
    assert!(resolve(&catalog(), "https://foreign.test", &selection()).is_err());
    let mut next = selection();
    next.client.as_mut().unwrap().leaf_sha256 = [8; 32];
    assert!(resolve(&catalog(), "https://fixture.test", &next).is_err());
    next = selection();
    next.client.as_mut().unwrap().identity_id = [8; 32];
    assert!(resolve(&catalog(), "https://fixture.test", &next).is_err());
    next = selection();
    next.ca_ids.clear();
    assert!(resolve(&catalog(), "https://fixture.test", &next).is_err());
    next = selection();
    next.ca_ids.push([8; 32]);
    assert!(resolve(&catalog(), "https://fixture.test", &next).is_err());
    let mut changed = catalog();
    changed.identities[0].ca_ids = vec![hex::encode([8; 32])];
    assert!(resolve(&changed, "https://fixture.test", &selection()).is_err());
    let mut changed = catalog();
    changed.cas[0].origins = vec!["https://foreign.test".into()];
    assert!(resolve(&changed, "https://fixture.test", &selection()).is_err());
}
