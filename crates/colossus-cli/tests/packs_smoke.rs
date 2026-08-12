//! Credential-free end-to-end pack lifecycle and effect-audit smoke test.
#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "support/process.rs"]
mod process_support;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{BundleFileEntry, BundleManifest, PackSignature};
use colossus_packs::canonical_bundle_signing_bytes;
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "9999999999999999999999999999999999999999999999999999999999999999";
const SIGNING_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(binary);
    process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_PACK_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_PACK_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn unsigned_pack_requires_override_and_lifecycle_is_permit_bound_and_audited() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let mcp_server = Path::new(env!("CARGO_BIN_EXE_colossus-mcp-test-server"));
    let directory = tempdir().expect("directory");
    let source = directory.path().join("source-pack");
    let docs = source.join("docs");
    let bin = source.join("bin");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&docs).expect("pack docs");
    fs::create_dir_all(&bin).expect("pack bin");
    fs::create_dir_all(&workflows).expect("workflows");
    let body = b"offline pack evidence\n";
    fs::write(docs.join("README.md"), body).expect("pack body");
    let executable_name = if cfg!(windows) {
        "colossus-pack-tool.exe"
    } else {
        "colossus-pack-tool"
    };
    let executable_path = bin.join(executable_name);
    fs::copy(binary, &executable_path).expect("copy executable fixture");
    let executable = fs::read(&executable_path).expect("read executable fixture");
    let executable_relative = format!("bin/{executable_name}");
    let mcp_name = "colossus-pack-mcp";
    let mcp_path = bin.join(mcp_name);
    fs::copy(mcp_server, &mcp_path).expect("copy MCP fixture");
    let mcp_binary = fs::read(&mcp_path).expect("read MCP fixture");
    let mcp_relative = format!("bin/{mcp_name}");
    fs::write(
        source.join("colossus.pack.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "format_version": 1,
            "name": "smoke-pack",
            "version": "0.1.0",
            "description": "Pack smoke fixture.",
            "publisher": "example",
            "license": "Apache-2.0",
            "capabilities": ["docs", "tools", "mcp_servers", "binaries"],
            "permissions": ["process"],
            "files": [
                {
                    "path": "docs/README.md",
                    "sha256": hex::encode(Sha256::digest(body)),
                    "size": body.len(),
                    "content_type": "text/markdown"
                },
                {
                    "path": executable_relative,
                    "sha256": hex::encode(Sha256::digest(&executable)),
                    "size": executable.len(),
                    "content_type": "application/octet-stream"
                },
                {
                    "path": mcp_relative,
                    "sha256": hex::encode(Sha256::digest(&mcp_binary)),
                    "size": mcp_binary.len(),
                    "content_type": "application/octet-stream"
                }
            ],
            "tools": [{
                "name": "demo.fixed",
                "command": executable_relative,
                "args": ["--version"],
                "env_refs": {},
                "permissions": ["process"]
            }],
            "mcp_servers": [{
                "name": "pack-fixture",
                "command": mcp_relative,
                "args": [],
                "env_refs": {},
                "allowed_tools": ["echo"],
                "permissions": ["process"]
            }],
            "binaries": [executable_relative, mcp_relative],
            "docs": ["docs/README.md"]
        }))
        .expect("manifest"),
    )
    .expect("write manifest");

    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    let install_root = directory.path().join("installed-packs");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PACK_TEST_JOURNAL_KEY
    journal_key_id: pack-test-journal-v1
    signing_variable: COLOSSUS_PACK_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
packs:
  installRoot: {install_root}
