//! Loopback-only mTLS fixture. It never prints headers, request bodies, or tokens.

use super::{HostSecret, Value, json};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
};
use rustls::{
    RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
    server::WebPkiClientVerifier,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MARKER: &str = "LARGE_MCP_TOKEN_ACCEPTANCE_OK";
pub(super) const PREFIX: &str = "ColossusSyntheticMcpToken.v1.";

pub(super) fn assert_no_secret(value: &str) {
    assert!(
        !value.contains(PREFIX),
        "synthetic credential appeared in released data"
    );
}

pub(super) fn token(size: usize) -> HostSecret {
    HostSecret::new(format!("{PREFIX}{}", "x".repeat(size - PREFIX.len()))).unwrap()
}

#[derive(Default)]
struct Evidence {
    initialized: usize,
    discovered: usize,
    called: usize,
    provider: usize,
    summary: usize,
}

pub(super) struct Server {
    pub origin: String,
    evidence: Arc<Mutex<BTreeMap<usize, Evidence>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(root: &Path) -> Self {
        let mut ca_params =
            CertificateParams::new(vec!["Colossus Desktop test CA".into()]).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
        let mut server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_key = KeyPair::generate().unwrap();
        let server_cert = server_params.signed_by(&server_key, &ca).unwrap();
        let mut client_params =
            CertificateParams::new(vec!["Colossus Desktop client".into()]).unwrap();
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_key = KeyPair::generate().unwrap();
        let client_cert = client_params.signed_by(&client_key, &ca).unwrap();
        fs::write(root.join("ca.pem"), ca.pem()).unwrap();
        fs::write(root.join("client.pem"), client_cert.pem()).unwrap();
        #[cfg(windows)]
        colossus_windows_native::create_private_file(
            &root.join("client-key.pem"),
            client_key.serialize_pem().as_bytes(),
        )
        .unwrap();
        #[cfg(target_os = "macos")]
        fs::write(root.join("client-key.pem"), client_key.serialize_pem()).unwrap();
        let mut trusted_clients = RootCertStore::empty();
        trusted_clients.add(ca.der().clone()).unwrap();
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(trusted_clients),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .unwrap();
        let tls =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_client_cert_verifier(verifier)
                .with_single_cert(
                    vec![server_cert.der().clone()],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
                )
                .unwrap();
        let tls = Arc::new(tls);
        let expected_leaf = client_cert.der().clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        listener.set_nonblocking(true).unwrap();
        let evidence = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_evidence = Arc::clone(&evidence);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .unwrap();
                        let mut stream = StreamOwned::new(
                            ServerConnection::new(Arc::clone(&tls)).unwrap(),
                            socket,
                        );
                        serve(&mut stream, &thread_evidence);
                        assert_eq!(
                            stream.conn.peer_certificates().unwrap()[0],
                            expected_leaf,
                            "managed request used the imported client leaf"
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("loopback fixture accept failed: {}", error.kind()),
                }
            }
        });
        Self {
            origin,
            evidence,
            stop,
            thread: Some(thread),
        }
    }

    pub fn assert_roundtrips(&self) {
        let evidence = self.evidence.lock().unwrap();
        for size in [8192, 65536] {
            let result = evidence
                .get(&size)
                .expect("exact-token traffic was received");
            assert!(
                result.initialized >= 1,
                "MCP initialization missing for {size}"
            );
            assert!(result.discovered >= 1, "MCP discovery missing for {size}");
            assert_eq!(result.called, 1, "MCP tool did not execute once for {size}");
            assert!(
                result.provider >= 2,
                "provider round trip missing for {size}"
            );
            assert_eq!(
                result.summary, 1,
                "provider did not receive actual MCP result for {size}"
            );
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !std::thread::panicking() {
                result.expect("HTTP fixture failed");
            }
        }
    }
}

