//! End-to-end strict configuration and secret-safe diagnostic acceptance.

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const JOURNAL_SECRET: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SIGNING_SECRET: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const PROVIDER_SECRET: &str =
    "config-provider-secret-333333333333333333333333333333333333333333333333";
const RAW_CONFIG_SECRET: &str =
    "raw-config-secret-444444444444444444444444444444444444444444444444";

fn command(binary: &Path, config: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .args(["--config", config.to_str().expect("config path")])
        .env("COLOSSUS_CONFIG_DISPLAY_JOURNAL_SECRET", JOURNAL_SECRET)
        .env("COLOSSUS_CONFIG_DISPLAY_SIGNING_SECRET", SIGNING_SECRET)
        .env("COLOSSUS_CONFIG_DISPLAY_PROVIDER_SECRET", PROVIDER_SECRET);
    command
}

fn config_document(root: &Path) -> Value {
    json!({
        "schemaVersion": 1,
        "storage": {
            "path": root.join("state.redb"),
            "keys": {
                "kind": "environment",
                "journal_variable": "COLOSSUS_CONFIG_DISPLAY_JOURNAL_SECRET",
                "journal_key_id": "config-security-journal-v1",
                "signing_variable": "COLOSSUS_CONFIG_DISPLAY_SIGNING_SECRET",
                "anchor_path": root.join("anchor.json")
            }
        },
        "access": {
            "profile": "pinned",
            "tools": {"include": ["echo"], "exclude": []},
            "actions": {
                "allow": ["provider.openai.chat"],
                "requireApproval": [],
                "deny": []
            }
        },
        "policy": {"kind": "built_in", "require_post_effect": true},
        "workflows": {
            "repository": root.join("workflows"),
            "user": root.join("workflows")
        },
        "providers": {
            "profiles": {
                "hosted": {
                    "kind": "open_ai_compatible",
                    "model": "config-security-model",
                    "baseUrl": "https://example.com/v1",
                    "credentialReference": "env:COLOSSUS_CONFIG_DISPLAY_PROVIDER_SECRET",
                    "timeoutMs": 5000
                }
            },
            "roles": {"primary": "hosted"}
        },
        "agent": {"maxTurns": 2},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native",
            "profile": "config-security-v1",
            "allowBrokerFallback": false,
            "helperPath": null,
            "ociRuntime": null,
            "ociImage": null,
            "ociProxyImage": null,
            "filesystem": [],
            "executables": [],
            "environment": [],
            "networkDestinations": ["https://example.com"],
            "timeoutMs": 5000,
            "maxOutputBytes": 1048576,
            "maxProcesses": 1,
            "maxMemoryBytes": 67108864,
            "maxConcurrency": 1
        }
    })
}

fn assert_secrets_absent(bytes: &[u8]) {
    for secret in [
        JOURNAL_SECRET,
        SIGNING_SECRET,
        PROVIDER_SECRET,
        RAW_CONFIG_SECRET,
    ] {
        assert!(
            !bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()),
            "configuration output disclosed {secret}"
        );
    }
}

#[test]
fn config_show_preserves_only_references_and_unknown_secret_fields_fail_without_disclosure() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    fs::create_dir(directory.path().join("workflows")).expect("workflows");

    let config = directory.path().join("config.json");
    fs::write(
        &config,
        serde_json::to_vec_pretty(&config_document(directory.path())).expect("config JSON"),
    )
    .expect("write config");
    let shown = command(binary, &config)
        .args(["config", "show"])
        .output()
        .expect("config show");
    assert!(
        shown.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&shown.stdout),
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_secrets_absent(&shown.stdout);
    assert_secrets_absent(&shown.stderr);
    let rendered = String::from_utf8(shown.stdout).expect("UTF-8 config");
    for reference in [
        "journal_variable: COLOSSUS_CONFIG_DISPLAY_JOURNAL_SECRET",
        "signing_variable: COLOSSUS_CONFIG_DISPLAY_SIGNING_SECRET",
        "credentialReference: env:COLOSSUS_CONFIG_DISPLAY_PROVIDER_SECRET",
    ] {
        assert!(
            rendered.contains(reference),
            "missing reference: {reference}"
        );
    }

    let effective = command(binary, &config)
        .args(["config", "effective"])
        .output()
        .expect("effective config");
    assert!(
        effective.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&effective.stdout),
        String::from_utf8_lossy(&effective.stderr)
    );
    assert_secrets_absent(&effective.stdout);
    assert_secrets_absent(&effective.stderr);
    let effective: Value =
        serde_json::from_slice(&effective.stdout).expect("effective access JSON");
    assert_eq!(effective["profile"], "pinned");
    assert!(
        effective["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| {
                tool["name"] == "echo"
                    && tool["availability"] == "active"
                    && tool["selection_reason"] == "explicit include"
            }))
    );

    let mut invalid = config_document(directory.path());
    invalid["providers"]["profiles"]["hosted"]["apiKey"] = json!(RAW_CONFIG_SECRET);
    let invalid_config = directory.path().join("invalid-config.json");
    fs::write(
        &invalid_config,
        serde_json::to_vec_pretty(&invalid).expect("invalid config JSON"),
    )
    .expect("write invalid config");
    let rejected = command(binary, &invalid_config)
        .args(["config", "show"])
        .output()
        .expect("invalid config show");
    assert!(!rejected.status.success(), "unknown apiKey was accepted");
    assert_secrets_absent(&rejected.stdout);
    assert_secrets_absent(&rejected.stderr);
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .to_ascii_lowercase()
            .contains("unknown field"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
}
