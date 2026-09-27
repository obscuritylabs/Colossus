//! Provider setup before configuration, with real loopback discovery and durable evidence.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write as _},
    net::{TcpListener, TcpStream},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const SECRET: &str = "provider-setup-synthetic-secret";
const SECRET_VARIABLE: &str = "COLOSSUS_PROVIDER_SETUP_TEST_KEY";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

struct Fixture {
    directory: tempfile::TempDir,
    home: process_support::IsolatedUserHome,
}

impl Fixture {
    fn new() -> Self {
        let directory = process_support::tempdir().expect("workspace");
        let home = process_support::isolated_user_home(directory.path());
        Self { directory, home }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
        command
            .current_dir(self.directory.path())
            .env("HOME", self.home.path())
            .env("COLOSSUS_HOME", self.home.colossus_home())
            .env(SECRET_VARIABLE, SECRET)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command
            .env("USERPROFILE", self.home.path())
            .env("LOCALAPPDATA", self.home.local_app_data())
            .env("TEMP", self.home.temporary_directory())
            .env("TMP", self.home.temporary_directory());
        let mut child = command.spawn().expect("CLI");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let stdout = thread::spawn(move || read_bounded(stdout));
        let stderr = thread::spawn(move || read_bounded(stderr));
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait().expect("CLI status") {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout.join();
                let _ = stderr.join();
                panic!("provider setup command exceeded its deadline");
            }
            thread::sleep(Duration::from_millis(20));
        };
        Output {
            status,
            stdout: stdout.join().expect("stdout reader"),
            stderr: stderr.join().expect("stderr reader"),
        }
    }

    fn assert_no_configuration(&self) {
        assert!(!self.home.colossus_home().join("config.yaml").exists());
        assert!(!self.directory.path().join(".colossus/config.yaml").exists());
    }
}

fn read_bounded(reader: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .expect("CLI output");
    assert!(u64::try_from(bytes.len()).expect("output length") <= MAX_OUTPUT_BYTES);
    bytes
}

fn assert_secret_absent(bytes: &[u8]) {
    assert!(
        !bytes
            .windows(SECRET.len())
            .any(|bytes| bytes == SECRET.as_bytes())
    );
}

fn success(output: &Output) -> Value {
    assert_secret_absent(&output.stdout);
    assert_secret_absent(&output.stderr);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}

#[test]
fn provider_presets_are_available_without_a_configuration() {
    let fixture = Fixture::new();
    let output = fixture.run(&["provider", "presets"]);
    let presets = success(&output);
    let presets = presets.as_array().expect("presets");
    for id in [
        "codex",
        "openrouter",
        "openai",
        "ollama",
        "custom-chat",
        "custom-responses",
    ] {
        assert!(
            presets.iter().any(|preset| preset["id"] == id),
            "missing preset {id}"
        );
    }
    assert!(presets.iter().any(|preset| preset["id"] == "custom-chat" && preset["protocol"] == "chat_completions"));
    assert!(
        presets
            .iter()
            .any(|preset| preset["id"] == "custom-responses" && preset["protocol"] == "responses")
    );
    fixture.assert_no_configuration();
}

#[test]
fn manual_responses_setup_writes_only_a_credential_reference_and_never_overwrites() {
    let fixture = Fixture::new();
    let arguments = [
        "--config",
        "new-provider.yaml",
        "provider",
        "setup",
        "--preset",
        "custom-responses",
        "--base-url",
        "http://localhost:1234/v1/",
        "--credential-env",
        SECRET_VARIABLE,
        "--model",
        "local-model",
        "--context-window-tokens",
        "64000",
        "--max-output-tokens",
        "8000",
        "--tool-calls",
        "true",
        "--streaming",
        "false",
        "--image-inputs",
        "true",
    ];
    let output = fixture.run(&arguments);
    let result = success(&output);
    assert_eq!(result["created"], true);
    let config_path = fixture.directory.path().join("new-provider.yaml");
    let original = fs::read(&config_path).expect("created configuration");
    assert_secret_absent(&original);
    let config =
        colossus_runtime::RuntimeConfig::from_yaml(std::str::from_utf8(&original).expect("YAML"))
            .expect("valid setup configuration");
    let provider = &config.providers.profiles["setup-provider"];
    assert_eq!(provider.kind.as_str(), "openai_responses");
    assert_eq!(
        provider.base_url.as_deref(),
        Some("http://localhost:1234/v1")
    );
    assert_eq!(
        provider.credential_reference.as_deref(),
        Some("env:COLOSSUS_PROVIDER_SETUP_TEST_KEY")
    );
    let model = &config.models.profiles["primary"];
    assert_eq!(model.model, "local-model");
    assert_eq!(
        (model.context_window_tokens, model.max_output_tokens),
        (64_000, 8_000)
    );
    assert!(model.capabilities.tool_calls && model.capabilities.image_inputs);
    assert!(!model.capabilities.streaming);
    assert_eq!(config.models.roles["primary"], "primary");
    assert_eq!(
        config.sandbox.network_destinations,
        ["http://localhost:1234"]
    );
    let repeated = fixture.run(&arguments);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("already exists"));
    assert_secret_absent(&repeated.stdout);
    assert_secret_absent(&repeated.stderr);
    assert_eq!(
        fs::read(config_path).expect("original configuration"),
        original
    );
    fixture.assert_no_configuration();
}