fn request(
    stream: &mut StreamOwned<ServerConnection, TcpStream>,
) -> (String, String, usize, Value) {
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(128 * 1024 + 1024 * 1024));
    let header_end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).expect("bounded fixture request");
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        assert!(bytes.len() < 128 * 1024, "fixture header bound");
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let first = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>();
    let method = first[0].to_owned();
    let path = first[1].to_owned();
    let size = path
        .split('/')
        .find_map(|segment| segment.parse::<usize>().ok())
        .unwrap();
    assert!([8192, 65536].contains(&size));
    let authorization = headers
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim())
        })
        .collect::<Vec<_>>();
    assert_eq!(authorization.len(), 1, "one bearer header required");
    let expected = token(size);
    assert!(
        authorization[0].strip_prefix("Bearer ") == Some(expected.expose()),
        "native pipeline changed the exact synthetic token"
    );
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(length <= 1024 * 1024, "fixture body bound");
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
    };
    (method, path, size, body)
}

fn serve(
    stream: &mut StreamOwned<ServerConnection, TcpStream>,
    evidence: &Mutex<BTreeMap<usize, Evidence>>,
) {
    let (method, path, size, body) = request(stream);
    let mut all = evidence.lock().unwrap();
    let evidence = all.entry(size).or_default();
    let (status, response) = if method == "GET" {
        ("405 Method Not Allowed", None)
    } else if method == "DELETE" {
        ("204 No Content", None)
    } else if path.starts_with("/provider/") {
        evidence.provider += 1;
        ("200 OK", Some(completion(&body, evidence)))
    } else if body.get("id").is_none() {
        ("202 Accepted", None)
    } else {
        let result = match body["method"].as_str().unwrap() {
            "initialize" => {
                evidence.initialized += 1;
                json!({"protocolVersion": body["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "synthetic-credential-acceptance", "version": "1"}})
            }
            "tools/list" => {
                evidence.discovered += 1;
                json!({"tools": [{"name": "credential_roundtrip", "description": "Return a fixed harmless acceptance marker.", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}, "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}}]})
            }
            "tools/call" => {
                assert_eq!(body["params"]["name"], "credential_roundtrip");
                evidence.called += 1;
                json!({"content": [{"type": "text", "text": MARKER}], "isError": false})
            }
            "resources/list" => json!({"resources": []}),
            "prompts/list" => json!({"prompts": []}),
            "ping" => json!({}),
            _ => panic!("unexpected MCP fixture operation"),
        };
        (
            "200 OK",
            Some(json!({"jsonrpc": "2.0", "id": body["id"], "result": result})),
        )
    };
    let body = response.map(|value| value.to_string()).unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("fixture response");
    stream.flush().expect("flush TLS response");
}

fn completion(body: &Value, evidence: &mut Evidence) -> Value {
    let serialized = zeroize::Zeroizing::new(body.to_string());
    assert_no_secret(&serialized);
    assert_ne!(
        body["stream"], true,
        "acceptance uses non-streaming provider"
    );
    let messages = body["messages"].as_array().unwrap();
    let last_user = messages
        .iter()
        .rposition(|message| message["role"] == "user")
        .unwrap();
    let tool_results = messages[last_user + 1..]
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect::<Vec<_>>();
    let (finish, message) = if tool_results.is_empty() {
        assert!(
            body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "mcp_call"),
            "real MCP gateway was not offered"
        );
        (
            "tool_calls",
            json!({"role": "assistant", "content": null, "tool_calls": [{"id": "synthetic-call", "type": "function", "function": {"name": "mcp_call", "arguments": json!({"server": "large-token", "tool": "credential_roundtrip", "arguments": {}}).to_string()}}]}),
        )
    } else {
        assert!(
            tool_results
                .iter()
                .any(|message| message["content"].to_string().contains(MARKER)),
            "provider did not receive actual MCP marker"
        );
        evidence.summary += 1;
        (
            "stop",
            json!({"role": "assistant", "content": "The synthetic MCP credential round trip succeeded."}),
        )
    };
    json!({"id": "chatcmpl-acceptance", "object": "chat.completion", "created": 0, "model": "synthetic-acceptance", "choices": [{"index": 0, "message": message, "finish_reason": finish}], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
}
