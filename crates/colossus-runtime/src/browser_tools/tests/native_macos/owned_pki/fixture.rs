//! Synthetic mTLS server requires the exact reviewed public client leaf.
use super::*;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{RootCertStore, server::WebPkiClientVerifier};
use std::{path::Path, sync::atomic::AtomicUsize};
use tokio::{net::TcpListener, sync::oneshot, task::JoinSet};
use tokio_rustls::TlsAcceptor;

pub(super) struct Fixture {
    pub requests: Arc<AtomicUsize>,
    pub reviewed_clients: Arc<AtomicUsize>,
    pub foreign_connections: Arc<AtomicUsize>,
    pub wrong_clients: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start(origin: &str, inputs: &Path) -> Self {
        let port = origin
            .strip_prefix("https://127.0.0.1:")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| !matches!(port, 0 | 443))
            .expect("one signed numeric HTTPS fixture");
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let foreign = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let foreign_origin = format!("https://{}", foreign.local_addr().unwrap());
        let leaf = CertificateDer::from(std::fs::read(inputs.join("fixture-leaf.der")).unwrap());
        let key = zeroize::Zeroizing::new(std::fs::read(inputs.join("fixture-key.pk8")).unwrap());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec()));
        let reviewed = std::fs::read(inputs.join("fixture-client.der")).unwrap();
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(
                std::fs::read(inputs.join("fixture-ca.der")).unwrap(),
            ))
            .unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
                .build()
                .unwrap();
        let tls = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![leaf], key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        let requests = Arc::new(AtomicUsize::new(0));
        let reviewed_clients = Arc::new(AtomicUsize::new(0));
        let foreign_connections = Arc::new(AtomicUsize::new(0));
        let wrong_clients = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        let admitted = Arc::clone(&reviewed_clients);
        let foreign_counted = Arc::clone(&foreign_connections);
        let rejected = Arc::clone(&wrong_clients);
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = &mut stopped => break,
                    completed = connections.join_next(), if !connections.is_empty() => {
                        completed.unwrap().unwrap();
                    }
                    accepted = foreign.accept() => {
                        let (connection, _) = accepted.unwrap();
                        foreign_counted.fetch_add(1, Ordering::AcqRel);
                        drop(connection);
                    }
                    accepted = listener.accept() => {
                        let (connection, _) = accepted.unwrap();
                        assert!(connections.len() < 4, "fixture connection ceiling");
                        let acceptor = acceptor.clone();
                        let counted = Arc::clone(&counted);
                        let admitted = Arc::clone(&admitted);
                        let rejected = Arc::clone(&rejected);
                        let reviewed = reviewed.clone();
                        let denied = foreign_origin.clone();
                        connections.spawn(async move {
                            tokio::time::timeout(Duration::from_secs(10), async {
                                let Ok(mut connection) = acceptor.accept(connection).await else { return; };
                                let peer = connection.get_ref().1.peer_certificates();
                                if !peer.is_some_and(|chain| chain.len() == 1 && chain[0].as_ref() == reviewed) {
                                    rejected.fetch_add(1, Ordering::AcqRel);
                                    let _ = connection.shutdown().await;
                                    return;
                                }
                                admitted.fetch_add(1, Ordering::AcqRel);
                                let mut header = Vec::new();
                                let mut block = [0; 1024];
                                while !header.windows(4).any(|part| part == b"\r\n\r\n") {
                                    let length = connection.read(&mut block).await.unwrap();
                                    if length == 0 { return; }
                                    header.extend_from_slice(&block[..length]);
                                    assert!(header.len() <= 8192);
                                }
                                assert!(!String::from_utf8_lossy(&header).to_ascii_lowercase().contains("proxy-authorization:"));
                                counted.fetch_add(1, Ordering::AcqRel);
                                let page = if header.starts_with(b"GET /second") {
                                    PAGE.replace("Core Mac fixture", "Core Mac native identity navigation")
                                } else { PAGE.into() };
                                let page = page.replace("</body>", &format!(
                                    "<output id='denial'>Testing denied origin</output><img src='{denied}/denied' onerror=\"document.getElementById('denial').textContent='Denied origin blocked'\"></body>"));
                                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
                                connection.write_all(response.as_bytes()).await.unwrap();
                                connection.shutdown().await.unwrap();
                            }).await.expect("bounded native client mTLS exchange");
                        });
                    }
                }
            }
            drop(listener);
            drop(foreign);
            while let Some(completed) = connections.join_next().await {
                completed.unwrap();
            }
        });
        Self {
            requests,
            reviewed_clients,
            foreign_connections,
            wrong_clients,
            stop,
            task,
        }
    }
    pub async fn close(self) {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(11), self.task)
            .await
            .expect("synthetic mTLS fixture drainage")
            .unwrap();
    }
}
