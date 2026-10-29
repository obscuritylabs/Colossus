use super::*;
use colossus_ports::CredentialError;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::task::JoinHandle;

#[derive(Default)]
struct MemoryVault {
    record: Mutex<Option<Vec<u8>>>,
    fail_write: AtomicUsize,
}
impl CredentialVault for MemoryVault {
    fn read(&self, _: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        self.record
            .lock()
            .unwrap()
            .as_ref()
            .map(|bytes| VaultRecord::new(bytes.clone()))
            .transpose()
    }
    fn write(&self, _: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
        if self
            .fail_write
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            == Ok(1)
        {
            return Err(CredentialError::Io);
        }
        *self.record.lock().unwrap() = Some(record.expose().to_vec());
        Ok(())
    }
    fn delete(&self, _: &CredentialKey) -> Result<(), CredentialError> {
        *self.record.lock().unwrap() = None;
        Ok(())
    }
}
impl MemoryVault {
    fn json(&self) -> Value {
        serde_json::from_slice(self.record.lock().unwrap().as_ref().unwrap()).unwrap()
    }
}

struct Server {
    url: String,
    pem: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(drop_first: bool, redirect: bool) -> Self {
        let key = KeyPair::generate().unwrap();
        // Deliberately CA=false, like the local Helm browser certificate.
        let cert = CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let pem = cert.pem();
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "https://localhost:{}/api/enroll",
            listener.local_addr().unwrap().port()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let drop_response = AtomicBool::new(drop_first);
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let Ok(Ok(mut stream)) =
                    tokio::time::timeout(Duration::from_secs(5), acceptor.accept(stream)).await
                else {
                    continue;
                };
                let observed = observed.clone();
                let drop_response = &drop_response;
                let _ = tokio::time::timeout(Duration::from_secs(5), async move {
                    let request = read_request(&mut stream).await;
                    observed.lock().unwrap().push(request);
                    if drop_response.swap(false, Ordering::SeqCst) {
                        stream.shutdown().await.unwrap();
                        return;
                    }
                    let response = if redirect {
                        "HTTP/1.1 302 Found\r\nLocation: https://invalid.example/api/enroll\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                    } else {
                        let body = json!({
                            "node": {"project_id":"project-a", "node_id":"node-a", "instance_id":"runtime-a"},
                            "certificate_pem":"synthetic-connector-leaf",
                            "ca_pem":"synthetic-connector-ca",
                            "grpc_endpoint":"https://runtimes.example:443"
                        }).to_string();
                        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    };
                    stream.write_all(response.as_bytes()).await.unwrap();
                    stream.shutdown().await.unwrap();
                })
                .await;
            }
        });
        Self {
            url,
            pem,
            requests,
            task,
        }
    }

    fn trust(&self) -> EnrollmentTrust {
        EnrollmentTrust::from_pem(self.pem.as_bytes()).unwrap()
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

async fn read_request(stream: &mut (impl AsyncRead + AsyncWrite + Unpin)) -> Value {
    let mut bytes = Vec::new();
    let (header_end, length) = loop {
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 16_384);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + length {
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 16_384);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

async fn isolated(test: &str) -> bool {
    const SELECTOR: &str = "COLOSSUS_CONNECTOR_ENROLLMENT_TEST";
    if std::env::var(SELECTOR).as_deref() == Ok(test) {
        return false;
    }
    #[cfg(windows)]
    let directory = {
        let parent = std::path::PathBuf::from(
            std::env::var_os("COLOSSUS_NATIVE_TEST_ROOT")
                .unwrap_or_else(|| std::env::var_os("USERPROFILE").expect("test user profile")),
        );
        assert!(
            parent.is_absolute() && parent.is_dir(),
            "existing absolute native test root"
        );
        tempfile::Builder::new()
            .prefix("colossus-connector-enrollment-")
            .tempdir_in(parent)
            .unwrap()
    };
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().unwrap();
    let home =
        colossus_home::ColossusHome::ensure_at(directory.path().join("installation")).unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--test-threads=1"])
            .env(SELECTOR, test)
            .env("COLOSSUS_HOME", home.root())
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("isolated enrollment test deadline")
    .unwrap();
    assert!(
        output.status.success(),
        "isolated enrollment test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    home.confined_root()
        .open_existing_file(std::path::Path::new("cloud-connector/host-id"))
        .expect("child executed real enrollment inventory");
    true
}
fn store() -> (EnrollmentStore, Arc<MemoryVault>) {
    let vault = Arc::new(MemoryVault::default());
    (
        EnrollmentStore::new(vault.clone(), "test-node").unwrap(),
        vault,
    )
}

async fn enroll(
    store: &EnrollmentStore,
    url: &str,
    instance: &str,
    trust: EnrollmentTrust,
) -> Result<ConnectionConfig, &'static str> {
    store
        .enroll_with_trust(
            url.into(),
            Zeroizing::new("a".repeat(64)),
            instance.into(),
            vec![],
            false,
            trust,
        )
        .await
}

#[test]
fn trust_accepts_explicit_leaf_but_rejects_ambiguous_invalid_and_tampered_pem() {
    let key = KeyPair::generate().unwrap();
    let pem = CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap()
        .pem();
    let trust = EnrollmentTrust::from_pem(pem.as_bytes()).unwrap();
    trust.validate().unwrap();
    let lf = pem.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(trust, EnrollmentTrust::from_pem(lf.as_bytes()).unwrap());
    assert_eq!(trust, EnrollmentTrust::from_pem(crlf.as_bytes()).unwrap());
    for bytes in [
        vec![],
        vec![b'x'; EnrollmentTrust::MAX_PEM_BYTES + 1],
        b"not a certificate".to_vec(),
        b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n".to_vec(),
        key.serialize_pem().into_bytes(),
        format!("{pem}{pem}").into_bytes(),
        format!("{pem}untrusted trailing data").into_bytes(),
        format!("untrusted prefix\n{pem}").into_bytes(),
    ] {
        assert!(EnrollmentTrust::from_pem(&bytes).is_err());
    }
    let mut changed = serde_json::to_value(&trust).unwrap();
    changed["certificate_sha256"] = json!("0".repeat(64));
    let changed: EnrollmentTrust = serde_json::from_value(changed).unwrap();
    assert_eq!(changed.validate(), Err("invalid enrollment trust binding"));
    assert_eq!(EnrollmentTrust::public_roots(), EnrollmentTrust::default());
}

#[tokio::test]
async fn private_tls_requires_exact_trust_and_hostname_and_never_follows_redirects() {
    if isolated("enrollment::tests::private_tls_requires_exact_trust_and_hostname_and_never_follows_redirects").await {
        return;
    }
    let server = Server::start(false, false).await;
    assert!(
        enroll(
            &store().0,
            &server.url,
            "runtime-a",
            EnrollmentTrust::public_roots()
        )
        .await
        .is_err()
    );
    let other = Server::start(false, false).await;
    assert!(
        enroll(&store().0, &server.url, "runtime-a", other.trust())
            .await
            .is_err()
    );
    let wrong_host = server.url.replace("localhost", "127.0.0.1");
    assert!(
        enroll(&store().0, &wrong_host, "runtime-a", server.trust())
            .await
            .is_err()
    );
    assert_eq!(
        server.count(),
        0,
        "untrusted TLS must not receive an invitation"
    );
    let (enrollment, _) = store();
    assert_eq!(
        enroll(&enrollment, &server.url, "runtime-a", server.trust())
            .await
            .unwrap()
            .node_id,
        "node-a"
    );
    assert_eq!(server.count(), 1);
    let redirect = Server::start(false, true).await;
    assert_eq!(
        enroll(&store().0, &redirect.url, "runtime-a", redirect.trust())
            .await
            .err()
            .unwrap(),
        "cloud enrollment rejected"
    );
    assert_eq!(redirect.count(), 1);
}

async fn recover_exchange(drop_response: bool) {
    let server = Server::start(drop_response, false).await;
    let other = Server::start(false, false).await;
    let (enrollment, vault) = store();
    assert!(!enrollment.has_saved_exchange().unwrap());
    if !drop_response {
        vault.fail_write.store(2, Ordering::SeqCst);
    }
    assert!(
        enroll(&enrollment, &server.url, "runtime-a", server.trust())
            .await
            .is_err()
    );
    assert!(enrollment.has_saved_exchange().unwrap());
    assert_eq!(server.count(), 1);
    let pending = vault.json();
    assert!(pending["config"].is_null());
    assert_eq!(pending["pending"]["token"], "a".repeat(64));
    assert_eq!(
        pending["pending"]["trust"],
        serde_json::to_value(server.trust()).unwrap()
    );
    for (url, instance, trust) in [
        (format!("{}/other", server.url), "runtime-a", server.trust()),
        (server.url.clone(), "runtime-b", server.trust()),
        (server.url.clone(), "runtime-a", other.trust()),
        (
            server.url.clone(),
            "runtime-a",
            EnrollmentTrust::public_roots(),
        ),
    ] {
        assert!(enroll(&enrollment, &url, instance, trust).await.is_err());
    }
    assert_eq!(
        server.count(),
        1,
        "changed bindings must fail before a POST"
    );
    let config = enrollment
        .enroll_with_trust(
            server.url.clone(),
            Zeroizing::new(String::new()),
            "runtime-a".into(),
            vec![],
            false,
            server.trust(),
        )
        .await
        .unwrap();
    assert_eq!(config.node_id, "node-a");
    assert_eq!(server.count(), 2);
    {
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0]["csr_pem"], requests[1]["csr_pem"]);
        assert_eq!(requests[0]["token"], requests[1]["token"]);
    }
    assert_eq!(vault.json()["pending"]["token"], "");
    enrollment
        .enroll_with_trust(
            server.url.clone(),
            Zeroizing::new(String::new()),
            "runtime-a".into(),
            vec![],
            false,
            server.trust(),
        )
        .await
        .unwrap();
    assert_eq!(
        server.count(),
        2,
        "completed exchange must not consume another invitation"
    );
    assert!(
        enroll(&enrollment, &server.url, "runtime-a", other.trust())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn lost_response_reuses_pending_key_invitation_and_trust() {
    if isolated("enrollment::tests::lost_response_reuses_pending_key_invitation_and_trust").await {
        return;
    }
    recover_exchange(true).await;
}

#[tokio::test]
async fn failed_completed_record_save_reconciles_the_committed_exchange() {
    if isolated("enrollment::tests::failed_completed_record_save_reconciles_the_committed_exchange")
        .await
    {
        return;
    }
    recover_exchange(false).await;
}

#[test]
fn legacy_records_default_to_public_roots_and_saved_record_probe_is_redacted() {
    let (enrollment, vault) = store();
    let record = json!({
        "config": null,
        "pending": {
            "client_key_pem":"synthetic-private-key",
            "csr_pem":"synthetic-csr",
            "token":"synthetic-invitation",
            "enrollment_url":"https://control.example/api/enroll",
            "instance_id":"runtime-a"
        }
    });
    *vault.record.lock().unwrap() = Some(serde_json::to_vec(&record).unwrap());
    assert!(enrollment.has_saved_exchange().unwrap());
    let record: Record = serde_json::from_value(record).unwrap();
    assert_eq!(record.pending.trust, EnrollmentTrust::public_roots());
    *vault.record.lock().unwrap() = Some(b"invalid synthetic record".to_vec());
    assert_eq!(
        enrollment.has_saved_exchange(),
        Err("enrollment record invalid")
    );
}
