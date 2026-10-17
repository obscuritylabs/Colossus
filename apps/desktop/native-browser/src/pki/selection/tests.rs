use super::*;
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose};

fn leaf(mut params: CertificateParams) -> Vec<u8> {
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    params
        .self_signed(&KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec()
}

#[test]
fn review_matches_only_exact_native_origin_and_certificate_fingerprint() {
    let der = leaf(CertificateParams::new(Vec::<String>::new()).unwrap());
    let review = IdentityRequest::new(7, 9, "https://example.com", &[&der]).unwrap();
    assert_eq!(review.candidate_index(&fingerprint(&der)), Some(0));
    assert_eq!(review.candidate_index(&"a".repeat(64)), None);
    for origin in [
        "http://example.com",
        "https://example.com/",
        "https://example.com:443",
        "https://user@example.com",
        "https://example.com?x=1",
        "https://EXAMPLE.com",
        "https://example.com/path",
    ] {
        assert!(
            IdentityRequest::new(7, 9, origin, &[&der]).is_err(),
            "{origin}"
        );
    }
    assert!(IdentityRequest::new(7, 9, "https://example.com:8443", &[&der]).is_ok());
}

#[test]
fn invalid_expired_ca_and_wrong_usage_candidates_do_not_select_keys() {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = leaf(params);
    assert!(IdentityRequest::new(1, 1, "https://example.com", &[&ca]).is_err());
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    params.not_after = rcgen::date_time_ymd(2001, 1, 1);
    let expired = leaf(params);
    assert!(IdentityRequest::new(1, 1, "https://example.com", &[&expired]).is_err());
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let wrong_usage = params.self_signed(&KeyPair::generate().unwrap()).unwrap();
    assert!(
        IdentityRequest::new(1, 1, "https://example.com", &[wrong_usage.der().as_ref()]).is_err()
    );
    assert!(IdentityRequest::new(1, 1, "https://example.com", &[b"malformed"]).is_err());
}

#[test]
fn reviews_expire_and_preserve_original_candidate_indexes_without_ambiguity() {
    let valid = leaf(CertificateParams::new(Vec::<String>::new()).unwrap());
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let excluded = leaf(params);
    let mut review =
        IdentityRequest::new(7, 9, "https://example.com", &[&excluded, &valid]).unwrap();
    assert_eq!(review.candidate_index(&fingerprint(&valid)), Some(1));
    review.candidates[0].not_after = 1;
    assert_eq!(review.candidate_index(&fingerprint(&valid)), None);
    review.expires_at = Instant::now();
    assert!(review.is_expired());
    assert_eq!(review.candidate_index(&fingerprint(&valid)), None);
    assert!(IdentityRequest::new(7, 9, "https://example.com", &[&valid, &valid]).is_err());
    assert!(
        IdentityRequest::new(7, 9, "https://example.com", &vec![valid.as_slice(); 65]).is_err()
    );
}
