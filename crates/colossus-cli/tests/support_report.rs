//! Exercises the standalone support report before Colossus startup.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;

use process_support::{isolate_user_home, tempdir};
use serde_json::{Value, json};
use std::{fs, net::TcpListener, process::Command};

#[test]
fn support_report_saves_before_startup_without_contact_or_extra_files() {
    let directory = tempdir().expect("private test directory");
    let provider = TcpListener::bind("127.0.0.1:0").expect("local test listener");
    provider
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let endpoint = format!("http://{}", provider.local_addr().unwrap());
    let config = directory.path().join("support-path-sentinel.yaml");
    let invalid_config = format!("[{endpoint}, support-secret-sentinel");
    fs::write(&config, &invalid_config).unwrap();
    let destination = directory.path().join("destinations");
    fs::create_dir(&destination).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
    let home = isolate_user_home(&mut command, directory.path());
    let status = command
        .current_dir(&destination)
        .env("OPENAI_API_KEY", "support-environment-secret-sentinel")
        .env("HTTP_PROXY", &endpoint)
        .env("HTTPS_PROXY", &endpoint)
        .arg("--workspace")
        .arg(directory.path().join("missing-workspace"))
        .arg("--config")
        .arg(&config)
        .args([
            "--worker-required",
            "support",
            "report",
            "--output",
            "support report.json",
        ])
        .output()
        .expect("run the offline support command");
    assert!(status.status.success(), "{status:?}");
    let bytes = fs::read(destination.join("support report.json")).unwrap();
    assert!(bytes.len() <= 64 * 1024);
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        report,
        json!({
            "schema_version": 1,
            "colossus_version": env!("CARGO_PKG_VERSION"),
            "platform": { "os": std::env::consts::OS, "architecture": std::env::consts::ARCH },
            "runtime_mode": "unresolved",
            "worker": { "status": "unavailable", "reason": "not_observed" }
        })
    );
    assert!(std::str::from_utf8(&bytes).unwrap().ends_with('\n'));
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
    assert_eq!(fs::read_to_string(config).unwrap(), invalid_config);
    assert!(!home.colossus_home().exists());
    assert!(!directory.path().join("missing-workspace").exists());
    assert_eq!(
        provider.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
