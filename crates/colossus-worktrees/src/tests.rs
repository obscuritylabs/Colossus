use super::*;
use crate::{model::Catalog, storage::Storage};
use git2::{Repository, Signature};
use std::{fs, path::Path};

pub(crate) struct Fixture {
    _temp: tempfile::TempDir,
    pub(crate) repository: std::path::PathBuf,
    root: std::path::PathBuf,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let repository = temp.path().join("repository");
        let repo = Repository::init(&repository).expect("repo");
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        fs::write(repository.join("README.md"), "owned fixture\n").expect("fixture file");
        fs::write(repository.join(".gitignore"), "build/\n").expect("ignore fixture");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("README.md")).expect("add");
        index.add_path(Path::new(".gitignore")).expect("add ignore");
        index.write().expect("write");
        let tree_id = index.write_tree().expect("tree");
        let tree = repo.find_tree(tree_id).expect("tree");
        let signature =
            Signature::now("Colossus fixture", "fixture@example.invalid").expect("identity");
        repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
            .expect("commit");
        Self {
            repository,
            root: temp.path().join("managed"),
            _temp: temp,
        }
    }
    pub(crate) fn manager(&self) -> WorktreeManager {
        WorktreeManager::open(&self.repository, Some(&self.root)).expect("manager")
    }
    pub(crate) fn create(&self, name: &str) -> ManagedWorktree {
        self.manager()
            .create(&CreateWorktree {
                name: name.into(),
                branch: None,
                base: None,
                existing_branch: false,
            })
            .expect("create")
    }
}

#[test]
fn creates_separate_checkout_and_preserves_primary_head_and_branch() {
    let fixture = Fixture::new();
    let before = Repository::open(&fixture.repository)
        .unwrap()
        .head()
        .unwrap()
        .target();
    let worktree = fixture.create("feature-one");
    assert_eq!(
        fs::read_to_string(worktree.path.join("README.md")).unwrap(),
        "owned fixture\n"
    );
    assert_eq!(worktree.branch, "colossus/feature-one");
    assert_eq!(worktree.state, WorktreeState::Active);
    assert_eq!(worktree.dirty, Some(false));
    assert_eq!(
        Repository::open(&fixture.repository)
            .unwrap()
            .head()
            .unwrap()
            .target(),
        before
    );
    assert_eq!(fixture.manager().list().unwrap().worktrees.len(), 1);
}

#[test]
fn shared_users_block_archive_and_archive_preserves_files() {
    let fixture = Fixture::new();
    let entry = fixture.create("users");
    let first = acquire_workspace_lease(&entry.path)
        .unwrap()
        .expect("managed lease");
    let second = acquire_workspace_lease(&entry.path)
        .unwrap()
        .expect("second shared lease");
    assert!(fixture.manager().list().unwrap().worktrees[0].in_use);
    assert_eq!(
        fixture.manager().archive(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    drop(first);
    drop(second);
    let archived = fixture.manager().archive(&entry.id).unwrap();
    assert_eq!(archived.state, WorktreeState::Archived);
    assert!(!archived.in_use);
    assert!(entry.path.join("README.md").is_file());
    assert!(acquire_workspace_lease(&entry.path).is_err());
    assert_eq!(fixture.manager().open_path(&entry.id).unwrap(), entry.path);
    assert!(acquire_workspace_lease(&entry.path).unwrap().is_some());
}

#[test]
fn clean_archived_removal_retains_branch_and_durable_receipt() {
    let fixture = Fixture::new();
    let entry = fixture.create("remove");
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Conflict
    );
    fixture.manager().archive(&entry.id).unwrap();
    let result = fixture.manager().remove(&entry.id).unwrap();
    assert_eq!(result.state, WorktreeState::Removed);
    assert!(!entry.path.exists());
    assert!(
        Repository::open(&fixture.repository)
            .unwrap()
            .find_branch(&entry.branch, git2::BranchType::Local)
            .is_ok()
    );
    assert!(
        fixture
            .root
            .join("receipts")
            .join(format!("{}.json", entry.id))
            .is_file()
    );
}

#[test]
fn dirty_and_untracked_files_block_removal() {
    let fixture = Fixture::new();
    let entry = fixture.create("dirty");
    fixture.manager().archive(&entry.id).unwrap();
    fs::write(entry.path.join("README.md"), "keep my edits").unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Dirty
    );
    assert_eq!(
        fs::read_to_string(entry.path.join("README.md")).unwrap(),
        "keep my edits"
    );
    fs::write(entry.path.join("README.md"), "owned fixture\n").unwrap();
    fs::write(entry.path.join("untracked.txt"), "keep untracked data").unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Dirty
    );
    assert!(entry.path.join("untracked.txt").is_file());
}

