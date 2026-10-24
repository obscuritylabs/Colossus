use super::*;

#[test]
fn owner_resolution_is_exact_and_never_interprets_a_connection_id_as_an_address() {
    let replica = "0123456789abcdef0123456789abcdef";
    let connection = format!("{replica}-fedcba9876543210fedcba9876543210");
    assert!(is_local(replica, &connection));
    assert!(!is_local("fedcba9876543210fedcba9876543210", &connection));
    for malformed in [
        "https://127.0.0.1:8443",
        "replica-connection",
        "-",
        "",
        "0123456789abcdef0123456789abcdef-f",
    ] {
        assert!(replica_owner(malformed).is_none());
    }
}

#[test]
fn ordinary_enrollment_is_insufficient_without_exact_relay_authentication() {
    let expected = "a".repeat(64);
    assert!(!identity::token_matches(&expected, None));
    assert!(!identity::token_matches(&expected, Some("")));
    assert!(!identity::token_matches(&expected, Some(&"b".repeat(64))));
    assert!(identity::token_matches(&expected, Some(&expected)));
}
