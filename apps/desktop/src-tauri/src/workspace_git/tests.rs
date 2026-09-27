use super::{discovery::discover, reader::Reader};
use git2::{Repository, Signature};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let repo = Repository::init(&root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        Self {
            _directory: directory,
            root,
            repo,
        }
    }
    fn write(&self, path: &str, value: &str) {
        let file = self.root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, value).unwrap();
    }
    fn commit(&self, message: &str) -> git2::Oid {
        let mut index = self.repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Git test", "git@example.invalid").unwrap();
        let parent = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok());
        self.repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent.iter().collect::<Vec<_>>(),
            )
            .unwrap()
    }
}

#[test]
fn non_git_and_unborn_branch_have_distinct_states() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        discover(&fs::canonicalize(directory.path()).unwrap())
            .unwrap()
            .is_none()
    );
    let fixture = Fixture::new();
    let binding = discover(&fixture.root).unwrap().unwrap();
    assert!(!binding.needs_approval());
    let status = Reader::open(&binding)
        .unwrap()
        .status(&binding, "test".into())
        .unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.head.is_none());
    assert!(status.files.is_empty());
}

#[test]
fn status_separates_index_worktree_untracked_and_ignored_without_writing() {
    let fixture = Fixture::new();
    fixture.write("tracked.txt", "before\n");
    fixture.write("deleted.txt", "before\n");
    fixture.write(".gitignore", "ignored/\n");
    fixture.commit("Initial");
    fixture.write("tracked.txt", "staged\n");
    let mut index = fixture.repo.index().unwrap();
    index.add_path(Path::new("tracked.txt")).unwrap();
    index.write().unwrap();
    fixture.write("tracked.txt", "unstaged content\n");
    fixture.write("new.txt", "new\n");
    fixture.write("ignored/private.txt", "ignored\n");
    fs::remove_file(fixture.root.join("deleted.txt")).unwrap();
    let index_before = fs::read(fixture.root.join(".git/index")).unwrap();
    let config_before = fs::read(fixture.root.join(".git/config")).unwrap();
    let binding = discover(&fixture.root).unwrap().unwrap();
    let status = Reader::open(&binding)
        .unwrap()
        .status(&binding, "test".into())
        .unwrap();
    let mixed = status
        .files
        .iter()
        .find(|file| file.path == "tracked.txt")
        .unwrap();
    assert_eq!(mixed.staged, Some("modified"));
    assert_eq!(mixed.unstaged, Some("modified"));
    assert!(
        status
            .files
            .iter()
            .any(|file| file.path == "new.txt" && file.untracked)
    );
    assert!(
        status
            .files
            .iter()
            .any(|file| file.path == "deleted.txt" && file.unstaged == Some("deleted"))
    );
    assert!(
        !status
            .files
            .iter()
            .any(|file| file.path.contains("private"))
    );
    assert_eq!(
        fs::read(fixture.root.join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(fixture.root.join(".git/config")).unwrap(),
        config_before
    );
}

#[test]
fn detached_head_history_and_root_commit_details_are_readable() {
    let fixture = Fixture::new();
    fixture.write("first.txt", "first\n");
    let first = fixture.commit("First commit\n\nBody");
    fixture.write("next.txt", "next\n");
    fixture.commit("Next commit");
    fixture.repo.set_head_detached(first).unwrap();
    let binding = discover(&fixture.root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    let (branch, head) = reader.head().unwrap();
    assert!(branch.is_none());
    assert_eq!(head, Some(first.to_string()));
    let (history, more) = reader.history(&first.to_string(), 0).unwrap();
    assert_eq!(history.len(), 1);
    assert!(!more);
    let details = reader.details(&binding, &first.to_string()).unwrap();
    assert!(details.parents.is_empty());
    assert!(details.message.contains("Body"));
    assert_eq!(details.files[0].path, "first.txt");
    assert_eq!(details.files[0].status, "added");
}

#[test]
fn subdirectory_requires_metadata_approval_and_only_releases_scoped_files() {
    let fixture = Fixture::new();
    fixture.write("inside/a.txt", "first\n");
    fixture.write("outside.txt", "first\n");
    fixture.commit("Initial");
    fixture.write("inside/a.txt", "changed\n");
    fixture.write("outside.txt", "outside changed\n");
    let binding = discover(&fixture.root.join("inside")).unwrap().unwrap();
    assert!(binding.needs_approval());
    let reader = Reader::open(&binding).unwrap();
    let status = reader.status(&binding, "test".into()).unwrap();
    assert!(status.scoped);
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].path, "a.txt");
    let details = reader
        .details(&binding, status.head.as_deref().unwrap())
        .unwrap();
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].path, "a.txt");
}

