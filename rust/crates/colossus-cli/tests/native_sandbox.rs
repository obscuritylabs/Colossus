#![cfg(any(target_os = "linux", target_os = "macos"))]
//! Native helper integration and escape tests.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
    time::Duration,
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SIGNING_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> Output {
    Command::new(binary)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run colossus")
}

#[test]
fn native_helper_enforces_filesystem_environment_and_process_tree_boundaries() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let allowed = directory.path().join("allowed");
    let denied = directory.path().join("denied");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&allowed).expect("allowed");
    fs::create_dir_all(&denied).expect("denied");
    fs::create_dir_all(&workflows).expect("workflows");
    let allowed_file = allowed.join("allowed.txt");
    let denied_file = denied.join("denied.txt");
    fs::write(&allowed_file, "allowed").expect("allowed file");
    fs::write(&denied_file, "denied").expect("denied file");
    let escape = allowed.join("escape.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&denied_file, &escape).expect("escape symlink");

    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    let tls_root = if cfg!(target_os = "macos") {
        "/private/etc/ssl"
    } else {
        "/etc/ssl"
    };
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_TEST_JOURNAL_KEY
    journal_key_id: test-journal-v1
    signing_variable: COLOSSUS_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [process.spawn]
  approval_actions: []
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
sandbox:
  backend: native
  profile: integration-native-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {allowed}
      mode: write
    - root: {tls_root}
      mode: read
  executables:
    - /bin/cat
    - /usr/bin/env
    - /bin/sh
    - /bin/sleep
    - /usr/bin/curl
  environment: [SAFE]
  networkDestinations: []
  timeoutMs: 2000
  maxOutputBytes: 1048576
  maxProcesses: 1
  maxMemoryBytes: 268435456
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            allowed = allowed.display(),
            tls_root = tls_root,
        ),
    )
    .expect("config");

    let doctor = run(binary, &config, &["sandbox", "doctor"]);
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor: Value = serde_json::from_slice(&doctor.stdout).expect("doctor JSON");
    if doctor["native_supported"] != Value::Bool(true) {
        return;
    }

    let allowed_output = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/bin/cat",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            allowed_file.to_str().expect("allowed file"),
        ],
    );
    assert!(
        allowed_output.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed_output.stderr)
    );
    let result: Value = serde_json::from_slice(&allowed_output.stdout).expect("result JSON");
    assert_eq!(
        BASE64
            .decode(result["stdout_base64"].as_str().expect("stdout"))
            .expect("base64"),
        b"allowed"
    );

    let escaped = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/bin/cat",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            escape.to_str().expect("escape path"),
        ],
    );
    assert!(
        !escaped.status.success(),
        "symlink escape unexpectedly succeeded"
    );

    let environment = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/usr/bin/env",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--env",
            "SAFE=yes",
        ],
    );
    assert!(environment.status.success());
    let environment: Value = serde_json::from_slice(&environment.stdout).expect("env JSON");
    assert_eq!(
        BASE64
            .decode(environment["stdout_base64"].as_str().expect("stdout"))
            .expect("base64"),
        b"SAFE=yes\n"
    );

    let marker = allowed.join("child-escaped");
    let command = format!(
        "(sleep 1; echo escaped > '{}') & sleep 30",
        marker.display()
    );
    let timed_out = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/bin/sh",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "-c",
            &command,
        ],
    );
    assert!(
        !timed_out.status.success()
            && String::from_utf8_lossy(&timed_out.stderr).contains("process-count limit"),
        "child process limit was not enforced: {}",
        String::from_utf8_lossy(&timed_out.stderr)
    );
    thread::sleep(Duration::from_millis(1_200));
    assert!(
        !marker.exists(),
        "timed-out descendant escaped its process group"
    );

    let normal_exit_marker = allowed.join("normal-exit-child-escaped");
    let normal_exit_command = format!(
        "(sleep 1; echo escaped > '{}') & exit 0",
        normal_exit_marker.display()
    );
    let relaxed_config = fs::read_to_string(&config)
        .expect("read config")
        .replace("  maxProcesses: 1", "  maxProcesses: 8");
    fs::write(&config, relaxed_config).expect("relax process count");
    let normal_exit = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/bin/sh",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "-c",
            &normal_exit_command,
        ],
    );
    assert!(
        normal_exit.status.success(),
        "leader should exit successfully: {}",
        String::from_utf8_lossy(&normal_exit.stderr)
    );
    thread::sleep(Duration::from_millis(1_200));
    assert!(
        !normal_exit_marker.exists(),
        "normal-exit descendant escaped its process group"
    );

    let direct_timeout = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/bin/sleep",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "30",
        ],
    );
    assert!(
        !direct_timeout.status.success()
            && String::from_utf8_lossy(&direct_timeout.stderr).contains("timeout"),
        "direct timeout was not enforced: {}",
        String::from_utf8_lossy(&direct_timeout.stderr)
    );

    if Path::new("/usr/bin/curl").exists() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("network listener");
        let address = listener.local_addr().expect("network address");
        let origin = format!("http://{address}");
        let updated = fs::read_to_string(&config).expect("read config").replace(
            "  networkDestinations: []",
            &format!("  networkDestinations:\n    - {origin}"),
        );
        fs::write(&config, updated).expect("network config");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("network accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).expect("network read");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .expect("network write");
        });
        let denied_port = address.port().saturating_add(1);
        let denied_url = format!("http://127.0.0.1:{denied_port}/");
        let denied_network = run(
            binary,
            &config,
            &[
                "process",
                "run",
                "/usr/bin/curl",
                "--cwd",
                allowed.to_str().expect("allowed path"),
                "--",
                "--fail",
                "--silent",
                &denied_url,
            ],
        );
        assert!(
            !denied_network.status.success(),
            "unlisted network destination unexpectedly succeeded"
        );
        let allowed_url = format!("{origin}/");
        let allowed_network = run(
            binary,
            &config,
            &[
                "process",
                "run",
                "/usr/bin/curl",
                "--cwd",
                allowed.to_str().expect("allowed path"),
                "--",
                "--fail",
                "--silent",
                &allowed_url,
            ],
        );
        assert!(
            allowed_network.status.success(),
            "{}",
            String::from_utf8_lossy(&allowed_network.stderr)
        );
        let result: Value =
            serde_json::from_slice(&allowed_network.stdout).expect("network result");
        assert_eq!(
            BASE64
                .decode(result["stdout_base64"].as_str().expect("stdout"))
                .expect("base64"),
            b"ok"
        );
        server.join().expect("server thread");
    }
}
