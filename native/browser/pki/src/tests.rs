use super::*;
use rcgen::{
    BasicConstraints, CertificateParams, CustomExtension, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};

fn certificate(params: CertificateParams) -> Vec<u8> {
    params
        .self_signed(&KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec()
}

#[test]
fn trust_requires_one_valid_ca_and_client_use_requires_valid_leaf_usage() {
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let der = certificate(ca);
    assert_eq!(ca_der(&der).unwrap(), der);
    assert_eq!(identity_validity(&der).unwrap(), None);
    let mut leaf = CertificateParams::new(Vec::<String>::new()).unwrap();
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let der = certificate(leaf.clone());
    assert!(identity_validity(&der).unwrap().is_some());
    assert!(ca_der(&der).is_err());
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    assert_eq!(identity_validity(&certificate(leaf.clone())).unwrap(), None);
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    leaf.not_before = rcgen::date_time_ymd(2000, 1, 1);
    leaf.not_after = rcgen::date_time_ymd(2001, 1, 1);
    assert_eq!(identity_validity(&certificate(leaf)).unwrap(), None);
    let mut extra = der;
    extra.push(0);
    assert!(identity_validity(&extra).is_err());
    assert!(ca_der(b"malformed").is_err());
}

#[test]
fn identity_eligibility_preserves_optional_usage_and_excludes_wrong_usage() {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::ExplicitNoCa;
    assert!(
        identity_validity(&certificate(params.clone()))
            .unwrap()
            .is_some()
    );
    params.key_usages = vec![KeyUsagePurpose::KeyEncipherment];
    assert_eq!(
        identity_validity(&certificate(params.clone())).unwrap(),
        None
    );
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    assert!(
        identity_validity(&certificate(params.clone()))
            .unwrap()
            .is_some()
    );
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    assert_eq!(
        identity_validity(&certificate(params.clone())).unwrap(),
        None
    );
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    assert!(identity_validity(&certificate(params)).unwrap().is_some());
}

#[test]
fn identity_eligibility_retains_native_validity_interval_and_rejects_future_leaf() {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    let expected = (
        params.not_before.unix_timestamp(),
        params.not_after.unix_timestamp(),
    );
    assert_eq!(
        identity_validity(&certificate(params.clone())).unwrap(),
        Some(expected)
    );
    params.not_before = rcgen::date_time_ymd(3000, 1, 1);
    assert_eq!(identity_validity(&certificate(params)).unwrap(), None);
}

#[test]
fn malformed_identity_input_and_duplicate_usage_extensions_fail_the_request() {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params
        .custom_extensions
        .push(CustomExtension::from_oid_content(
            &[2, 5, 29, 15],
            vec![0x03, 0x02, 0x07, 0x80],
        ));
    assert!(identity_validity(&certificate(params)).is_err());
    for der in [&[][..], &b"malformed"[..], &vec![0; 65_537][..]] {
        assert!(identity_validity(der).is_err());
        assert!(identity_label(der).is_err());
    }
    let mut extra = certificate(CertificateParams::new(Vec::<String>::new()).unwrap());
    extra.push(0);
    assert!(identity_label(&extra).is_err());
}

fn subject_certificate(subject: &str) -> Vec<u8> {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Example organization");
    params.distinguished_name.push(DnType::CommonName, subject);
    certificate(params)
}

#[test]
fn identity_label_prefers_public_common_name_and_retains_literal_plain_text() {
    let subject = "Renée 王 <client> \"quoted\" \\ identity";
    let der = subject_certificate(subject);
    assert_eq!(identity_label(&der).unwrap(), subject);
    assert!(identity_validity(&der).unwrap().is_some());
}

#[test]
fn identity_label_strips_controls_direction_overrides_and_extra_whitespace() {
    let der = subject_certificate(
        " \tAlice\0\n\u{85}\u{061c}\u{200e}\u{202e}\u{2066} Admin\u{2069}\u{feff}  ",
    );
    let label = identity_label(&der).unwrap();
    assert_eq!(label, "Alice Admin");
    assert!(!label.chars().any(char::is_control));
}

#[test]
fn identity_label_falls_back_to_public_distinguished_name_then_fixed_text() {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Example organization");
    params
        .distinguished_name
        .push(DnType::OrganizationalUnitName, "Client team");
    let label = identity_label(&certificate(params.clone())).unwrap();
    assert_eq!(label, "O=Example organization, OU=Client team");
    params.distinguished_name = DistinguishedName::new();
    assert_eq!(
        identity_label(&certificate(params)).unwrap(),
        "Client certificate"
    );
}

#[test]
fn identity_label_caps_json_bytes_and_truncates_only_at_unicode_boundaries() {
    for subject in ["é王😀".repeat(200), "\"\\".repeat(200), "a ".repeat(200)] {
        let label = identity_label(&subject_certificate(&subject)).unwrap();
        let json = serde_json::to_string(&label).unwrap();
        assert!(json.len() <= 100, "{} JSON bytes", json.len());
        assert_eq!(serde_json::from_str::<String>(&json).unwrap(), label);
        assert!(label.ends_with('…'));
        assert_eq!(label.trim(), label);
        assert!(!label[..label.len() - '…'.len_utf8()].ends_with(' '));
    }
    let label = identity_label(&subject_certificate(&"a".repeat(98))).unwrap();
    assert_eq!(serde_json::to_string(&label).unwrap().len(), 100);
    assert!(!label.ends_with('…'));
}

#[test]
fn identity_label_does_not_grant_eligibility_or_conflate_matching_subjects() {
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.distinguished_name
        .push(DnType::CommonName, "Example client");
    let ca_der = certificate(ca);
    assert_eq!(identity_label(&ca_der).unwrap(), "Example client");
    assert_eq!(identity_validity(&ca_der).unwrap(), None);
    let first = subject_certificate("Example client");
    let second = subject_certificate("Example client");
    assert_eq!(
        identity_label(&first).unwrap(),
        identity_label(&second).unwrap()
    );
    assert_ne!(fingerprint(&first), fingerprint(&second));
}
