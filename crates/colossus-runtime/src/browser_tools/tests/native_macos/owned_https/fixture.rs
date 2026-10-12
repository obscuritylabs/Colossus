//! Exact numeric HTTPS server and a passive forbidden-origin TCP witness.
use super::*;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::{path::Path, sync::atomic::AtomicUsize};
use tokio::{net::TcpListener, sync::oneshot, task::JoinSet};
use tokio_rustls::TlsAcceptor;

pub(super) struct Fixture {
    pub requests: Arc<AtomicUsize>,
    pub foreign_connections: Arc<AtomicUsize>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start(origin: &str, inputs: &Path) -> Self {
        let port = origin
            .strip_prefix("https://127.0.0.1:")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| !matches!(port, 0 | 443))
            .expect("one independently bound numeric HTTPS fixture");
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let foreign = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let foreign_origin = format!("https://{}", foreign.local_addr().unwrap());
        let leaf = CertificateDer::from(std::fs::read(inputs.join("fixture-leaf.der")).unwrap());
        let key = zeroize::Zeroizing::new(std::fs::read(inputs.join("fixture-key.pk8")).unwrap());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec()));
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![leaf], key)
        .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        let requests = Arc::new(AtomicUsize::new(0));
        let foreign_connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        let foreign_counted = Arc::clone(&foreign_connections);
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = &mut stopped => break,
                    accepted = foreign.accept() => {
                        let (connection, _) = accepted.unwrap();
                        foreign_counted.fetch_add(1, Ordering::AcqRel);
                        drop(connection);
                    }
                    completed = connections.join_next(), if !connections.is_empty() => {
                        completed.unwrap().unwrap();
                    }
                    accepted = listener.accept() => {
                        let (connection, _) = accepted.unwrap();
                        assert!(connections.len() < 4, "fixture connection bound");
                        let acceptor = acceptor.clone();
                        let counted = Arc::clone(&counted);
                        let denied = foreign_origin.clone();
                        connections.spawn(async move {
                            tokio::time::timeout(Duration::from_secs(5), async {
                                let Ok(mut connection) = acceptor.accept(connection).await else { return; };
                                let mut header = Vec::new();
                                let mut block = [0; 1024];
                                while !header.windows(4).any(|part| part == b"\r\n\r\n") {
                                    let length = connection.read(&mut block).await.unwrap();
                                    if length == 0 { return; }
                                    header.extend_from_slice(&block[..length]);
                                    assert!(header.len() <= 8192);
                                }
                                assert!(!String::from_utf8_lossy(&header).to_ascii_lowercase()
                                    .contains("proxy-authorization:"));
                                counted.fetch_add(1, Ordering::AcqRel);
                                let page = if header.starts_with(b"GET /second") {
                                    PAGE.replace("Core Mac fixture", "Core Mac navigation")
                                } else {
                                    PAGE.into()
                                };
                                let page = page.replace("</body>", &format!(
                                    "<output id='denial'>Testing denied origin</output><img src='{denied}/denied' onerror=\"document.getElementById('denial').textContent='Denied origin blocked'\"></body>"));
                                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
                                connection.write_all(response.as_bytes()).await.unwrap();
                                connection.shutdown().await.unwrap();
                            }).await.expect("bounded native TLS fixture exchange");
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
            foreign_connections,
            stop,
            task,
        }
    }
    pub async fn close(self) {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(6), self.task)
            .await
            .expect("native fixture server drainage")
            .unwrap();
    }
}
