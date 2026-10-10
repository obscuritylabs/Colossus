use super::*;

fn self_identity_fixture() -> (DarwinProcessSnapshot, u32) {
    let identity = DarwinProcessIdentity::bind(std::process::id()).expect("genuine self token");
    let audit_session = identity.audit_session_id();
    (
        DarwinProcessSnapshot {
            uid: identity.real_uid(),
            audit_session: None,
            identities: vec![identity],
        },
        audit_session,
    )
}

#[test]
fn audit_session_filter_preserves_genuine_matching_identity() {
    let (snapshot, audit_session) = self_identity_fixture();
    let expected = snapshot.identities()[0].clone();
    let filtered = snapshot.for_audit_session(audit_session).unwrap();
    assert_eq!(filtered.identities(), &[expected]);
    assert!(filtered.empty_evidence().is_none());
}

#[test]
fn audit_session_filter_rejects_placeholder_identifiers() {
    for invalid in [0, u32::MAX] {
        let (snapshot, _) = self_identity_fixture();
        assert_eq!(
            snapshot.for_audit_session(invalid).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn restricted_census_cannot_be_relabelled_as_another_empty_domain() {
    let (snapshot, audit_session) = self_identity_fixture();
    let filtered = snapshot.for_audit_session(audit_session).unwrap();
    assert_eq!(
        filtered
            .for_audit_session(audit_session + 1)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
}
