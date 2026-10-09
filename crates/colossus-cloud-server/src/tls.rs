use crate::config::Config;
use futures::{Stream, StreamExt};
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::WebPkiClientVerifier,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tokio_stream::wrappers::TcpListenerStream;
use zeroize::Zeroizing;

pub(crate) fn incoming(
    listener: TcpListener,
    config: &Config,
    ca_pem: &str,
) -> Result<impl Stream<Item = Result<TlsStream<TcpStream>, std::io::Error>>, &'static str> {
    let certificate =
        std::fs::read(&config.server_certificate).map_err(|_| "cannot load server certificate")?;
    let key =
        Zeroizing::new(std::fs::read(&config.server_key).map_err(|_| "cannot load server key")?);
    let chain = CertificateDer::pem_slice_iter(&certificate)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid server certificate")?;
    let key = PrivateKeyDer::from_pem_slice(&key).map_err(|_| "invalid server key")?;
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(ca_pem.as_bytes()) {
        roots
            .add(certificate.map_err(|_| "invalid client CA")?)
            .map_err(|_| "invalid client CA")?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|_| "invalid client CA")?;
    let mut tls = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| "TLS 1.3 unavailable")?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(|_| "invalid TLS identity")?;
    tls.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    Ok(TcpListenerStream::new(listener)
        .map(move |socket| {
            let acceptor = acceptor.clone();
            async move {
                match socket {
                    Ok(socket) => timeout(Duration::from_secs(5), acceptor.accept(socket))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .map(Ok),
                    Err(error) => Some(Err(error)),
                }
            }
        })
        .buffer_unordered(64)
        .filter_map(|result| async move { result }))
}
