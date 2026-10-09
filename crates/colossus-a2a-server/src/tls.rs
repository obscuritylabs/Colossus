use axum::serve::Listener;
use futures::{StreamExt, stream::FuturesUnordered};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::io::Read;
use std::{future::Future, net::SocketAddr, path::Path, pin::Pin, sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use zeroize::Zeroizing;

type Handshake = Pin<Box<dyn Future<Output = Option<(TlsStream<TcpStream>, SocketAddr)>> + Send>>;
pub(crate) struct TlsListener {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
    pending: FuturesUnordered<Handshake>,
}
impl TlsListener {
    pub(crate) fn new(
        tcp: TcpListener,
        certificate: &Path,
        key: &Path,
    ) -> Result<Self, &'static str> {
        if !certificate.is_absolute() {
            return Err("A2A certificate path must be absolute");
        }
        let mut certificate_bytes = Vec::new();
        std::fs::File::open(certificate)
            .map_err(|_| "A2A server certificate unavailable")?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut certificate_bytes)
            .map_err(|_| "A2A server certificate unavailable")?;
        if certificate_bytes.len() > 1024 * 1024 {
            return Err("A2A server certificate exceeds its bound");
        }
        let key = Zeroizing::new(crate::config::private_file(key, 1024 * 1024)?);
        let certificates = CertificateDer::pem_slice_iter(&certificate_bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "invalid A2A certificate")?;
        let key = PrivateKeyDer::from_pem_slice(&key).map_err(|_| "invalid A2A private key")?;
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(|_| "TLS 1.3 unavailable")?
                .with_no_client_auth()
                .with_single_cert(certificates, key)
                .map_err(|_| "invalid A2A TLS identity")?;
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(Self {
            tcp,
            acceptor: TlsAcceptor::from(Arc::new(config)),
            pending: FuturesUnordered::new(),
        })
    }
}
impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            tokio::select! {
                accepted = self.tcp.accept(), if self.pending.len() < 64 => {
                    if let Ok((socket, address)) = accepted {
                        let acceptor = self.acceptor.clone();
                        self.pending.push(Box::pin(async move { tokio::time::timeout(Duration::from_secs(5), acceptor.accept(socket)).await.ok().and_then(Result::ok).map(|socket| (socket, address)) }));
                    } else { tokio::time::sleep(Duration::from_millis(250)).await; }
                }
                completed = self.pending.next(), if !self.pending.is_empty() => {
                    if let Some(Some(connection)) = completed { return connection; }
                }
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}