#[test]
fn ignored_files_also_block_removal() {
    let fixture = Fixture::new();
    let entry = fixture.create("ignored");
    fs::create_dir(entry.path.join("build")).unwrap();
    fs::write(entry.path.join("build/result"), "user data").unwrap();
    fixture.manager().archive(&entry.id).unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Dirty
    );
    assert!(entry.path.join("build/result").is_file());
}

#[test]
fn git_lock_and_current_workspace_block_removal() {
    let fixture = Fixture::new();
    let entry = fixture.create("locked");
    let current = WorktreeManager::open(&entry.path, Some(&fixture.root)).unwrap();
    assert_eq!(
        current.archive(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    fixture.manager().archive(&entry.id).unwrap();
    let repo = Repository::open(&fixture.repository).unwrap();
    repo.find_worktree(&format!("colossus-{}", entry.id))
        .unwrap()
        .lock(Some("keep this worktree"))
        .unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Conflict
    );
    assert!(entry.path.is_dir());
}

#[test]
fn replaced_checkout_and_forged_ownership_fail_closed() {
    let fixture = Fixture::new();
    let entry = fixture.create("ownership");
    fixture.manager().archive(&entry.id).unwrap();
    let repo = Repository::open(&entry.path).unwrap();
    let origin = repo.path().join("colossus-managed-worktree.json");
    fs::write(origin, "{}").unwrap();
    drop(repo);
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Ownership
    );
    assert!(entry.path.is_dir());
}

#[test]
fn interrupted_records_remain_visible_and_cannot_be_replayed_or_removed() {
    let fixture = Fixture::new();
    let entry = fixture.create("interrupted");
    let storage = Storage::open(&fixture.root, false).unwrap();
    let _lease = storage.lock().unwrap();
    let mut catalog: Catalog = storage.load().unwrap();
    catalog.entries[0].state = WorktreeState::Removing;
    storage.save(&catalog).unwrap();
    drop(_lease);
    let visible = fixture.manager().list().unwrap().worktrees;
    assert_eq!(visible[0].state, WorktreeState::Removing);
    assert!(visible[0].note.is_some());
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Conflict
    );
    assert!(entry.path.is_dir());
}