sandbox:
  backend: native
  profile: pack-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
  - root: {workspace}
    mode: write
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 1
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            install_root = install_root.display(),
            workspace = directory.path().display(),
        ),
    )
    .expect("config");

    let verify = run(
        binary,
        &config,
        directory.path(),
        &["packs", "verify", source.to_str().expect("source")],
    );
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let verify: Value = serde_json::from_slice(&verify.stdout).expect("verify JSON");
    assert_eq!(verify["trusted"], false);

    let denied = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "packs",
            "install",
            source.to_str().expect("source"),
        ],
    );
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("allow_untrusted"));

    let install = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "packs",
            "install",
            source.to_str().expect("source"),
            "--allow-untrusted",
        ],
    );
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    let installed: Value = serde_json::from_slice(&install.stdout).expect("install JSON");
    assert_eq!(installed["status"], "enabled");

    let call = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "packs",
            "call",
            "demo.fixed",
        ],
    );
    assert!(
        call.status.success(),
        "{}",
        String::from_utf8_lossy(&call.stderr)
    );
    let call: Value = serde_json::from_slice(&call.stdout).expect("call JSON");
    assert_eq!(call["exit_code"], 0);
    assert!(
        call["stdout"]
            .as_str()
            .is_some_and(|output| output.starts_with("colossus "))
    );

    let servers = run(binary, &config, directory.path(), &["mcp", "servers"]);
    assert!(
        servers.status.success(),
        "{}",
        String::from_utf8_lossy(&servers.stderr)
    );
    assert!(String::from_utf8_lossy(&servers.stdout).contains("pack-fixture"));
    let tools = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "mcp",
            "tools",
            "--server",
            "pack-fixture",
        ],
    );
    assert!(
        tools.status.success(),
        "{}",
        String::from_utf8_lossy(&tools.stderr)
    );
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("MCP tools JSON");
    assert_eq!(tools[0]["name"], "echo");
    let mcp_call = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "mcp",
            "call",
            "pack-fixture",
            "echo",
            r#"{"text":"pack mcp"}"#,
        ],
    );
    assert!(
        mcp_call.status.success(),
        "{}",
        String::from_utf8_lossy(&mcp_call.stderr)
    );
    assert!(String::from_utf8_lossy(&mcp_call.stdout).contains("pack mcp"));

    let bundle = directory.path().join("offline-bundle");
    fs::create_dir(&bundle).expect("bundle");
    let artifact = b"offline release artifact";
    fs::write(bundle.join("artifact.bin"), artifact).expect("bundle artifact");
    let signing_key = SigningKey::from_bytes(&[12_u8; 32]);
    let public_key = signing_key.verifying_key().to_bytes();
    let key_id = hex::encode(Sha256::digest(public_key));
    let trust = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "packs",
            "trust",
            "add",
            "colossus",
            "--public-key",
            &BASE64.encode(public_key),
        ],
    );
    assert!(
        trust.status.success(),
        "{}",
        String::from_utf8_lossy(&trust.stderr)
    );
    let mut bundle_manifest = BundleManifest {
        format_version: 1,
        name: "colossus-offline".into(),
        version: "0.6.0".into(),
        publisher: "colossus".into(),
        created_at: "2026-07-11T00:00:00Z".into(),
        source_revision: Some("pack-smoke".into()),
        files: vec![BundleFileEntry {
            path: "artifact.bin".into(),
            sha256: hex::encode(Sha256::digest(artifact)),
            size: Some(artifact.len() as u64),
        }],
        signatures: Vec::new(),
    };
    let unsigned = canonical_bundle_signing_bytes(&bundle_manifest).expect("unsigned manifest");
    bundle_manifest.signatures.push(PackSignature {
        algorithm: "ed25519".into(),
        key_id: key_id.clone(),
        signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
    });
    fs::write(
        bundle.join("manifest.json"),
        serde_json::to_vec_pretty(&bundle_manifest).expect("bundle manifest"),
    )
    .expect("write bundle manifest");
    let bundle_verify = run(
        binary,
        &config,
        directory.path(),
        &["bundle", "verify", bundle.to_str().expect("bundle path")],
    );
    assert!(
        bundle_verify.status.success(),
        "{}",
        String::from_utf8_lossy(&bundle_verify.stderr)
    );
    let bundle_verify: Value =
        serde_json::from_slice(&bundle_verify.stdout).expect("bundle evidence");
    assert_eq!(bundle_verify["trust_key_id"], key_id);

    for (action, expected) in [
        ("disable", "disabled"),
        ("enable", "enabled"),
        ("uninstall", "uninstalled"),
    ] {
        let output = run(
            binary,
            &config,
            directory.path(),
            &[
                "--approval-mode",
                "full-access",
                "packs",
                action,
                "smoke-pack",
            ],
        );
        assert!(
            output.status.success(),
            "{}: {}",
            action,
            String::from_utf8_lossy(&output.stderr)
        );
        let output: Value = serde_json::from_slice(&output.stdout).expect("lifecycle JSON");
        assert_eq!(output["status"], expected);
    }

    let list = run(binary, &config, directory.path(), &["packs", "list"]);
    assert!(list.status.success());
    let list: Value = serde_json::from_slice(&list.stdout).expect("list JSON");
    assert_eq!(list[0]["status"], "uninstalled");

    let audit = run(
        binary,
        &config,
        directory.path(),
        &["audit", "show", "--limit", "200"],
    );
    assert!(audit.status.success());
    let audit: Value = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let event_types = audit
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    for expected in [
        "pack.installed.v1",
        "pack.disabled.v1",
        "pack.enabled.v1",
        "pack.uninstalled.v1",
    ] {
        assert!(event_types.contains(&expected), "missing {expected}");
    }
    assert!(
        event_types
            .iter()
            .filter(|event| **event == "effect.completed.v1")
            .count()
            >= 11
    );
}
