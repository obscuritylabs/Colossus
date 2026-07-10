//! Opt-in live OCI sandbox acceptance test.

#![cfg(unix)]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use std::{
    env, fs,
    path::Path,
    process::{Command, Output},
    thread,
    time::Duration,
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SIGNING_KEY: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> Output {
    Command::new(binary)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_OCI_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_OCI_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run colossus")
}

fn containers(runtime: &Path) -> Vec<String> {
    let output = Command::new(runtime)
        .args([
            "container",
            "ls",
            "--all",
            "--filter",
            "name=colossus-",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .expect("list OCI containers");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn networks(runtime: &Path) -> Vec<String> {
    let output = Command::new(runtime)
        .args([
            "network",
            "ls",
            "--filter",
            "name=colossus-",
            "--format",
            "{{.Name}}",
        ])
        .output()
        .expect("list OCI networks");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "requires an OCI runtime plus preloaded workload and proxy image digests"]
fn live_oci_enforces_mount_environment_network_timeout_and_cleanup_boundaries() {
    let runtime = fs::canonicalize(env::var("COLOSSUS_OCI_RUNTIME").expect("OCI runtime path"))
        .expect("canonical OCI runtime");
    let image = env::var("COLOSSUS_OCI_IMAGE").expect("immutable preloaded OCI image");
    assert!(image.contains("@sha256:"), "image must be digest pinned");
    let proxy_image =
        env::var("COLOSSUS_OCI_PROXY_IMAGE").expect("immutable preloaded OCI proxy image");
    assert!(
        proxy_image.starts_with("sha256:") || proxy_image.contains("@sha256:"),
        "proxy image must be digest pinned"
    );
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let allowed = directory.path().join("allowed");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&allowed).expect("allowed");
    fs::create_dir_all(&workflows).expect("workflows");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_OCI_JOURNAL_KEY
    journal_key_id: oci-journal-v1
    signing_variable: COLOSSUS_OCI_SIGNING_KEY
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
  backend: oci
  profile: integration-oci-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: {runtime}
  ociImage: {image}
  ociProxyImage: {proxy_image}
  filesystem:
    - root: {allowed}
      mode: write
  executables:
    - /bin/sh
    - /bin/sleep
    - /usr/bin/env
    - /usr/local/bin/python3
  environment: [SAFE]
  networkDestinations: []
  timeoutMs: 6000
  maxOutputBytes: 1048576
  maxProcesses: 16
  maxMemoryBytes: 134217728
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            runtime = runtime.display(),
            allowed = allowed.display(),
            proxy_image = proxy_image,
        ),
    )
    .expect("config");

    let before = containers(&runtime);
    let networks_before = networks(&runtime);
    let write = run(
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
            "printf ok > result.txt && cat result.txt",
        ],
    );
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    let write: Value = serde_json::from_slice(&write.stdout).expect("write result");
    assert_eq!(
        BASE64
            .decode(write["stdout_base64"].as_str().expect("stdout"))
            .expect("base64"),
        b"ok"
    );
    assert_eq!(
        fs::read_to_string(allowed.join("result.txt")).expect("mounted write"),
        "ok"
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
    assert!(
        environment.status.success(),
        "{}",
        String::from_utf8_lossy(&environment.stderr)
    );
    let environment: Value =
        serde_json::from_slice(&environment.stdout).expect("environment result");
    assert_eq!(
        BASE64
            .decode(
                environment["stdout_base64"]
                    .as_str()
                    .expect("environment stdout")
            )
            .expect("base64"),
        b"SAFE=yes\n"
    );

    let root_write = run(
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
            "printf escaped > /etc/colossus-escape",
        ],
    );
    assert!(
        !root_write.status.success(),
        "read-only container root unexpectedly accepted a write"
    );

    let network = run(
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
            "/usr/local/bin/python3 -c \"import socket; socket.create_connection(('1.1.1.1', 53), 0.5)\"",
        ],
    );
    assert!(
        !network.status.success(),
        "network-none container unexpectedly connected"
    );

    let network_config = directory.path().join("network-config.yaml");
    fs::write(
        &network_config,
        fs::read_to_string(&config)
            .expect("read config")
            .replace(
                "  networkDestinations: []",
                "  networkDestinations:\n    - http://example.com\n    - https://example.com",
            )
            .replace("  timeoutMs: 6000", "  timeoutMs: 12000"),
    )
    .expect("network config");
    let allowed_network = run(
        binary,
        &network_config,
        &[
            "process",
            "run",
            "/usr/local/bin/python3",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "-c",
            "import urllib.request; print(urllib.request.urlopen('http://example.com', timeout=4).status)",
        ],
    );
    assert!(
        allowed_network.status.success(),
        "approved OCI proxy request failed: {}",
        String::from_utf8_lossy(&allowed_network.stderr)
    );
    let allowed_tls = run(
        binary,
        &network_config,
        &[
            "process",
            "run",
            "/usr/local/bin/python3",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "-c",
            "import urllib.request; print(urllib.request.urlopen('https://example.com', timeout=4).status)",
        ],
    );
    assert!(
        allowed_tls.status.success(),
        "approved OCI TLS proxy request failed: {}",
        String::from_utf8_lossy(&allowed_tls.stderr)
    );
    let denied_network = run(
        binary,
        &network_config,
        &[
            "process",
            "run",
            "/usr/local/bin/python3",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "-c",
            "import urllib.request; urllib.request.urlopen('http://example.org', timeout=2)",
        ],
    );
    assert!(
        !denied_network.status.success(),
        "unapproved OCI proxy destination unexpectedly succeeded"
    );
    let bypass = run(
        binary,
        &network_config,
        &[
            "process",
            "run",
            "/usr/local/bin/python3",
            "--cwd",
            allowed.to_str().expect("allowed path"),
            "--",
            "-c",
            "import socket; socket.create_connection(('1.1.1.1', 53), 0.5)",
        ],
    );
    assert!(
        !bypass.status.success(),
        "networked OCI workload bypassed its internal proxy-only network"
    );
    assert_eq!(containers(&runtime), before, "OCI proxy container leaked");
    assert_eq!(
        networks(&runtime),
        networks_before,
        "OCI proxy network leaked"
    );

    let timeout = run(
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
        !timeout.status.success() && String::from_utf8_lossy(&timeout.stderr).contains("timeout"),
        "OCI timeout was not enforced: {}",
        String::from_utf8_lossy(&timeout.stderr)
    );
    assert_eq!(
        containers(&runtime),
        before,
        "OCI container leaked after run"
    );

    use std::os::unix::fs::PermissionsExt as _;
    let fault_helper = directory.path().join("fault-helper.py");
    fs::write(
        &fault_helper,
        format!(
            r#"#!/usr/bin/python3
import json
import subprocess
import sys
import time

document = json.load(sys.stdin)
job_id = document["job"]["job_id"]
name = "colossus-" + "".join(character for character in job_id if character.isalnum())
subprocess.run([
    {runtime:?}, "run", "--detach", "--rm", "--pull=never", "--network=none",
    "--read-only", "--cap-drop=ALL", "--name", name, "--entrypoint", "/bin/sleep",
    {image:?}, "30"
], check=True, env={{}})
time.sleep(30)
"#,
            runtime = runtime.display().to_string(),
        ),
    )
    .expect("fault helper");
    fs::set_permissions(&fault_helper, fs::Permissions::from_mode(0o700))
        .expect("fault helper permissions");
    let fault_config = directory.path().join("fault-config.yaml");
    fs::write(
        &fault_config,
        fs::read_to_string(&config).expect("read config").replace(
            "  helperPath: null",
            &format!("  helperPath: {}", fault_helper.display()),
        ),
    )
    .expect("fault config");
    let unknown = run(
        binary,
        &fault_config,
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
        !unknown.status.success()
            && String::from_utf8_lossy(&unknown.stderr).contains("OutcomeUnknown"),
        "cancelled helper was not recorded as unknown: {}",
        String::from_utf8_lossy(&unknown.stderr)
    );
    for _ in 0..30 {
        if containers(&runtime) == before {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        containers(&runtime),
        before,
        "cancellation cleanup guard leaked an OCI container"
    );
    assert_eq!(
        networks(&runtime),
        networks_before,
        "cancellation cleanup guard leaked an OCI network"
    );
    let audit = run(
        binary,
        &fault_config,
        &["audit", "show", "--from", "1", "--limit", "200"],
    );
    assert!(audit.status.success());
    assert!(
        String::from_utf8_lossy(&audit.stdout).contains("effect.outcome_unknown.v1"),
        "unknown outcome was not journaled"
    );
}
