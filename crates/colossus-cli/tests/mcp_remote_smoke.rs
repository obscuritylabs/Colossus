//! Opt-in credential-free acceptance against a public Streamable HTTP MCP server.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

#[path = "support/process.rs"]
mod process_support;

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn run(config: &Path, arguments: &[&str]) -> std::process::Output {
    let workspace = config.parent().expect("workspace");
    let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
    let _home = process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .output()
        .expect("run Colossus")
}

#[test]
#[ignore = "requires public HTTPS access to docs.mcp.cloudflare.com"]
fn cloudflare_docs_discovery_and_call_require_stateless_opt_in() {
    let directory = process_support::tempdir().expect("private workspace");
    let workspace = directory.path().canonicalize().expect("workspace");
    let config = workspace.join("config.yaml");
    let mut settings = json!({
        "schemaVersion": 3,
        "storage": { "path": workspace.join("state.redb"), "keys": {"kind": "none"} },
        "access": {"profile": "development"},
        "mcp": {"servers": {"cloudflare-docs": {
            "transport": "streamable_http",
            "url": "https://docs.mcp.cloudflare.com/mcp",
            "allowedTools": ["search_cloudflare_documentation"],
            "allowStateless": false,
            "timeoutMs": 30000,
            "maxOutputBytes": 1048576
        }}},
        "sandbox": {
            "backend": if cfg!(windows) { "windows_job" } else { "native" },
            "networkDestinations": ["https://docs.mcp.cloudflare.com"],
            "timeoutMs": 30000,
            "maxOutputBytes": 1048576
        }
    });
    fs::write(&config, settings.to_string()).expect("configuration");
    let rejected = run(&config, &["mcp", "tools"]);
    assert!(
        !rejected.status.success(),
        "stateless server requires opt-in"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("initialization failed"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    settings["mcp"]["servers"]["cloudflare-docs"]["allowStateless"] = json!(true);
    fs::write(&config, settings.to_string()).expect("stateless configuration");
    let discovered = run(&config, &["mcp", "tools"]);
    assert!(
        discovered.status.success(),
        "{}",
        String::from_utf8_lossy(&discovered.stderr)
    );
    let tools: Value = serde_json::from_slice(&discovered.stdout).expect("discovered tools");
    assert_eq!(tools[0]["name"], "search_cloudflare_documentation");

    let called = run(
        &config,
        &[
            "--approval-mode",
            "full-access",
            "mcp",
            "call",
            "cloudflare-docs",
            "search_cloudflare_documentation",
            r#"{"query":"What are Cloudflare Workers?"}"#,
        ],
    );
    assert!(
        called.status.success(),
        "{}",
        String::from_utf8_lossy(&called.stderr)
    );
    let result: Value = serde_json::from_slice(&called.stdout).expect("tool result");
    assert_eq!(result["server"], "cloudflare-docs");
    assert_eq!(result["tool"], "search_cloudflare_documentation");
    assert_ne!(result["result"]["isError"], json!(true), "tool succeeded");
    assert!(
        result["result"]["content"]
            .as_array()
            .expect("content blocks")
            .iter()
            .any(|block| block["text"]
                .as_str()
                .is_some_and(|text| text.contains("Workers"))),
        "expected documentation result"
    );

    let audit = run(&config, &["audit", "verify"]);
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
}
