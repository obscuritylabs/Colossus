//! Live CLI-to-MCP mTLS acceptance with a private, local certificate authority.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

#[path = "support/process.rs"]
mod process_support;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa, KeyPair,
};
use rustls::{
    RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
    server::WebPkiClientVerifier,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

#[test]
fn cli_discovers_remote_mcp_through_imported_client_identity() {
    let directory = process_support::tempdir().expect("private workspace");
    let workspace = directory.path().canonicalize().expect("workspace");
    let mut ca_params = CertificateParams::new(vec!["Colossus CLI test CA".into()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
    let mut server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_key = KeyPair::generate().unwrap();
    let server = server_params.signed_by(&server_key, &ca).unwrap();
    let mut client_params = CertificateParams::new(vec!["Colossus CLI client".into()]).unwrap();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_key = KeyPair::generate().unwrap();
    let client = client_params.signed_by(&client_key, &ca).unwrap();
    let ca_path = workspace.join("ca.pem");
    let certificate_path = workspace.join("client.pem");
    let key_path = workspace.join("client-key.pem");
    fs::write(&ca_path, ca.pem()).unwrap();
    fs::write(&certificate_path, client.pem()).unwrap();
    fs::write(&key_path, client_key.serialize_pem()).unwrap();

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
                vec![server.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
            )
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback TLS listener");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let expected_leaf = client.der().clone();
    let server_task = thread::spawn(move || serve_mcp(listener, Arc::new(tls), expected_leaf));

    let config = workspace.join("config.yaml");
    let origin = format!("https://localhost:{port}");
    let settings = json!({
        "schemaVersion": 3,
        "storage": {"path": workspace.join("state.redb"), "keys": {"kind": "none"}},
        "access": {"profile": "development"},
        "network": {
            "caBundlePath": ca_path,
            "clientCertificatePath": certificate_path,
            "clientKeyPath": key_path,
        },
        "mcp": {"servers": {"mtls-fixture": {
            "transport": "streamable_http", "url": format!("{origin}/mcp"),
            "allowedTools": ["mtls_verified"], "allowStateless": true,
            "timeoutMs": 15000, "maxOutputBytes": 65536,
        }}},
        "sandbox": {
            "backend": if cfg!(windows) {"windows_job"} else {"native"},
            "networkDestinations": [origin], "timeoutMs": 15000,
            "maxOutputBytes": 65536,
        },
    });
    fs::write(&config, settings.to_string()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
    let _home = process_support::isolate_user_home(&mut command, &workspace);
    let output = command
        .current_dir(&workspace)
        .args(["--config", config.to_str().unwrap(), "mcp", "tools"])
        .output()
        .expect("run CLI against live mTLS MCP server");
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tools: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(tools[0]["name"], "mtls_verified");
    server_task.join().expect("mTLS fixture completed");
    let released = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!released.contains("BEGIN PRIVATE KEY"));
}

fn serve_mcp(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    expected_leaf: rustls::pki_types::CertificateDer<'static>,
) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let (socket, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "CLI never completed mTLS discovery"
                );
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) => panic!("MCP accept failed: {error}"),
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut stream = StreamOwned::new(ServerConnection::new(Arc::clone(&tls)).unwrap(), socket);
        let (method, body) = read_request(&mut stream);
        assert_eq!(stream.conn.peer_certificates().unwrap()[0], expected_leaf);
        let operation = body.get("method").and_then(Value::as_str);
        let (status, response) = match (method.as_str(), operation) {
            ("GET", _) => ("405 Method Not Allowed", None),
            ("POST", Some("server/discover")) => (
                "200 OK",
                Some(json!({
                    "jsonrpc": "2.0", "id": body["id"],
                    "error": {"code": -32601, "message": "Method not found"},
                })),
            ),
            ("POST", Some("initialize")) => (
                "200 OK",
                Some(json!({
                    "jsonrpc": "2.0", "id": body["id"],
                    "result": {
                        "protocolVersion": body["params"]["protocolVersion"],
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "mtls-fixture", "version": "1"},
                    }
                })),
            ),
            ("POST", Some("notifications/initialized")) => ("200 OK", None),
            ("POST", Some("tools/list")) => (
                "200 OK",
                Some(json!({
                    "jsonrpc": "2.0", "id": body["id"],
                    "result": {"tools": [{
                        "name": "mtls_verified", "description": "Live client certificate proof",
                        "inputSchema": {"type": "object"},
                    }]}
                })),
            ),
            _ => panic!("unexpected MCP operation: {operation:?}"),
        };
        let payload = response.map(|value| value.to_string()).unwrap_or_default();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        )
        .unwrap();
        stream.flush().unwrap();
        if operation == Some("tools/list") {
            return;
        }
    }
}

fn read_request(stream: &mut StreamOwned<ServerConnection, TcpStream>) -> (String, Value) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).expect("TLS request");
        assert!(count > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        assert!(bytes.len() < 64 * 1024, "request headers too large");
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let method = headers.split_whitespace().next().unwrap().to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    assert!(length <= 64 * 1024, "request body too large");
    while bytes.len() < header_end + length {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).expect("TLS body");
        assert!(count > 0, "request ended before body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
    };
    (method, body)
}
