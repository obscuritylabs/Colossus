//! Actual-binary acceptance for offline worktree management and safe removal.
#[path = "support/process.rs"]
mod process_support;

use git2::{Repository, Signature};
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Fixture {
    root: tempfile::TempDir,
    home: process_support::IsolatedUserHome,
    repository: PathBuf,
    environment: Vec<(OsString, OsString)>,
}
impl Fixture {
    fn new() -> Self {
        let root = process_support::tempdir().unwrap();
        let mut process_template = Command::new(env!("CARGO_BIN_EXE_colossus"));
        let home = process_support::isolate_user_home(&mut process_template, root.path());
        let environment = process_template
            .get_envs()
            .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value.to_owned())))
            .collect();
        let repository = root.path().join("repository");
        let repo = Repository::init(&repository).unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        fs::write(repository.join("README.md"), "owned CLI fixture\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = Signature::now("Colossus fixture", "fixture@example.invalid").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "CLI fixture",
            &tree,
            &[],
        )
        .unwrap();
        fs::create_dir_all(home.colossus_home()).unwrap();
        // Any accidental runtime/configuration startup must fail this acceptance.
        fs::write(
            home.colossus_home().join("config.yaml"),
            "schemaVersion: 12345\ninvalidFixtureOnly: true\n",
        )
        .unwrap();
        Self {
            root,
            home,
            repository,
            environment,
        }
    }
    fn run(&self, arguments: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
        command
            .current_dir(self.root.path())
            .arg("--workspace")
            .arg(&self.repository)
            .args(["--output", "json"])
            .args(arguments)
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null());
        command.output().expect("owned CLI process")
    }
    fn json(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
        assert!(
            output.status.success(),
            "CLI fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("structured worktree output")
    }
}

#[test]
fn offline_commands_create_restore_and_remove_owned_checkout_without_a_runtime() {
    let fixture = Fixture::new();
    let before = Repository::open(&fixture.repository)
        .unwrap()
        .head()
        .unwrap()
        .target();
    let created = fixture.json(&["worktree", "create", "cli-feature"]);
    let id = created["id"].as_str().unwrap();
    let path = PathBuf::from(created["path"].as_str().unwrap());
    assert_eq!(created["branch"], "colossus/cli-feature");
    assert_eq!(created["state"], "active");
    assert_eq!(
        fixture.json(&["worktree", "list"])["worktrees"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fixture.json(&["worktree", "archive", id]);
    assert!(path.join("README.md").is_file());
    assert_eq!(
        fixture.json(&["worktree", "open", id, "--print-path"])["path"],
        created["path"]
    );
    fixture.json(&["worktree", "archive", id]);
    fs::write(path.join("retain.txt"), "keep these bytes").unwrap();
    assert!(
        !fixture
            .run(&["worktree", "remove", id, "--yes"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(path.join("retain.txt")).unwrap(),
        "keep these bytes"
    );
    fs::remove_file(path.join("retain.txt")).unwrap();
    assert!(!fixture.run(&["worktree", "remove", id]).status.success());
    assert!(path.is_dir());
    assert_eq!(
        fixture.json(&["worktree", "remove", id, "--yes"])["state"],
        "removed"
    );
    assert!(!path.exists());
    let source = Repository::open(&fixture.repository).unwrap();
    assert_eq!(source.head().unwrap().target(), before);
    assert!(
        source
            .find_branch("colossus/cli-feature", git2::BranchType::Local)
            .is_ok()
    );
    assert!(
        !fixture.home.colossus_home().join("workspaces").exists(),
        "offline worktree commands must not acquire runtime state"
    );
}

#[test]
fn runtime_flags_are_rejected_before_worktree_mutation() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        "--config",
        "unread-invalid-config.yaml",
        "worktree",
        "create",
        "forbidden",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("worktree operations are offline"));
    assert!(
        fixture.json(&["worktree", "list"])["worktrees"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture.json(&[
            "worktree",
            "settings",
            "--branch-prefix",
            "feature/",
            "--open-after-create",
            "false"
        ])["settings"]["branchPrefix"],
        "feature/"
    );
    assert_eq!(
        fixture.json(&["worktree", "create", "configured"])["branch"],
        "feature/configured"
    );
}
