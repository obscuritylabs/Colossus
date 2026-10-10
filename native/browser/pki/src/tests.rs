use super::*;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
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
