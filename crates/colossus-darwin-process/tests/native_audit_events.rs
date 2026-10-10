#![cfg(target_os = "macos")]
//! Real scoped kernel source/export evidence, with an external native observer.
//!
//! These ignored tests require a logged-in macOS GUI user, Python 3, the native SDK
//! compiler and user launchd. They neither launch a browser nor change audit policy.
//! Run: cargo test -p colossus-darwin-process --test native_audit_events -- --ignored --nocapture
//! The outside observer is a fixed fixture, not a production receiver or containment proof.

use colossus_darwin_process::{DarwinAuditSessionEvents, RetainedDarwinAuditSession};
use std::{fs, os::fd::AsRawFd as _, path::Path, process::Command, time::Duration};

#[test]
#[ignore = "requires a logged-in GUI launchd domain and native SDK compiler"]
fn exported_kernel_end_waits_for_detached_last_child() {
    run_native("normal");
}

#[test]
#[ignore = "requires a logged-in GUI launchd domain and native SDK compiler"]
fn exported_kernel_end_survives_owned_producer_crash() {
    run_native("crash");
}

fn run_native(scenario: &str) {
    let directory = tempfile::tempdir().expect("private owned native fixture directory");
    let executable = directory.path().join("audit-events-probe");
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let compiled = Command::new("/usr/bin/xcrun")
        .args([
            "clang",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-deprecated-declarations",
        ])
        .arg(tests.join("fixtures/audit_events_probe.c"))
        .args(["-lbsm", "-o"])
        .arg(&executable)
        .output()
        .expect("native compiler for fixed owned fixture");
    assert!(
        compiled.status.success(),
        "native fixture compilation failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let output = Command::new("python3")
        .args(["-B"])
        .arg(tests.join("audit_events/run.py"))
        .arg(directory.path())
        .arg(&executable)
        .arg(std::env::current_exe().expect("fixed Rust producer executable"))
        .arg(scenario)
        .output()
        .expect("bounded owned session fixture runner");
    let receipt = fs::read_to_string(directory.path().join("receipt.json"))
        .unwrap_or_else(|_| String::from_utf8_lossy(&output.stdout).into_owned());
    println!("{receipt}");
    assert!(
        output.status.success(),
        "native kernel source/export fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "internal fresh-session producer invoked only by the two native fixtures"]
fn fixture_producer_entrypoint() {
    let Some(socket) = std::env::var_os("COLOSSUS_NATIVE_AUDIT_FIXTURE_SOCKET") else {
        return;
    };
    let service = std::env::var_os("COLOSSUS_NATIVE_AUDIT_FIXTURE_SERVICE")
        .expect("fixed private fixture service");
    let executable = std::env::var_os("COLOSSUS_NATIVE_AUDIT_FIXTURE_PROBE")
        .expect("fixed private fixture executable");
    let retained = RetainedDarwinAuditSession::current().expect("genuine retained current session");
    let mut events =
        DarwinAuditSessionEvents::current(&retained).expect("scoped readonly kernel source");
    let alive = events
        .wait_for_end(Duration::from_millis(100))
        .expect_err("a current owner prevents kernel zero-process END");
    assert_eq!(alive.kind(), std::io::ErrorKind::TimedOut);
    println!("RUST_CURRENT_OWNER_PREVENTS_END=true");
    let descriptor = events.into_owned_fd().expect("unread kernel device export");
    // SAFETY: fcntl borrows our owned valid descriptor; no descriptor is closed or
    // manufactured. Only this fixed child receives it, and ownership remains here.
    let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0, "read owned descriptor inheritance flags");
    // SAFETY: the same owned descriptor stays valid across this flag change and
    // fixed child launch. Its Rust owner closes it after the child is reaped.
    let changed = unsafe {
        libc::fcntl(
            descriptor.as_raw_fd(),
            libc::F_SETFD,
            flags & !libc::FD_CLOEXEC,
        )
    };
    assert_eq!(
        changed, 0,
        "allow fixed native child to receive exported source"
    );
    println!(
        "RUST_KERNEL_SOURCE_EXPORTED asid={} public_current_and_export=true",
        retained.audit_session_id()
    );
    let status = Command::new(executable)
        .arg("producer-api")
        .arg(socket)
        .arg(service)
        .arg(descriptor.as_raw_fd().to_string())
        .arg(std::process::id().to_string())
        .status()
        .expect("exact native source delegate");
    assert!(status.success(), "owned delegate must exit normally");
    drop(descriptor);
    drop(retained);
}
