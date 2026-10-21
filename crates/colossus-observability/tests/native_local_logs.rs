//! Native host composition exercises public APIs in isolated subscriber processes.
#![cfg(feature = "host-exporters")]

use colossus_home::{ColossusHome, ConfinedRoot};
use colossus_observability::{
    LocalLogLevel, LocalLogSink, ObservabilityConfig, ObservabilityGuard,
};
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const CASE_VARIABLE: &str = "COLOSSUS_NATIVE_LOG_TEST_CASE";
const ROOT_VARIABLE: &str = "COLOSSUS_NATIVE_LOG_TEST_ROOT";
const BOOTSTRAP_METADATA: &str = "native-bootstrap-test-metadata";

#[cfg(windows)]
struct PrivateDirectory {
    path: PathBuf,
    parent: PathBuf,
}

#[cfg(windows)]
impl PrivateDirectory {
    fn new() -> Self {
        let parent = colossus_windows_native::current_user_profile_directory()
            .expect("native test profile")
            .canonicalize()
            .expect("canonical test profile");
        let path = parent.join(format!("colossus-host-logs-test-{}", uuid::Uuid::now_v7()));
        colossus_windows_native::create_private_directory(&path).expect("private test directory");
        Self { path, parent }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(windows)]
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        use std::os::windows::fs::MetadataExt as _;
        let metadata = std::fs::symlink_metadata(&self.path).expect("test directory metadata");
        assert_eq!(
            metadata.file_attributes() & 0x400,
            0,
            "test directory became a reparse point"
        );
        let canonical = self.path.canonicalize().expect("canonical test directory");
        assert_eq!(canonical, self.path);
        assert_eq!(canonical.parent(), Some(self.parent.as_path()));
        assert!(
            canonical
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("colossus-host-logs-test-")
        );
        std::fs::remove_dir_all(canonical)
            .expect("remove only the generated private test directory");
    }
}

#[cfg(not(windows))]
type PrivateDirectory = tempfile::TempDir;

fn private_directory() -> PrivateDirectory {
    #[cfg(windows)]
    let directory = PrivateDirectory::new();
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().expect("private test directory");
    directory
}

fn child(case: &str, root: Option<&Path>) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args(["--exact", "native_logging_child", "--nocapture"])
        .env(CASE_VARIABLE, case)
        .env_remove("OTEL_SDK_DISABLED")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = root {
        command.env(ROOT_VARIABLE, root);
    }
    let mut child = command.spawn().expect("spawn isolated native host logger");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().expect("native host status").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("native host logger exceeded its bounded shutdown deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("native host output");
    assert!(
        output.status.success(),
        "native host failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn disabled_otlp_still_installs_and_flushes_private_native_logs() {
    let directory = private_directory();
    let output = child("disabled_local", Some(directory.path()));
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains(BOOTSTRAP_METADATA));
    assert!(!stdout.contains("host.local_logging"));
    assert!(output.stderr.is_empty());
    let text = std::fs::read_to_string(directory.path().join("home/logs/sidecar-0.jsonl"))
        .expect("flushed private native logs");
    let records = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert!(text.contains("host.local_logging.info"));
    assert!(text.contains("host.local_logging.debug"));
    assert!(!text.contains("PRIVATE"));
}

#[test]
fn native_console_json_reserves_stdout_even_without_a_local_sink() {
    let output = child("console_only", None);
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains(BOOTSTRAP_METADATA));
    assert!(!stdout.contains("host.console_logging"));
    assert!(!stdout.contains("\"event_name\""));
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    let records = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["event_name"], "host.console_logging");
}

#[test]
fn native_logging_child() {
    let Ok(case) = std::env::var(CASE_VARIABLE) else {
        return;
    };
    match case.as_str() {
        "disabled_local" => {
            let directory =
                PathBuf::from(std::env::var_os(ROOT_VARIABLE).expect("test-owned root"));
            let home = ColossusHome::ensure_at(directory.join("home")).expect("private home");
            let root = ConfinedRoot::bind(home.root()).expect("native root");
            let logs =
                LocalLogSink::open(&root, "sidecar", LocalLogLevel::Debug).expect("local sink");
            let guard =
                ObservabilityGuard::install_native(&ObservabilityConfig::default(), Some(logs))
                    .expect("disabled exporter native composition")
                    .expect("local sink retains the native subscriber");
            {
                let span = tracing::info_span!("private", body = "PRIVATE_PARENT_FIELD");
                let _entered = span.enter();
                tracing::info!(target: "colossus.lifecycle", event_name = "host.local_logging.info");
                tracing::debug!(target: "colossus.provider", event_name = "host.local_logging.debug");
                tracing::error!(target: "reqwest", body = "PRIVATE_DEPENDENCY_BODY");
                tracing::info!(target: "colossus.journal", body = "PRIVATE_JOURNAL_BODY");
            }
            drop(guard);
            // Drop must drain the writer and release its file lock even though
            // the process-global subscriber still retains its sender clone.
            let _reopened = LocalLogSink::open(&root, "sidecar", LocalLogLevel::Info)
                .expect("native guard released local log custody");
        }
        "console_only" => {
            let mut config = ObservabilityConfig {
                enabled: true,
                ..ObservabilityConfig::default()
            };
            config.logs.stdout_json = true;
            let guard = ObservabilityGuard::install_native(&config, None)
                .expect("native console composition")
                .expect("console sink retains the native subscriber");
            tracing::info!(target: "colossus.lifecycle", event_name = "host.console_logging");
            drop(guard);
        }
        _ => panic!("unknown fixed native log test case"),
    }
    println!("{BOOTSTRAP_METADATA}");
    std::io::stdout()
        .flush()
        .expect("flush bootstrap test metadata");
}
