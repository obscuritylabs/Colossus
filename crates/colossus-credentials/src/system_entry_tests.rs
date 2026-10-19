use super::*;

#[cfg(target_os = "linux")]
#[path = "system_entry_linux_tests.rs"]
mod linux;

#[test]
fn missing_entries_are_distinct_from_native_access_failures() {
    assert!(release_secret(Ok(None)).unwrap().is_none());
    assert_eq!(
        release_secret(Err(CredentialError::Unavailable)),
        Err(CredentialError::Unavailable)
    );
    assert_eq!(
        CredentialError::Unavailable.to_string(),
        "credential store is unavailable"
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn platform_response_text_never_enters_the_credential_error() {
    let failure = keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other(
        "private native response",
    )));
    assert_eq!(safe_error(failure), CredentialError::Unavailable);
    assert_eq!(
        safe_error(keyring_core::Error::BadEncoding(b"private bytes".to_vec())),
        CredentialError::Corrupt
    );
}

#[test]
fn binary_native_secrets_remain_exact_and_bounded() {
    let bytes = vec![0, 255, 1, 128];
    assert_eq!(
        release_secret(Ok(Some(Zeroizing::new(bytes.clone()))))
            .unwrap()
            .unwrap()
            .as_slice(),
        bytes
    );
    assert_eq!(
        release_secret(Ok(Some(Zeroizing::new(vec![0; MAX_SECRET_BYTES + 1])))),
        Err(CredentialError::Oversized)
    );
}

#[test]
fn invalid_entry_identities_fail_before_native_store_connection() {
    for (service, account) in [
        ("", "account"),
        ("service", ""),
        ("service\0secret", "account"),
    ] {
        assert!(matches!(
            SystemCredentialEntry::new(service, account),
            Err(CredentialError::InvalidInput)
        ));
    }
    assert!(matches!(
        SystemCredentialEntry::new("service", &"x".repeat(MAX_ID_BYTES + 1)),
        Err(CredentialError::InvalidInput)
    ));
}