#[test]
fn linked_worktree_uses_its_own_head_and_changes() {
    let fixture = Fixture::new();
    fixture.write("first.txt", "first\n");
    fixture.commit("Initial");
    let linked_directory = tempfile::tempdir().unwrap();
    let linked = fs::canonicalize(linked_directory.path())
        .unwrap()
        .join("linked");
    fixture.repo.worktree("review", &linked, None).unwrap();
    fs::write(linked.join("first.txt"), "worktree change\n").unwrap();
    let binding = discover(&linked).unwrap().unwrap();
    assert!(binding.needs_approval());
    let status = Reader::open(&binding)
        .unwrap()
        .status(&binding, "test".into())
        .unwrap();
    assert!(status.linked_worktree);
    assert_eq!(status.branch.as_deref(), Some("review"));
    assert_eq!(status.files.len(), 1);
    assert_eq!(
        fs::read_to_string(fixture.root.join("first.txt")).unwrap(),
        "first\n"
    );
}

#[test]
fn alternate_object_store_and_forged_worktree_links_are_rejected() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join(".git/objects/info")).unwrap();
    fixture.write(".git/objects/info/alternates", "outside\n");
    let binding = discover(&fixture.root).unwrap().unwrap();
    assert!(Reader::open(&binding).is_err());
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join(".git"),
        format!("gitdir: {}", fixture.root.join(".git").display()),
    )
    .unwrap();
    assert!(discover(&fs::canonicalize(directory.path()).unwrap()).is_err());
}

#[test]
fn alternate_object_store_case_variants_are_rejected_before_opening() {
    for path in [
        ".git/objects/info/Alternates",
        ".git/objects/info/HTTP-Alternates",
        ".git/OBJECTS/INFO/ALTERNATES",
    ] {
        let fixture = Fixture::new();
        fixture.write(path, "outside\n");
        let binding = discover(&fixture.root).unwrap().unwrap();
        let failure = super::discovery::inspect_metadata(&binding).unwrap_err();
        assert!(
            failure.message.contains("alternate object stores"),
            "{path}"
        );
    }
}

#[test]
fn oversized_case_variant_index_is_rejected_before_opening() {
    let fixture = Fixture::new();
    fs::File::create(fixture.root.join(".git/INDEX"))
        .unwrap()
        .set_len(17 * 1024 * 1024)
        .unwrap();
    let binding = discover(&fixture.root).unwrap().unwrap();
    let failure = super::discovery::inspect_metadata(&binding).unwrap_err();
    assert!(failure.message.contains("inspection limit"));
}

#[test]
fn staged_rename_matches_git_and_commit_details_preserve_the_old_path() {
    let fixture = Fixture::new();
    fixture.write("before.txt", "original content\n");
    fixture.commit("Initial");
    fs::rename(
        fixture.root.join("before.txt"),
        fixture.root.join("after.txt"),
    )
    .unwrap();
    git(&fixture, &["add", "--all"], true);
    let porcelain = git(&fixture, &["status", "--porcelain=v1", "-z"], true);
    assert!(porcelain.starts_with("R  after.txt\0before.txt\0"));
    let binding = discover(&fixture.root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    let status = reader.status(&binding, "test".into()).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].path, "after.txt");
    assert_eq!(status.files[0].previous_path.as_deref(), Some("before.txt"));
    assert_eq!(status.files[0].staged, Some("renamed"));
    let id = fixture.commit("Rename");
    let details = reader.details(&binding, &id.to_string()).unwrap();
    assert_eq!(details.files[0].status, "renamed");
    assert_eq!(
        details.files[0].previous_path.as_deref(),
        Some("before.txt")
    );
}

#[test]
fn conflict_and_merge_state_match_git() {
    let fixture = Fixture::new();
    fixture.write("conflict.txt", "original\n");
    fixture.commit("Initial");
    git(&fixture, &["checkout", "-b", "topic"], true);
    fixture.write("conflict.txt", "topic\n");
    fixture.commit("Topic");
    git(&fixture, &["checkout", "main"], true);
    fixture.write("conflict.txt", "main\n");
    fixture.commit("Main");
    git(
        &fixture,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "merge",
            "--no-commit",
            "topic",
        ],
        false,
    );
    assert!(git(&fixture, &["status", "--porcelain=v1"], true).contains("UU conflict.txt"));
    let binding = discover(&fixture.root).unwrap().unwrap();
    let status = Reader::open(&binding)
        .unwrap()
        .status(&binding, "test".into())
        .unwrap();
    assert_eq!(status.operation, Some("Merge in progress"));
    assert!(
        status
            .files
            .iter()
            .any(|file| file.path == "conflict.txt" && file.conflicted)
    );
}

