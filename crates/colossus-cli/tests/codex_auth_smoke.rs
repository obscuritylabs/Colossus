//! End-to-end Codex account-command readiness checks with an isolated fake CLI.

#![cfg(unix)]

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

const TEST_REFRESH_TOKEN: &str = "codex-readiness-test-refresh-token";
const TEST_ID_TOKEN: &str = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiY29kZXgtcmVhZGluZXNzLXRlc3QtYWNjb3VudCJ9fQ.signature";
const TEST_ACCESS_TOKEN: &str = "header.eyJleHAiOjQxMDI0NDQ4MDB9.signature";

fn write_auth(codex_home: &Path, mode: u32) {
    fs::create_dir_all(codex_home).expect("create isolated Codex home");
    fs::set_permissions(codex_home, fs::Permissions::from_mode(0o700))
        .expect("make isolated Codex home private");
    let auth_path = codex_home.join("auth.json");
    fs::write(
        &auth_path,
        serde_json::to_vec(&json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": TEST_ID_TOKEN,
                "access_token": TEST_ACCESS_TOKEN,
                "refresh_token": TEST_REFRESH_TOKEN,
                "account_id": "codex-readiness-test-account"
            }
        }))
        .expect("serialize isolated Codex auth"),
    )
    .expect("write isolated Codex auth");
    fs::set_permissions(auth_path, fs::Permissions::from_mode(mode))
        .expect("set isolated Codex auth permissions");
}

fn fake_codex(root: &Path, remove_auth_on_logout: bool) -> PathBuf {
    let executable = root.join(if remove_auth_on_logout {
        "fake-codex-removes-auth"
    } else {
        "fake-codex-noop"
    });
    let logout = if remove_auth_on_logout {
        r#"
if [ "${3:-}" = "logout" ]; then
  /bin/rm -f -- "$CODEX_HOME/auth.json"
fi
"#
    } else {
        ""
    };
    fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
if [ "${{1:-}}" != "-c" ] || [ "${{2:-}}" != 'cli_auth_credentials_store="file"' ]; then
  exit 64
fi
if [ -n "${{COLOSSUS_TEST_CODEX_MARKER:-}}" ]; then
  : > "$COLOSSUS_TEST_CODEX_MARKER"
fi
{logout}exit 0
"#
        ),
    )
    .expect("write fake Codex executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("make fake Codex executable private");
    executable
}

fn run_codex(
    root: &Path,
    codex_home: impl AsRef<std::ffi::OsStr>,
    executable: &Path,
    action: &str,
) -> Output {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let mut command = Command::new(binary);
    let _isolated_home = process_support::isolate_user_home(&mut command, root);
    command
        .current_dir(root)
        .env("CODEX_HOME", codex_home)
        .env("COLOSSUS_TEST_CODEX_MARKER", root.join("codex-invoked"))
        .args([
            "--output",
            "json",
            "codex",
            "--codex-bin",
            executable.to_str().expect("fake Codex path is UTF-8"),
            action,
        ])
        .output()
        .expect("run isolated Codex account command")
}

fn assert_no_test_credential(output: &Output) {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains(TEST_REFRESH_TOKEN));
    assert!(!combined.contains(TEST_ID_TOKEN));
    assert!(!combined.contains(TEST_ACCESS_TOKEN));
}

