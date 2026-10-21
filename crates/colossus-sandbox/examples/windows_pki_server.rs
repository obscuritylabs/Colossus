//! Owned loopback TLS fixture; client-CA hints exclude unrelated Windows identities.
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::WebPkiClientVerifier,
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    task::JoinSet,
};
use tokio_rustls::TlsAcceptor;

type Events = Arc<Mutex<Vec<serde_json::Value>>>;

fn record(events: &Events, endpoint: &str, path: &str, fingerprint: Option<&str>, status: u16) {
    let mut events = events.lock().unwrap();
    assert!(events.len() < 512, "fixture event ceiling");
    events.push(serde_json::json!({"endpoint":endpoint,"path":path,"client_fingerprint_sha256":fingerprint,"status":status}));
}

fn configuration(directory: &Path, certificate: &str, mutual: bool) -> ServerConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap();
    let builder = if mutual {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(
                std::fs::read(directory.join("ca.der")).unwrap(),
            ))
            .unwrap();
        // Rustls sends only this generated root subject in CertificateRequest.
        // Python ssl does not expose that issuer-hint API.
        let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .unwrap();
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };
    // Select the key by the public fixture certificate name, not an incoming request.
    let key = PrivateKeyDer::from_pem_slice(
        &std::fs::read(directory.join(format!("{certificate}.key"))).unwrap(),
    )
    .unwrap();
    let certificate =
        CertificateDer::from(std::fs::read(directory.join(format!("{certificate}.der"))).unwrap());
    builder.with_single_cert(vec![certificate], key).unwrap()
}

async fn serve(
    listener: TcpListener,
    endpoint: &'static str,
    acceptor: TlsAcceptor,
    events: Events,
    expected: String,
    redirect: String,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            socket = listener.accept() => {
                let Ok((socket, _)) = socket else { break; };
                if connections.len() >= 16 { drop(socket); continue; }
                let acceptor = acceptor.clone(); let events = Arc::clone(&events);
                let expected = expected.clone(); let redirect = redirect.clone();
                connections.spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(5), async move {
                        let Ok(mut stream) = acceptor.accept(socket).await else {
                            record(&events, endpoint, "tls_denied", None, 0); return;
                        };
                        let fingerprint = stream.get_ref().1.peer_certificates().and_then(|certs| certs.first()).map(|cert| hex::encode(Sha256::digest(cert.as_ref())));
                        let mut bytes = Vec::new();
                        while bytes.len() < 16*1024 {
                            let mut byte = [0];
                            if stream.read_exact(&mut byte).await.is_err() { return; }
                            bytes.push(byte[0]);
                            if bytes.ends_with(b"\r\n\r\n") { break; }
                        }
                        if !bytes.ends_with(b"\r\n\r\n") { return; }
                        let header = String::from_utf8_lossy(&bytes);
                        let path = header.lines().next().and_then(|line| line.strip_prefix("GET ")).and_then(|line| line.split_once(' ')).map(|(path,_)|path).unwrap_or("unknown");
                        let (path,status,title) = match path {
                            "/public" => ("/public",200,"Colossus private CA verified"),
                            "/redirect-denied" => ("/redirect-denied",302,"redirect"),
                            "/client-check" if fingerprint.as_deref() == Some(expected.as_str()) => ("/client-check",200,"Colossus mTLS identity verified"),
                            "/client-check" => ("/client-check",403,"Colossus identity rejected"),
                            _ => ("unknown",404,"unavailable"),
                        };
                        record(&events, endpoint, path, fingerprint.as_deref(), status);
                        let body = format!("<!doctype html><title>{title}</title><p>{title}</p>");
                        let location = if status == 302 { format!("Location: {redirect}/client-check\r\n") } else { String::new() };
                        let response = format!("HTTP/1.1 {status} Fixture\r\n{location}Content-Type: text/html\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len());
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    }).await;
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {},
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("new owned fixture directory"),
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("fixture-material.json")).unwrap())
            .unwrap();
    let expected = metadata["fingerprints_sha256"]["client"]
        .as_str()
        .unwrap()
        .to_owned();
    let cases = [
        ("public", "server", false),
        ("mtls", "server", true),
        ("redirect_mtls", "server", true),
        ("untrusted", "untrusted_server", false),
        ("wrong_hostname", "wrong_host_server", false),
        ("expired", "expired_server", false),
    ];
    let mut listeners = Vec::new();
    let mut urls = BTreeMap::new();
    for (endpoint, certificate, mutual) in cases {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        urls.insert(
            endpoint,
            format!(
                "https://127.0.0.1:{}",
                listener.local_addr().unwrap().port()
            ),
        );
        listeners.push((
            endpoint,
            listener,
            configuration(&directory, certificate, mutual),
        ));
    }
    let events: Events = Default::default();
    let mut servers = JoinSet::new();
    for (endpoint, listener, configuration) in listeners {
        servers.spawn(serve(
            listener,
            endpoint,
            TlsAcceptor::from(Arc::new(configuration)),
            Arc::clone(&events),
            expected.clone(),
            urls["redirect_mtls"].clone(),
        ));
    }
    println!("{}", serde_json::json!({"ready":true,"urls":urls}));
    std::io::stdout().flush().unwrap();
    let input = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).unwrap();
        line
    });
    let stopped = tokio::time::timeout(Duration::from_secs(330), input).await;
    assert!(
        matches!(stopped,Ok(Ok(ref line)) if line == "stop\n" || line == "stop\r\n" || line.is_empty())
    );
    servers.abort_all();
    while servers.join_next().await.is_some() {}
    let report = serde_json::json!({"events":*events.lock().unwrap(),"overflow":false});
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("server-events.json"))
        .unwrap();
    output
        .write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
        .unwrap();
}