#[test]
fn unrelated_roots_and_worktree_ids_are_never_adopted() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.root).unwrap();
    fs::write(fixture.root.join("keep.txt"), "keep").unwrap();
    assert!(WorktreeManager::open(&fixture.repository, Some(&fixture.root)).is_err());
    assert_eq!(
        fs::read_to_string(fixture.root.join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn preferences_persist_and_validate_future_branch_defaults() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let settings = WorktreeSettings {
        branch_prefix: "feature/".into(),
        open_after_create: false,
    };
    manager.update_settings(settings.clone()).unwrap();
    assert_eq!(fixture.manager().list().unwrap().settings, settings);
    assert_eq!(fixture.create("custom").branch, "feature/custom");
    assert!(
        manager
            .update_settings(WorktreeSettings {
                branch_prefix: "../../".into(),
                open_after_create: true
            })
            .is_err()
    );
}

#[test]
fn names_revision_expressions_and_existing_branches_are_explicit() {
    let fixture = Fixture::new();
    for name in ["../escape", "CON", "aux", "-bad", "bad.", "feature/path"] {
        assert!(
            fixture
                .manager()
                .create(&CreateWorktree {
                    name: name.into(),
                    branch: None,
                    base: None,
                    existing_branch: false
                })
                .is_err()
        );
    }
    assert!(
        fixture
            .manager()
            .create(&CreateWorktree {
                name: "expression".into(),
                branch: None,
                base: Some("HEAD~1".into()),
                existing_branch: false
            })
            .is_err()
    );
    let entry = fixture.create("existing");
    fixture.manager().archive(&entry.id).unwrap();
    fixture.manager().remove(&entry.id).unwrap();
    let linked = fixture
        .manager()
        .create(&CreateWorktree {
            name: "reuse".into(),
            branch: Some(entry.branch),
            base: None,
            existing_branch: true,
        })
        .unwrap();
    assert_eq!(linked.state, WorktreeState::Active);
}

#[test]
fn runtime_lease_does_not_adopt_unmanaged_or_missing_workspaces() {
    let fixture = Fixture::new();
    assert!(
        acquire_workspace_lease(&fixture.repository)
            .unwrap()
            .is_none()
    );
    let nested = fixture.repository.join("nested/deeper");
    fs::create_dir_all(&nested).unwrap();
    assert!(acquire_workspace_lease(&nested).unwrap().is_none());
    let plain = fixture._temp.path().join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(acquire_workspace_lease(&plain).unwrap().is_none());
    assert!(!fixture.root.exists());
    assert!(acquire_workspace_lease(&fixture.repository.join("missing")).is_err());
    assert!(acquire_workspace_lease(&fixture.repository.join("README.md")).is_err());
}

#[test]
fn exclusive_use_lease_prevents_concurrent_runtime_acquisition() {
    let fixture = Fixture::new();
    let entry = fixture.create("exclusive");
    let storage = Storage::open(&fixture.root, false).unwrap();
    let lease = crate::lease::exclusive(&storage, &entry.id).unwrap();
    assert_eq!(
        acquire_workspace_lease(&entry.path).err().unwrap().code,
        WorktreeErrorCode::Busy
    );
    drop(lease);
    assert!(acquire_workspace_lease(&entry.path).unwrap().is_some());
}

#[test]
fn missing_origin_or_catalog_is_not_recreated_during_session_startup() {
    let fixture = Fixture::new();
    let entry = fixture.create("orphan");
    let origin = Repository::open(&entry.path)
        .unwrap()
        .path()
        .join("colossus-managed-worktree.json");
    let bytes = fs::read(&origin).unwrap();
    fs::remove_file(&origin).unwrap();
    assert_eq!(
        acquire_workspace_lease(&entry.path).err().unwrap().code,
        WorktreeErrorCode::Ownership
    );
    assert!(!origin.exists());
    fs::write(&origin, bytes).unwrap();
    fs::remove_file(fixture.root.join("catalog.json")).unwrap();
    assert_eq!(
        acquire_workspace_lease(&entry.path).err().unwrap().code,
        WorktreeErrorCode::Ownership
    );
    assert!(!fixture.root.join("catalog.json").exists());
}

#[test]
fn competing_catalog_mutations_fail_without_changing_the_inventory() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let storage = Storage::open(&fixture.root, false).unwrap();
    let held = storage.lock().unwrap();
    assert_eq!(
        manager
            .create(&CreateWorktree {
                name: "blocked".into(),
                branch: None,
                base: None,
                existing_branch: false
            })
            .unwrap_err()
            .code,
        WorktreeErrorCode::Busy
    );
    assert!(storage.load().unwrap().entries.is_empty());
    drop(held);
    assert!(manager.list().unwrap().worktrees.is_empty());
}

#[test]
fn existing_checked_out_branches_are_rejected_before_creating_intent() {
    let fixture = Fixture::new();
    let branch = Repository::open(&fixture.repository)
        .unwrap()
        .head()
        .unwrap()
        .shorthand()
        .unwrap()
        .to_owned();
    assert_eq!(
        fixture
            .manager()
            .create(&CreateWorktree {
                name: "in-use-branch".into(),
                branch: Some(branch),
                base: None,
                existing_branch: true
            })
            .unwrap_err()
            .code,
        WorktreeErrorCode::Conflict
    );
    assert!(fixture.manager().list().unwrap().worktrees.is_empty());
}

#[test]
fn unrelated_separate_gitdir_keeps_its_unmanaged_startup_behavior() {
    let fixture = Fixture::new();
    let separate = fixture.repository.join("separate-gitdir");
    fs::rename(fixture.repository.join(".git"), &separate).unwrap();
    fs::write(fixture.repository.join(".git"), "gitdir: separate-gitdir\n").unwrap();
    assert!(
        acquire_workspace_lease(&fixture.repository)
            .unwrap()
            .is_none()
    );
}