#[test]
fn login_and_status_complete_only_for_a_runtime_usable_credential() {
    for action in ["login", "status"] {
        let directory = tempdir().expect("isolated command directory");
        let codex_home = directory.path().join("codex-home");
        write_auth(&codex_home, 0o600);
        let executable = fake_codex(directory.path(), false);
        let output = run_codex(directory.path(), &codex_home, &executable, action);

        assert!(
            output.status.success(),
            "{action} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).expect("JSON account report");
        assert_eq!(report["completed"], true);
        assert_eq!(report["credential_store"], "file");
        assert!(directory.path().join("codex-invoked").is_file());
        assert_no_test_credential(&output);
    }
}

#[test]
fn login_and_status_reject_a_group_readable_credential_before_cli_start() {
    for action in ["login", "status"] {
        let directory = tempdir().expect("isolated command directory");
        let codex_home = directory.path().join("codex-home");
        write_auth(&codex_home, 0o644);
        let executable = fake_codex(directory.path(), false);
        let output = run_codex(directory.path(), &codex_home, &executable, action);

        assert!(
            !output.status.success(),
            "unsafe {action} unexpectedly passed"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("credential store is unsafe"), "{stderr}");
        assert!(
            !directory.path().join("codex-invoked").exists(),
            "unsafe auth must fail before the external CLI starts"
        );
        assert!(!stderr.contains(codex_home.to_string_lossy().as_ref()));
        assert_no_test_credential(&output);
    }
}

#[test]
fn relative_codex_home_is_rejected_before_the_cli_starts() {
    let directory = tempdir().expect("isolated command directory");
    let relative_home = Path::new("relative-codex-home");
    write_auth(&directory.path().join(relative_home), 0o600);
    let executable = fake_codex(directory.path(), false);
    let output = run_codex(
        directory.path(),
        relative_home.as_os_str(),
        &executable,
        "status",
    );

    assert!(!output.status.success());
    assert!(
        !directory.path().join("codex-invoked").exists(),
        "relative CODEX_HOME must fail before the external CLI starts"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CODEX_HOME must be absolute"), "{stderr}");
    assert!(!stderr.contains(directory.path().to_string_lossy().as_ref()));
    assert_no_test_credential(&output);
}

#[test]
fn status_rejects_an_auth_fifo_without_blocking() {
    let directory = tempdir().expect("isolated command directory");
    let codex_home = directory.path().join("codex-home");
    fs::create_dir(&codex_home).expect("create isolated Codex home");
    fs::set_permissions(&codex_home, fs::Permissions::from_mode(0o700))
        .expect("make isolated Codex home private");
    assert!(
        Command::new("mkfifo")
            .arg(codex_home.join("auth.json"))
            .status()
            .expect("run mkfifo")
            .success()
    );
    let executable = fake_codex(directory.path(), false);
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let mut command = Command::new(binary);
    let _isolated_home = process_support::isolate_user_home(&mut command, directory.path());
    let mut child = command
        .current_dir(directory.path())
        .env("CODEX_HOME", &codex_home)
        .env(
            "COLOSSUS_TEST_CODEX_MARKER",
            directory.path().join("codex-invoked"),
        )
        .args([
            "--output",
            "json",
            "codex",
            "--codex-bin",
            executable.to_str().expect("fake Codex path is UTF-8"),
            "status",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn isolated Codex status");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child
            .try_wait()
            .expect("poll isolated Codex status")
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop blocked Codex status");
            let _ = child.wait();
            panic!("Codex status blocked while opening an auth FIFO");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child
        .wait_with_output()
        .expect("collect Codex status output");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("credential store is unsafe"), "{stderr}");
    assert!(
        !directory.path().join("codex-invoked").exists(),
        "auth FIFO must fail before the external CLI starts"
    );
    assert_no_test_credential(&output);
}

#[test]
fn status_rejects_missing_auth_before_cli_start() {
    let directory = tempdir().expect("isolated command directory");
    let codex_home = directory.path().join("codex-home");
    fs::create_dir(&codex_home).expect("create isolated Codex home");
    fs::set_permissions(&codex_home, fs::Permissions::from_mode(0o700))
        .expect("make isolated Codex home private");
    let executable = fake_codex(directory.path(), false);
    let output = run_codex(directory.path(), &codex_home, &executable, "status");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no runtime-usable"), "{stderr}");
    assert!(
        !directory.path().join("codex-invoked").exists(),
        "missing status auth must fail before the external CLI starts"
    );
    assert_no_test_credential(&output);
}

#[test]
fn login_requires_a_usable_credential_after_cli_success() {
    let directory = tempdir().expect("isolated command directory");
    let codex_home = directory.path().join("codex-home");
    fs::create_dir(&codex_home).expect("create isolated Codex home");
    fs::set_permissions(&codex_home, fs::Permissions::from_mode(0o700))
        .expect("make isolated Codex home private");
    let executable = fake_codex(directory.path(), false);
    let output = run_codex(directory.path(), &codex_home, &executable, "login");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("did not produce a usable"), "{stderr}");
    assert!(
        directory.path().join("codex-invoked").is_file(),
        "login may start from an unavailable store and must validate its postcondition"
    );
    assert_no_test_credential(&output);
}

#[test]
fn logout_requires_the_usable_credential_to_be_gone() {
    let directory = tempdir().expect("isolated command directory");
    let codex_home = directory.path().join("codex-home");
    write_auth(&codex_home, 0o600);
    let executable = fake_codex(directory.path(), false);
    let output = run_codex(directory.path(), &codex_home, &executable, "logout");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("credential remains"), "{stderr}");
    assert_no_test_credential(&output);
}

#[test]
fn logout_accepts_verified_removal_but_not_an_unsafe_remaining_store() {
    let removed = tempdir().expect("isolated removal directory");
    let removed_home = removed.path().join("codex-home");
    write_auth(&removed_home, 0o600);
    let removing_executable = fake_codex(removed.path(), true);
    let removed_output = run_codex(
        removed.path(),
        &removed_home,
        &removing_executable,
        "logout",
    );
    assert!(
        removed_output.status.success(),
        "verified logout failed: {}",
        String::from_utf8_lossy(&removed_output.stderr)
    );
    let report: Value = serde_json::from_slice(&removed_output.stdout).expect("JSON logout report");
    assert_eq!(report["completed"], true);
    assert_no_test_credential(&removed_output);

    let unsafe_store = tempdir().expect("isolated unsafe directory");
    let unsafe_home = unsafe_store.path().join("codex-home");
    write_auth(&unsafe_home, 0o644);
    let noop_executable = fake_codex(unsafe_store.path(), false);
    let unsafe_output = run_codex(
        unsafe_store.path(),
        &unsafe_home,
        &noop_executable,
        "logout",
    );
    assert!(!unsafe_output.status.success());
    let stderr = String::from_utf8_lossy(&unsafe_output.stderr);
    assert!(stderr.contains("credential store is unsafe"), "{stderr}");
    assert!(
        !unsafe_store.path().join("codex-invoked").exists(),
        "unsafe logout auth must fail before the external CLI starts"
    );
    assert!(!stderr.contains(unsafe_home.to_string_lossy().as_ref()));
    assert_no_test_credential(&unsafe_output);
}