#[test]
fn noninteractive_setup_requires_an_explicit_model_and_leaves_no_file() {
    let fixture = Fixture::new();
    let output = fixture.run(&["provider", "setup", "--preset", "ollama", "--no-credential"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires --model"));
    fixture.assert_no_configuration();
}

fn read_request(stream: &mut TcpStream) -> String {
    // Winsock can carry the listener's nonblocking mode onto accepted streams.
    // Read headers with the bounded blocking timeout below.
    stream
        .set_nonblocking(false)
        .expect("blocking fixture stream");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16 * 1024, "request header bound");
        let mut byte = [0_u8];
        stream.read_exact(&mut byte).expect("request byte");
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).expect("HTTP request")
}

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "catalog request deadline");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("catalog listener: {error}"),
        }
    }
}

#[test]
fn chat_and_responses_discovery_load_cards_before_setup_and_persist_effect_evidence() {
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback catalog");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let origin = format!("http://{}", listener.local_addr().expect("catalog address"));
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let mut stream = accept(&listener);
            requests.push(read_request(&mut stream));
            let body = json!({"data": [{
                "id": "catalog-model", "name": "Catalog model", "description": "A discovered model.",
                "context_length": 128_000, "top_provider": {"max_completion_tokens": 16_384},
                "supported_parameters": ["tools"], "architecture": {"input_modalities": ["text", "image"]}
            }]}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).expect("catalog response");
            stream.flush().expect("catalog flush");
        }
        requests
    });
    for preset in ["custom-chat", "custom-responses"] {
        let output = fixture.run(&[
            "provider",
            "discover",
            "--preset",
            preset,
            "--base-url",
            &format!("{origin}/v1"),
            "--credential-env",
            SECRET_VARIABLE,
        ]);
        let models = success(&output);
        assert_eq!(models[0]["id"], "catalog-model");
        assert_eq!(models[0]["display_name"], "Catalog model");
        assert_eq!(models[0]["context_window_tokens"], 128_000);
        assert_eq!(models[0]["max_output_tokens"], 16_384);
        assert_eq!(models[0]["tool_calls"], true);
        assert_eq!(models[0]["image_inputs"], true);
        fixture.assert_no_configuration();
    }
    for request in server.join().expect("catalog server") {
        assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
        assert!(
            request
                .to_lowercase()
                .contains(&format!("authorization: bearer {SECRET}\r\n"))
        );
    }
    let audit_config = fixture.directory.path().join("catalog-audit.yaml");
    fs::write(&audit_config, "schemaVersion: 3\nstorage:\n  location: home_workspace\n  path: provider-discovery.redb\n  keys:\n    kind: none\n").expect("audit configuration");
    let audit = fixture.run(&[
        "--config",
        "catalog-audit.yaml",
        "audit",
        "show",
        "--limit",
        "100",
    ]);
    let records = success(&audit);
    let effect_types = records
        .as_array()
        .expect("audit records")
        .iter()
        .filter(|record| record["actor"]["id"] == "provider-diagnostics")
        .filter_map(|record| record["event_type"].as_str())
        .filter(|event| {
            matches!(
                *event,
                "effect.requested.v1" | "effect.started.v1" | "effect.completed.v1"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        effect_types,
        [
            "effect.requested.v1",
            "effect.started.v1",
            "effect.completed.v1",
            "effect.requested.v1",
            "effect.started.v1",
            "effect.completed.v1"
        ]
    );
}
