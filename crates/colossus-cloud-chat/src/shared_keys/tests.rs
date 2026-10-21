use super::*;
use colossus_contracts::SecureAnchorStatus;

fn config(endpoint: &str) -> SharedChatKeyConfig {
    SharedChatKeyConfig {
        journal_variable: "CHAT_JOURNAL_KEY".into(),
        signing_variable: "CHAT_SIGNING_KEY".into(),
        anchor_endpoint: endpoint.into(),
        anchor_bearer_variable: "CHAT_ANCHOR_TOKEN".into(),
        anchor_ca_pem_path: None,
        allow_loopback_http: false,
    }
}

#[test]
fn shared_keys_are_bound_to_namespace_and_purpose() {
    let master = [7; 32];
    let first = keys::derive(&master, b"journal", &"a".repeat(64)).unwrap();
    assert_eq!(
        *first,
        *keys::derive(&master, b"journal", &"a".repeat(64)).unwrap()
    );
    assert_ne!(
        *first,
        *keys::derive(&master, b"journal", &"b".repeat(64)).unwrap()
    );
    assert_ne!(
        *first,
        *keys::derive(&master, b"checkpoint", &"a".repeat(64)).unwrap()
    );
}

#[test]
fn authority_transport_requires_tls_or_explicit_loopback_fixture() {
    assert!(
        config("https://anchors.example/v1/anchors")
            .validate()
            .is_ok()
    );
    for invalid in [
        "http://anchors.example/v1/anchors",
        "http://127.0.0.1/v1/anchors",
        "https://user:password@anchors.example",
        "https://anchors.example?secret=value",
        "https://anchors.example/#fragment",
        "file:///tmp/anchors",
    ] {
        assert!(config(invalid).validate().is_err());
    }
    let mut local = config("http://127.0.0.1/v1/anchors");
    local.allow_loopback_http = true;
    assert!(local.validate().is_ok());
    local.anchor_endpoint = "http://192.0.2.1/v1/anchors".into();
    assert!(local.validate().is_err());
    local.anchor_endpoint = "https://anchors.example".into();
    local.journal_variable = "INVALID-SECRET-REF".into();
    assert!(local.validate().is_err());
}

#[test]
fn independent_attestation_rejects_rollback_and_divergence() {
    let anchor = SecureAnchor {
        format_version: 2,
        sequence: 40,
        hash: "a".repeat(64),
        status: SecureAnchorStatus::Verified,
        verification_profile: None,
    };
    assert!(validate_shared_anchor_advance(None, &anchor).is_ok());
    let mut next = anchor.clone();
    next.sequence = 39;
    assert!(validate_shared_anchor_advance(Some(&anchor), &next).is_err());
    next.sequence = 40;
    next.hash = "b".repeat(64);
    assert!(validate_shared_anchor_advance(Some(&anchor), &next).is_err());
    next.sequence = 41;
    assert!(validate_shared_anchor_advance(Some(&anchor), &next).is_ok());
}
