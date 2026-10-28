use super::*;
use rcgen::{CertificateParams, KeyPair};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tonic::transport::{Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig};

struct Task(JoinHandle<()>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn certificate(host: &str) -> (String, String) {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec![host.into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    (cert.pem(), key.serialize_pem())
}

#[test]
fn reserved_label_boundary_never_routes_lookalikes_or_nonloopback_addresses() {
    for host in [
        "localhost",
        "LOCALHOST",
        "localhost.",
        "colossus.localhost",
        "runtime.node.LOCALHOST.",
        "a.b.localhost",
    ] {
        let addresses = addresses(host, 9443).unwrap();
        assert_eq!(addresses[0], "127.0.0.1:9443".parse().unwrap());
        assert_eq!(addresses[1], "[::1]:9443".parse().unwrap());
        assert!(addresses.iter().all(|address| address.ip().is_loopback()));
    }
    for host in [
        "",
        "notlocalhost",
        "localhost.example",
        "foo.localhost.example",
        "localhostevil",
        "foo.localhostevil",
        ".localhost",
        "a..localhost",
        "localhost..",
        "localhost:9443",
        "127.0.0.1",
        "::1",
        "localhost\0",
    ] {
        assert!(
            addresses(host, 9443).is_none(),
            "must retain ordinary routing: {host:?}"
        );
    }
}

#[test]
fn literal_loopback_addresses_disable_only_their_own_proxy_path() {
    for url in ["http://127.0.0.1:9443", "https://[::1]:9443"] {
        assert!(loopback_ip(&url::Url::parse(url).unwrap()));
    }
    for url in [
        "https://192.0.2.1",
        "https://[::]",
        "https://127.0.0.1.example",
        "https://localhost.example",
    ] {
        assert!(!loopback_ip(&url::Url::parse(url).unwrap()));
    }
}

struct Https {
    port: u16,
    pem: String,
    observed: Arc<Mutex<Vec<(String, String, SocketAddr)>>>,
    _task: Task,
}
async fn https(host: &str) -> Https {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec![host.into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let pem = cert.pem();
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let task = tokio::spawn(async move {
        loop {
            let (stream, peer) = listener.accept().await.unwrap();
            let Ok(Ok(mut stream)) =
                tokio::time::timeout(std::time::Duration::from_secs(3), acceptor.accept(stream))
                    .await
            else {
                continue;
            };
            let sni = stream.get_ref().1.server_name().unwrap().to_owned();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    assert!(request.len() <= 8192);
                }
                captured
                    .lock()
                    .unwrap()
                    .push((sni, String::from_utf8(request).unwrap(), peer));
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ndirect",
                    )
                    .await
                    .unwrap();
                stream.shutdown().await.unwrap();
            })
            .await;
        }
    });
    Https {
        port,
        pem,
        observed,
        _task: Task(task),
    }
}

async fn proxy() -> (u16, Arc<AtomicUsize>, Task) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 8192];
            if stream.read(&mut bytes).await.unwrap() == 0 {
                continue;
            }
            observed.fetch_add(1, Ordering::SeqCst);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nproxied",
                )
                .await
                .unwrap();
        }
    });
    (port, count, Task(task))
}

#[tokio::test]
async fn http_localhost_ignores_external_resolution_and_proxy_but_preserves_tls_name() {
    let host = format!("http-{}.localhost", uuid::Uuid::now_v7().simple());
    let server = https(&host).await;
    let (proxy_port, proxy_count, _proxy) = proxy().await;
    let url = url::Url::parse(&format!("https://{host}:{}/probe", server.port)).unwrap();
    let builder = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(server.pem.as_bytes()).unwrap())
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).unwrap())
        .resolve(&host, "192.0.2.1:9".parse().unwrap())
        .timeout(std::time::Duration::from_secs(3));
    let response = http(builder, &url)
        .build()
        .unwrap()
        .get(url.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "direct");
    assert_eq!(proxy_count.load(Ordering::SeqCst), 0);
    {
        let observed = server.observed.lock().unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].0, host);
        assert!(
            observed[0]
                .1
                .to_ascii_lowercase()
                .contains(&format!("host: {host}:{}", server.port))
        );
        assert!(observed[0].2.ip().is_loopback());
    }
    let wrong = url::Url::parse(&format!("https://wrong.{host}:{}/probe", server.port)).unwrap();
    let builder = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(server.pem.as_bytes()).unwrap())
        .timeout(std::time::Duration::from_secs(3));
    assert!(
        http(builder, &wrong)
            .build()
            .unwrap()
            .get(wrong)
            .send()
            .await
            .is_err()
    );
    assert_eq!(server.observed.lock().unwrap().len(), 1);

    // Literal loopback needs no DNS override and must also bypass an ambient proxy.
    let (target_port, target_count, _target) = proxy().await;
    let ip = url::Url::parse(&format!("http://127.0.0.1:{target_port}/probe")).unwrap();
    let builder = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).unwrap())
        .timeout(std::time::Duration::from_secs(3));
    let response = http(builder, &ip)
        .build()
        .unwrap()
        .get(ip)
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "proxied");
    assert_eq!(target_count.load(Ordering::SeqCst), 1);
    assert_eq!(proxy_count.load(Ordering::SeqCst), 0);

    // The reserved label must not disable proxies for a lookalike external name.
    let other = url::Url::parse("http://localhost.example.invalid/probe").unwrap();
    let builder = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).unwrap())
        .timeout(std::time::Duration::from_secs(3));
    let response = http(builder, &other)
        .build()
        .unwrap()
        .get(other)
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "proxied");
    assert_eq!(proxy_count.load(Ordering::SeqCst), 1);
}

async fn grpc(host: &str) -> (u16, String, Task) {
    let (pem, key) = certificate(host);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let identity = Identity::from_pem(pem.clone(), key);
    let incoming = futures::stream::unfold(listener, |listener| async move {
        let accepted = listener.accept().await.map(|(stream, _)| stream);
        Some((accepted, listener))
    });
    let task = tokio::spawn(async move {
        Server::builder()
            .tls_config(ServerTlsConfig::new().identity(identity))
            .unwrap()
            .add_routes(tonic::service::Routes::default())
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    (port, pem, Task(task))
}

fn endpoint(host: &str, port: u16, ca: &str) -> Endpoint {
    Endpoint::from_shared(format!("https://{host}:{port}"))
        .unwrap()
        .tls_config(ClientTlsConfig::new().ca_certificate(Certificate::from_pem(ca)))
        .unwrap()
        .connect_timeout(std::time::Duration::from_secs(3))
}

#[tokio::test]
async fn grpc_localhost_dials_loopback_with_original_tls_hostname_and_ca() {
    let host = format!("grpc-{}.localhost", uuid::Uuid::now_v7().simple());
    let (port, pem, _server) = grpc(&host).await;

    let configured = endpoint(&host, port, &pem);
    assert_eq!(configured.uri().host(), Some(host.as_str()));
    let channel = connect(configured).await.unwrap();
    drop(channel);
    assert!(
        connect(endpoint(&format!("wrong.{host}"), port, &pem))
            .await
            .is_err()
    );
    let (wrong_ca, _) = certificate(&host);
    assert!(connect(endpoint(&host, port, &wrong_ca)).await.is_err());
}
