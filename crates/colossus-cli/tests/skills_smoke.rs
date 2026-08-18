//! Credential-free end-to-end declarative skill and resource smoke test.

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(binary);
    let _isolated_home = process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_SKILL_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_SKILL_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn skill_activation_and_resources_are_durable_policy_bound_and_data_only() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    // macOS exposes the tempfile root through `/var`, a symlink to `/private/var`.
    // Production skill roots reject symlink components, so construct every fixture
    // path from the exact canonical workspace selected by the runtime.
    let root = fs::canonicalize(directory.path()).expect("canonical directory");
    let workflows = root.join("workflows");
    let skills = root.join("skills");
    let user_skills = root.join("user-skills");
    let skill = skills.join("demo");
    fs::create_dir_all(skill.join("references")).expect("skill directory");
    fs::create_dir_all(&workflows).expect("workflows");
    fs::write(
        skill.join("SKILL.md"),
        "Use the demo instructions safely.\n",
    )
    .expect("skill");
    fs::write(
        skill.join("manifest.json"),
        r#"{"name":"demo","version":"1.0.0","description":"Demo data-only skill.","triggers":["demo"],"required_tools":["echo"],"permissions":["read-only"],"offline_compatible":true}"#,
    )
    .expect("manifest");
    fs::write(skill.join("references/guide.md"), "# Private guide\n").expect("resource");
    let state = root.join("state.redb");
    let anchor = root.join("anchor.json");
    let config = root.join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_SKILL_TEST_JOURNAL_KEY
    journal_key_id: skill-test-journal-v1
    signing_variable: COLOSSUS_SKILL_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo, skill.resource.list, skill.resource.read]
    exclude: []
  actions:
    allow: [skill.inspect, skill.read, skill.validate, skill.resource.list, skill.resource.read]
    requireApproval: [skill.scaffold, skill.write, skill.install]
    deny: []
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
skills:
  enabled: true
  allowUserOverrides: false
  bundled: {missing}
  repository: {skills}
  user: {user_skills}
  disabled: []
agent:
  maxTurns: 4
sandbox:
  backend: native
  profile: skill-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            skills = skills.display(),
            user_skills = user_skills.display(),
            missing = root.join("missing").display(),
        ),
    )
    .expect("config");

    let list = run(binary, &config, directory.path(), &["skills", "list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list: Value = serde_json::from_slice(&list.stdout).expect("list JSON");
    assert_eq!(list[0]["name"], "demo");
    assert_eq!(list[0]["source"], "repository:demo");

    let composition = run(
        binary,
        &config,
        directory.path(),
        &["skills", "compose", "do work", "--skill", "demo"],
    );
    assert!(composition.status.success());
    let composition: Value = serde_json::from_slice(&composition.stdout).expect("composition JSON");
    assert_eq!(composition["active_skills"][0]["name"], "demo");
    assert!(
        composition["instructions"]
            .as_str()
            .is_some_and(|value| value.contains("Use the demo instructions safely."))
    );

    let resources = run(
        binary,
        &config,
        directory.path(),
        &["skills", "resources", "demo"],
    );
    assert!(resources.status.success());
    let resources: Value = serde_json::from_slice(&resources.stdout).expect("resources JSON");
    assert_eq!(resources[0]["path"], "references/guide.md");

    let read = run(
        binary,
        &config,
        directory.path(),
        &["skills", "read", "demo", "references/guide.md"],
    );
    assert!(read.status.success());
    let read: Value = serde_json::from_slice(&read.stdout).expect("read JSON");
    assert_eq!(read["content"], "# Private guide\n");

    let traversal = run(
        binary,
        &config,
        directory.path(),
        &["skills", "read", "demo", "../outside"],
    );
    assert!(!traversal.status.success());

    let agent = run(
        binary,
        &config,
        directory.path(),
        &["run", "hello", "--skill", "demo"],
    );
    assert!(
        agent.status.success(),
        "{}",
        String::from_utf8_lossy(&agent.stderr)
    );
    let agent: Value = serde_json::from_slice(&agent.stdout).expect("agent JSON");
    assert_eq!(agent["output"], "hello");
    let run_id = agent["run_id"].as_str().expect("run id");
    let detail = run(
        binary,
        &config,
        directory.path(),
        &["telemetry", "show", run_id],
    );
    assert!(detail.status.success());
    let detail: Value = serde_json::from_slice(&detail.stdout).expect("detail JSON");
    assert!(detail["records"].as_array().is_some_and(|records| {
        records
            .iter()
            .any(|record| record["context"]["skill_ids"][0] == "demo")
    }));
    assert!(
        !serde_json::to_string(&detail)
            .expect("detail serialization")
            .contains("Use the demo instructions safely")
    );

    let denied = run(
        binary,
        &config,
        directory.path(),
        &[
            "skills",
            "scaffold",
            "denied",
            "Denied scaffold",
            "--instructions",
            "Must not be written.",
        ],
    );
    assert!(!denied.status.success());
    assert!(!user_skills.join("denied").exists());

    let scaffold = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "skills",
            "scaffold",
            "authored",
            "Authored skill",
            "--instructions",
            "Initial authoring instructions.",
            "--resource-dir",
            "references",
        ],
    );
    assert!(
        scaffold.status.success(),
        "{}",
        String::from_utf8_lossy(&scaffold.stderr)
    );
    let inspected = run(
        binary,
        &config,
        directory.path(),
        &["skills", "inspect", "authored"],
    );
    assert!(inspected.status.success());
    let inspected: Value = serde_json::from_slice(&inspected.stdout).expect("inspection JSON");
    assert_eq!(inspected["manifest"]["name"], "authored");
    assert!(inspected.get("instructions").is_none());

    let file = run(
        binary,
        &config,
        directory.path(),
        &["skills", "file-read", "authored", "SKILL.md"],
    );
    assert!(file.status.success());
    let file: Value = serde_json::from_slice(&file.stdout).expect("file JSON");
    let expected = file["sha256"].as_str().expect("hash");
    let write = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "skills",
            "write",
            "authored",
            "SKILL.md",
            "Updated authoring instructions.",
            "--expected-sha256",
            expected,
        ],
    );
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    let stale = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "skills",
            "write",
            "authored",
            "SKILL.md",
            "Stale instructions.",
            "--expected-sha256",
            expected,
        ],
    );
    assert!(!stale.status.success());

    let local = directory.path().join("local-source/local");
    fs::create_dir_all(local.join("examples")).expect("local source");
    fs::write(local.join("SKILL.md"), "Local instructions.\n").expect("local instructions");
    fs::write(
        local.join("manifest.json"),
        r#"{"name":"local","version":"1.0.0","description":"Local install source","triggers":[],"required_tools":[],"permissions":[],"offline_compatible":true}"#,
    )
    .expect("local manifest");
    let validation = run(
        binary,
        &config,
        directory.path(),
        &["skills", "validate", "local-source/local", "--local"],
    );
    assert!(validation.status.success());
    let install = run(
        binary,
        &config,
        directory.path(),
        &[
            "--approval-mode",
            "full-access",
            "skills",
            "install",
            "local-source/local",
        ],
    );
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    assert!(user_skills.join("local/SKILL.md").is_file());
}
