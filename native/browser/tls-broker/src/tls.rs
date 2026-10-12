use crate::authority::Authority;
use crate::client_identity::ClientIdentity;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::{ClientConfig, RootCertStore, ServerConfig, pki_types::PrivateKeyDer};
use sha2::{Digest as _, Sha256};
use std::{sync::Arc, time::Duration};

pub(crate) struct Tls {
    pub downstream: Arc<ServerConfig>,
    pub upstream: Arc<ClientConfig>,
    pub public_ca: Vec<u8>,
    pub public_leaf: Vec<u8>,
    pub public_leaf_sha256: [u8; 32],
    pub identity: Option<Arc<ClientIdentity>>,
}

pub(crate) fn prepare(
    authority: &Authority,
    ca_der: Vec<Vec<u8>>,
    lifetime: Duration,
    identity: Option<Arc<ClientIdentity>>,
) -> Result<Tls, ()> {
    if ca_der.is_empty()
        || ca_der.len() > 8
        || ca_der.iter().map(Vec::len).sum::<usize>() > 1024 * 1024
    {
        return Err(());
    }
    let mut roots = RootCertStore::empty();
    for root in ca_der {
        let der = colossus_native_browser_pki::ca_der(&root).map_err(|_| ())?;
        roots.add(der.into()).map_err(|_| ())?;
    }
    let mut provider = rustls::crypto::ring::default_provider();
    if identity.is_some() {
        // The native signer accepts only TLS1.3 SHA256 transcript construction.
        provider.cipher_suites.retain(|suite| {
            matches!(
                suite.suite(),
                rustls::CipherSuite::TLS13_AES_128_GCM_SHA256
                    | rustls::CipherSuite::TLS13_CHACHA20_POLY1305_SHA256
            )
        });
    }
    let provider = Arc::new(provider);
    let identity_versions = [&rustls::version::TLS13];
    let mut upstream = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(if identity.is_some() {
            &identity_versions
        } else {
            rustls::DEFAULT_VERSIONS
        })
        .map_err(|_| ())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    upstream.alpn_protocols = vec![b"http/1.1".to_vec()];
    // No resumption, tickets or 0-RTT can carry an earlier admission forward.
    upstream.resumption = rustls::client::Resumption::disabled();
    let now = time::OffsetDateTime::now_utc();
    let not_before = now.checked_sub(time::Duration::minutes(5)).ok_or(())?;
    let not_after = now
        .checked_add(time::Duration::try_from(lifetime).map_err(|_| ())?)
        .ok_or(())?;
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|_| ())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.not_before = not_before;
    params.not_after = not_after;
    let ca_key = KeyPair::generate().map_err(|_| ())?;
    let ca = params.self_signed(&ca_key).map_err(|_| ())?;
    let issuer = Issuer::new(params, ca_key);
    let mut leaf = CertificateParams::new(vec![authority.host.clone()]).map_err(|_| ())?;
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.not_before = not_before;
    leaf.not_after = not_after;
    let key = KeyPair::generate().map_err(|_| ())?;
    let cert = leaf.signed_by(&key, &issuer).map_err(|_| ())?;
    let public_leaf = cert.der().to_vec();
    let public_leaf_sha256 = Sha256::digest(&public_leaf).into();
    let mut downstream = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| ())?
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .map_err(|_| ())?;
    downstream.alpn_protocols = vec![b"http/1.1".to_vec()];
    downstream.send_tls13_tickets = 0;
    downstream.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    Ok(Tls {
        downstream: Arc::new(downstream),
        upstream: Arc::new(upstream),
        public_ca: ca.der().to_vec(),
        public_leaf,
        public_leaf_sha256,
        identity,
    })
}