#[test]
fn alternate_object_stores_are_rejected() {
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.repository).unwrap();
    fs::create_dir_all(repo.path().join("objects/info")).unwrap();
    fs::write(repo.path().join("objects/info/alternates"), "/outside\n").unwrap();
    assert!(
        fixture
            .manager()
            .create(&CreateWorktree {
                name: "alternate".into(),
                branch: None,
                base: None,
                existing_branch: false
            })
            .is_err()
    );
}

#[test]
fn hardlinked_checkout_bytes_are_never_removed() {
    let fixture = Fixture::new();
    let entry = fixture.create("hardlinks");
    fixture.manager().archive(&entry.id).unwrap();
    fs::hard_link(
        entry.path.join("README.md"),
        fixture.repository.join("outside-link"),
    )
    .unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Ownership
    );
    assert!(entry.path.join("README.md").is_file());
    assert_eq!(
        fs::read_to_string(fixture.repository.join("outside-link")).unwrap(),
        "owned fixture\n"
    );
}

#[test]
fn source_configuration_includes_are_rejected_before_creation() {
    let fixture = Fixture::new();
    let repo = Repository::open(&fixture.repository).unwrap();
    repo.config()
        .unwrap()
        .set_str("include.path", "unowned.config")
        .unwrap();
    assert!(
        fixture
            .manager()
            .create(&CreateWorktree {
                name: "config".into(),
                branch: None,
                base: None,
                existing_branch: false
            })
            .is_err()
    );
    assert!(fixture.manager().list().unwrap().worktrees.is_empty());
}

/// The child is activated only by the parent fixture below, with an owned path.
#[test]
#[ignore = "fixture subprocess entrypoint"]
fn lease_holder_process() {
    use std::io::Read as _;
    let path = std::env::var_os("COLOSSUS_WORKTREE_LEASE_FIXTURE").expect("fixture workspace");
    let ready = std::env::var_os("COLOSSUS_WORKTREE_READY_FIXTURE").expect("fixture readiness");
    let _lease = acquire_workspace_lease(Path::new(&path))
        .unwrap()
        .expect("managed lease");
    fs::write(ready, "shared lease held").unwrap();
    let mut signal = [0_u8];
    std::io::stdin().read_exact(&mut signal).unwrap();
}