#[test]
fn configured_helpers_are_not_executed_by_the_reader() {
    let fixture = Fixture::new();
    fixture.write("tracked.txt", "before\n");
    fixture.commit("Initial");
    fixture.write("tracked.txt", "after\n");
    fixture.write(".gitattributes", "*.txt filter=external diff=external\n");
    let mut config = fixture.repo.config().unwrap();
    for key in [
        "core.fsmonitor",
        "filter.external.clean",
        "filter.external.smudge",
        "diff.external.command",
        "core.hooksPath",
    ] {
        config.set_str(key, "this-program-must-never-run").unwrap();
    }
    let binding = discover(&fixture.root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    assert!(
        reader
            .status(&binding, "test".into())
            .unwrap()
            .files
            .iter()
            .any(|file| file.path == "tracked.txt")
    );
    let (_, head) = reader.head().unwrap();
    assert_eq!(
        reader.history(head.as_deref().unwrap(), 0).unwrap().0.len(),
        1
    );
    assert!(reader.details(&binding, head.as_deref().unwrap()).is_ok());
}

#[test]
fn changed_binding_and_invalid_revision_do_not_read_another_repository() {
    let fixture = Fixture::new();
    fixture.write("first.txt", "first\n");
    let id = fixture.commit("Initial");
    let binding = discover(&fixture.root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    assert!(reader.details(&binding, "HEAD:private.txt").is_err());
    assert!(reader.history(&id.to_string(), 400).is_err());
    fs::rename(fixture.root.join(".git"), fixture.root.join("saved-git")).unwrap();
    Repository::init(&fixture.root).unwrap();
    assert!(binding.revalidate().is_err());
}

#[test]
fn repository_configuration_includes_are_rejected_before_library_open() {
    let fixture = Fixture::new();
    let mut config = fs::read_to_string(fixture.root.join(".git/config")).unwrap();
    config.push_str("\n[include]\n path = ../../private-config\n");
    fs::write(fixture.root.join(".git/config"), config).unwrap();
    let binding = discover(&fixture.root).unwrap().unwrap();
    assert!(Reader::open(&binding).is_err());
}

#[test]
fn staged_submodule_commit_changes_are_shown_without_inspecting_its_files() {
    let fixture = Fixture::new();
    fixture.write("first.txt", "first\n");
    let first = fixture.commit("Initial");
    let mut index = fixture.repo.index().unwrap();
    let mut entry = index.get_path(Path::new("first.txt"), 0).unwrap();
    entry.path = b"module".to_vec();
    entry.mode = 0o160_000;
    entry.id = first;
    index.add(&entry).unwrap();
    index.write().unwrap();
    fixture.write("module/hidden.txt", "Not part of parent inspection\n");
    let binding = discover(&fixture.root).unwrap().unwrap();
    let status = Reader::open(&binding)
        .unwrap()
        .status(&binding, "test".into())
        .unwrap();
    assert!(
        status
            .files
            .iter()
            .any(|file| file.path == "module" && file.staged == Some("added"))
    );
    assert!(!status.files.iter().any(|file| file.path.contains("hidden")));
    assert!(status.notes.iter().any(|note| note.contains("Submodule")));
}

#[test]
fn commit_pagination_and_merge_details_have_defined_boundaries() {
    let fixture = Fixture::new();
    fixture.write("first.txt", "first\n");
    let first = fixture.commit("Initial");
    for number in 0..41 {
        fixture.write("first.txt", &format!("version {number}\n"));
        fixture.commit(&format!("Commit {number}"));
    }
    let head = fixture.repo.head().unwrap().peel_to_commit().unwrap();
    let tree = head.tree().unwrap();
    let parent = fixture.repo.find_commit(first).unwrap();
    let signature = Signature::now("Merge test", "merge@example.invalid").unwrap();
    let merge = fixture
        .repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Merge",
            &tree,
            &[&head, &parent],
        )
        .unwrap();
    let binding = discover(&fixture.root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    let (page, more) = reader.history(&merge.to_string(), 0).unwrap();
    assert_eq!(page.len(), 40);
    assert!(more);
    let (next, more) = reader.history(&merge.to_string(), 40).unwrap();
    assert_eq!(next.len(), 3);
    assert!(!more);
    let details = reader.details(&binding, &merge.to_string()).unwrap();
    assert_eq!(details.parents.len(), 2);
    assert!(details.files.is_empty());
}

#[cfg(unix)]
#[test]
fn metadata_symlinks_are_not_followed() {
    let fixture = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), fixture.root.join(".git/objects/linked")).unwrap();
    let binding = discover(&fixture.root).unwrap().unwrap();
    assert!(Reader::open(&binding).is_err());
}

#[test]
fn oversized_index_and_tracked_files_fail_with_explicit_limits() {
    let fixture = Fixture::new();
    fixture.write("tracked.txt", "small\n");
    fixture.commit("Initial");
    let binding = discover(&fixture.root).unwrap().unwrap();
    let index_path = fixture.root.join(".git/index");
    let index = fs::read(&index_path).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&index_path)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert!(Reader::open(&binding).is_err());
    fs::write(index_path, index).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.root.join("tracked.txt"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let result = Reader::open(&binding)
        .unwrap()
        .status(&binding, "test".into());
    assert!(result.is_err());
}

fn git(fixture: &Fixture, args: &[&str], success: bool) -> String {
    let output = std::process::Command::new("git")
        .current_dir(&fixture.root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("Git is required for parity fixtures");
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
