//! Signed offline bundle production and clean-prefix installation acceptance.

use colossus_packs::current_release_target;
use serde_json::Value;
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "9191919191919191919191919191919191919191919191919191919191919191";
const CHECKPOINT_KEY: &str = "9292929292929292929292929292929292929292929292929292929292929292";
const SIGNING_SEED: &str = "9393939393939393939393939393939393939393939393939393939393939393";

fn command(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .env_clear()
        .env("HOME", root)
        .env("COLOSSUS_BUNDLE_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_BUNDLE_TEST_CHECKPOINT_KEY", CHECKPOINT_KEY)
        .env("COLOSSUS_BUNDLE_TEST_SIGNING_SEED", SIGNING_SEED)
        .args(["--config", "config.yaml"]);
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn artifact_path(staged: &Path, target: &str) -> PathBuf {
    staged
        .join("artifacts")
        .join(target)
        .join(if target.ends_with("windows-msvc") {
            "colossus.exe"
        } else {
            "colossus"
        })
}

#[test]
fn trusted_bundle_is_built_verified_installed_and_executed_without_network() {
    let source_binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let root = fs::canonicalize(directory.path()).expect("canonical root");
    let staged = root.join("staged");
    let bundle = root.join("bundle");
    let prefix = root.join("prefix");
    let target = current_release_target().expect("current release target");
    let artifact = artifact_path(&staged, target);
    fs::create_dir_all(artifact.parent().expect("artifact parent")).expect("artifact directory");
    fs::copy(source_binary, &artifact).expect("stage binary");
    fs::write(staged.join("LICENSE"), b"Apache-2.0\n").expect("stage license");
    fs::write(
        root.join("config.yaml"),
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_BUNDLE_TEST_JOURNAL_KEY
    journal_key_id: bundle-smoke-journal-v1
    signing_variable: COLOSSUS_BUNDLE_TEST_CHECKPOINT_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: []
  approval_actions: []
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    echo:
      kind: echo
      model: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 5000
  roles:
    primary: echo
agent:
  maxTurns: 2
  tools: [echo]
subagents:
  maxConcurrent: 1
sandbox:
  backend: native
  profile: bundle-distribution-smoke-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {root}
      mode: write
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 30000
  maxOutputBytes: 1048576
  maxProcesses: 1
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = root.join("state.redb").display(),
            anchor = root.join("anchor.json").display(),
            workflows = root.join("workflows").display(),
            root = root.display(),
        ),
    )
    .expect("config");
    fs::create_dir(root.join("workflows")).expect("workflows");

    let key_info = command(source_binary, &root)
        .args([
            "--approval-mode",
            "full-access",
            "bundle",
            "key-info",
            "--signing-key-reference",
            "env:COLOSSUS_BUNDLE_TEST_SIGNING_SEED",
        ])
        .output()
        .expect("derive signing key info");
    assert_success(&key_info);
    assert!(!String::from_utf8_lossy(&key_info.stdout).contains(SIGNING_SEED));
    assert!(!String::from_utf8_lossy(&key_info.stderr).contains(SIGNING_SEED));
    let key_info = json(&key_info);
    let public_key = key_info["public_key"].as_str().expect("public key");
    assert_eq!(key_info["key_id"].as_str().expect("key id").len(), 64);
    let trust = command(source_binary, &root)
        .args([
            "--approval-mode",
            "full-access",
            "packs",
            "trust",
            "add",
            "colossus",
        ])
        .args(["--public-key", public_key])
        .output()
        .expect("add trust");
    assert_success(&trust);

    let blocked_root = tempdir().expect("blocked root");
    let blocked_destination = fs::canonicalize(blocked_root.path())
        .expect("canonical blocked root")
        .join("bundle");
    let blocked = command(source_binary, &root)
        .args([
            "--approval-mode",
            "full-access",
            "bundle",
            "build",
            staged.to_str().expect("staged path"),
            blocked_destination.to_str().expect("blocked path"),
            "--name",
            "colossus-offline",
            "--version",
            "0.6.0",
            "--publisher",
            "colossus",
            "--created-at",
            "2026-07-11T00:00:00Z",
            "--signing-key-reference",
            "env:COLOSSUS_BUNDLE_TEST_SIGNING_SEED",
        ])
        .output()
        .expect("blocked bundle build");
    assert!(!blocked.status.success());
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("outside policy-authorized write roots")
    );
    assert!(!blocked_destination.exists());

    let build = command(source_binary, &root)
        .args([
            "--approval-mode",
            "full-access",
            "bundle",
            "build",
            staged.to_str().expect("staged path"),
            bundle.to_str().expect("bundle path"),
            "--name",
            "colossus-offline",
            "--version",
            "0.6.0",
            "--publisher",
            "colossus",
            "--created-at",
            "2026-07-11T00:00:00Z",
            "--source-revision",
            "0123456789abcdef",
            "--signing-key-reference",
            "env:COLOSSUS_BUNDLE_TEST_SIGNING_SEED",
        ])
        .output()
        .expect("build bundle");
    assert_success(&build);
    let build_json = json(&build);
    assert_eq!(build_json["targets"], serde_json::json!([target]));
    assert_eq!(build_json["verification"]["file_count"], 2);
    assert!(!String::from_utf8_lossy(&build.stdout).contains(SIGNING_SEED));
    assert!(!String::from_utf8_lossy(&build.stderr).contains(SIGNING_SEED));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let escape = blocked_root.path().join("escape");
        fs::create_dir(&escape).expect("escape directory");
        let linked_prefix = root.join("linked-prefix");
        symlink(&escape, &linked_prefix).expect("linked prefix");
        let escaped_prefix = linked_prefix.join("install");
        let escaped = command(source_binary, &root)
            .args([
                "--approval-mode",
                "full-access",
                "bundle",
                "install",
                bundle.to_str().expect("bundle path"),
                "--prefix",
                escaped_prefix.to_str().expect("escaped prefix path"),
            ])
            .output()
            .expect("escaped bundle install");
        assert!(!escaped.status.success());
        assert!(
            String::from_utf8_lossy(&escaped.stderr)
                .contains("outside policy-authorized write roots")
        );
        assert!(!escape.join("install").exists());
    }

    let verify = command(source_binary, &root)
        .args(["bundle", "verify", bundle.to_str().expect("bundle path")])
        .output()
        .expect("verify bundle");
    assert_success(&verify);
    assert_eq!(
        json(&verify)["manifest_sha256"],
        build_json["verification"]["manifest_sha256"]
    );

    let install = command(source_binary, &root)
        .args([
            "--approval-mode",
            "full-access",
            "bundle",
            "install",
            bundle.to_str().expect("bundle path"),
            "--prefix",
            prefix.to_str().expect("prefix path"),
        ])
        .output()
        .expect("install bundle");
    assert_success(&install);
    let installation = json(&install);
    assert_eq!(installation["target"], target);
    let installed = PathBuf::from(
        installation["installed_path"]
            .as_str()
            .expect("installed path"),
    );
    assert!(installed.is_file());
    assert_eq!(
        fs::metadata(&installed).expect("installed metadata").len(),
        fs::metadata(&artifact).expect("staged metadata").len()
    );

    let version = command(&installed, &root)
        .arg("--version")
        .output()
        .expect("installed version");
    assert_success(&version);
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("colossus "));
    let run = command(&installed, &root)
        .args(["run", "bundle-installed"])
        .output()
        .expect("installed run");
    assert_success(&run);
    assert_eq!(json(&run)["output"], "bundle-installed");
    let audit = command(&installed, &root)
        .args(["audit", "verify"])
        .output()
        .expect("installed audit verify");
    assert_success(&audit);
    assert!(
        json(&audit)["last_sequence"]
            .as_u64()
            .is_some_and(|sequence| sequence >= 1)
    );

    let no_clobber = command(source_binary, &root)
        .args([
            "--approval-mode",
            "full-access",
            "bundle",
            "install",
            bundle.to_str().expect("bundle path"),
            "--prefix",
            prefix.to_str().expect("prefix path"),
        ])
        .output()
        .expect("repeat install");
    assert!(!no_clobber.status.success());
    assert!(String::from_utf8_lossy(&no_clobber.stderr).contains("refuses to replace"));

    fs::OpenOptions::new()
        .write(true)
        .open(bundle.join(artifact.strip_prefix(&staged).expect("relative artifact")))
        .expect("open bundle artifact")
        .write_all(b"tampered")
        .expect("tamper bundle");
    let tampered = command(source_binary, &root)
        .args(["bundle", "verify", bundle.to_str().expect("bundle path")])
        .output()
        .expect("verify tampered bundle");
    assert!(!tampered.status.success());
    assert!(String::from_utf8_lossy(&tampered.stderr).contains("file hash mismatch"));
}