#[test]
fn another_process_holds_use_lease_until_its_work_is_finished() {
    use std::{
        io::Write as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let fixture = Fixture::new();
    let entry = fixture.create("cross-process");
    let nested = entry.path.join("nested/deeper");
    fs::create_dir_all(&nested).unwrap();
    let ready = fixture._temp.path().join("child-ready");
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "tests::lease_holder_process",
                "--nocapture",
            ])
            .env("COLOSSUS_WORKTREE_LEASE_FIXTURE", &nested)
            .env("COLOSSUS_WORKTREE_READY_FIXTURE", &ready)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "lease child exited early"
        );
        assert!(
            Instant::now() < deadline,
            "lease child did not become ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(fixture.manager().list().unwrap().worktrees[0].in_use);
    assert_eq!(
        fixture.manager().archive(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    child.0.stdin.take().unwrap().write_all(&[1]).unwrap();
    assert!(child.0.wait().unwrap().success());
    fixture.manager().archive(&entry.id).unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap().state,
        WorktreeState::Removed
    );
}

#[test]
fn nested_shared_users_block_archive_until_every_lease_is_released() {
    let fixture = Fixture::new();
    let entry = fixture.create("nested-users");
    let nested = entry.path.join("nested/deeper");
    fs::create_dir_all(&nested).unwrap();
    let first = acquire_workspace_lease(&nested).unwrap().unwrap();
    let second = acquire_workspace_lease(nested.parent().unwrap())
        .unwrap()
        .unwrap();
    assert!(fixture.manager().list().unwrap().worktrees[0].in_use);
    assert_eq!(
        fixture.manager().archive(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    drop(first);
    assert_eq!(
        fixture.manager().archive(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    drop(second);
    assert_eq!(
        fixture.manager().archive(&entry.id).unwrap().state,
        WorktreeState::Archived
    );
}

#[test]
fn archived_managed_parent_rejects_nested_session_startup() {
    let fixture = Fixture::new();
    let entry = fixture.create("nested-archived");
    let nested = entry.path.join("nested/deeper");
    fs::create_dir_all(&nested).unwrap();
    fixture.manager().archive(&entry.id).unwrap();
    let catalog = fs::read(fixture.root.join("catalog.json")).unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Conflict
    );
    assert_eq!(
        fs::read(fixture.root.join("catalog.json")).unwrap(),
        catalog
    );
    fixture.manager().open_path(&entry.id).unwrap();
    assert!(acquire_workspace_lease(&nested).unwrap().is_some());
}

#[test]
fn exclusive_managed_parent_blocks_nested_session_startup() {
    let fixture = Fixture::new();
    let entry = fixture.create("nested-exclusive");
    let nested = entry.path.join("nested/deeper");
    fs::create_dir_all(&nested).unwrap();
    let storage = Storage::open(&fixture.root, false).unwrap();
    let exclusive = crate::lease::exclusive(&storage, &entry.id).unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Busy
    );
    drop(exclusive);
    assert!(acquire_workspace_lease(&nested).unwrap().is_some());
}

#[test]
fn nested_unmanaged_repository_does_not_hide_its_managed_parent() {
    let fixture = Fixture::new();
    let entry = fixture.create("nested-repository");
    let repository = entry.path.join("nested-repository");
    Repository::init(&repository).unwrap();
    let nested = repository.join("child");
    fs::create_dir(&nested).unwrap();
    let lease = acquire_workspace_lease(&nested).unwrap().unwrap();
    assert_eq!(
        fixture.manager().archive(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    drop(lease);
    fixture.manager().archive(&entry.id).unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Conflict
    );
}

#[test]
fn nested_managed_checkouts_retain_every_parent_and_release_on_failed_admission() {
    let fixture = Fixture::new();
    let outer = fixture.create("outer");
    let inner_manager =
        WorktreeManager::open(&fixture.repository, Some(&outer.path.join("inner-managed")))
            .unwrap();
    let inner = inner_manager
        .create(&CreateWorktree {
            name: "inner".into(),
            branch: None,
            base: None,
            existing_branch: false,
        })
        .unwrap();
    let nested = inner.path.join("child");
    fs::create_dir(&nested).unwrap();
    let lease = acquire_workspace_lease(&nested).unwrap().unwrap();
    assert_eq!(
        fixture.manager().archive(&outer.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    assert_eq!(
        inner_manager.archive(&inner.id).unwrap_err().code,
        WorktreeErrorCode::Busy
    );
    drop(lease);
    let storage = Storage::open(&fixture.root, false).unwrap();
    let exclusive = crate::lease::exclusive(&storage, &outer.id).unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Busy
    );
    // Admission reached the outer parent after leasing the inner checkout. Its
    // failure must release that provisional inner lease instead of leaking a user.
    assert_eq!(
        inner_manager.archive(&inner.id).unwrap().state,
        WorktreeState::Archived
    );
    drop(exclusive);
    assert_eq!(
        fixture.manager().archive(&outer.id).unwrap().state,
        WorktreeState::Archived
    );
}

#[test]
fn nested_managed_startup_rejects_missing_or_tampered_parent_metadata() {
    let fixture = Fixture::new();
    let entry = fixture.create("nested-orphan");
    let nested = entry.path.join("nested/deeper");
    fs::create_dir_all(&nested).unwrap();
    let origin = Repository::open(&entry.path)
        .unwrap()
        .path()
        .join(crate::lease::ORIGIN_FILE);
    let original = fs::read(&origin).unwrap();
    fs::remove_file(&origin).unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Ownership
    );
    assert!(!origin.exists());
    fs::write(&origin, b"{invalid origin}").unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Ownership
    );
    assert_eq!(fs::read(&origin).unwrap(), b"{invalid origin}");
    fs::write(&origin, original).unwrap();
    let catalog = fixture.root.join("catalog.json");
    fs::remove_file(&catalog).unwrap();
    assert_eq!(
        acquire_workspace_lease(&nested).err().unwrap().code,
        WorktreeErrorCode::Ownership
    );
    assert!(!catalog.exists());
}
