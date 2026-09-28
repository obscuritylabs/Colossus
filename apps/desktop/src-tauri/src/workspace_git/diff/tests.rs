use super::*;
use crate::workspace_git::discovery::discover;
use std::fs;

fn commit(repo: &git2::Repository) -> String {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Test", "test@example.com").unwrap();
    let parent = repo.head().ok().and_then(|r| r.peel_to_commit().ok());
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "Test commit",
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
    .to_string()
}

fn request(path: &str, source: DiffSource) -> DiffRequest {
    DiffRequest {
        workspace_id: "test".into(),
        repository_id: "test".into(),
        path: path.into(),
        source,
        commit_id: None,
    }
}

#[test]
fn staged_and_unstaged_are_distinct_and_deletions_keep_original_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    fs::write(root.join("file.rs"), "fn old() {}\n").unwrap();
    commit(&repo);
    fs::write(root.join("file.rs"), "fn staged() {}\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("file.rs")).unwrap();
    index.write().unwrap();
    fs::write(root.join("file.rs"), "fn working() {}\n").unwrap();
    let binding = discover(&root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    let staged = reader
        .diff(&binding, &request("file.rs", DiffSource::Staged))
        .unwrap();
    assert_eq!(staged.before.content.as_deref(), Some("fn old() {}\n"));
    assert_eq!(staged.after.content.as_deref(), Some("fn staged() {}\n"));
    assert_eq!((staged.additions, staged.deletions), (1, 1));
    assert_eq!(staged.hunks[0].lines[0].old_line, Some(1));
    let working = reader
        .diff(&binding, &request("file.rs", DiffSource::Unstaged))
        .unwrap();
    assert_eq!(working.before.content.as_deref(), Some("fn staged() {}\n"));
    assert_eq!(working.after.content.as_deref(), Some("fn working() {}\n"));
    fs::remove_file(root.join("file.rs")).unwrap();
    let deleted = reader
        .diff(&binding, &request("file.rs", DiffSource::Unstaged))
        .unwrap();
    assert_eq!(deleted.after.state, "absent");
    assert_eq!(deleted.deletions, 1);
}

#[test]
fn initial_history_and_renames_use_real_parent_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    fs::write(root.join("old.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    let first = commit(&repo);
    fs::rename(root.join("old.txt"), root.join("new.txt")).unwrap();
    let mut index = repo.index().unwrap();
    index.remove_path(Path::new("old.txt")).unwrap();
    index.add_path(Path::new("new.txt")).unwrap();
    index.write().unwrap();
    let binding = discover(&root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    let renamed = reader
        .diff(&binding, &request("new.txt", DiffSource::Staged))
        .unwrap();
    assert_eq!(renamed.previous_path.as_deref(), Some("old.txt"));
    assert_eq!(renamed.before.content, renamed.after.content);
    let mut initial = request("old.txt", DiffSource::Commit);
    initial.commit_id = Some(first);
    let initial = reader.diff(&binding, &initial).unwrap();
    assert_eq!(initial.before.state, "absent");
    assert_eq!(initial.additions, 4);
}

#[test]
fn empty_binary_large_and_unsafe_content_have_explicit_states() {
    assert_eq!(text_version(b"").state, "text");
    assert_eq!(text_version(b"\0").state, "binary");
    assert_eq!(text_version(&[255]).state, "binary");
    assert_eq!(text_version("a\u{202e}b".as_bytes()).state, "binary");
    assert_eq!(text_version(&vec![b'x'; 256 * 1024 + 1]).state, "too_large");
    assert_eq!(
        text_version("\n".repeat(MAX_LINES).as_bytes()).state,
        "too_large"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    git2::Repository::init(&root).unwrap();
    for (name, data) in [
        ("empty.txt", vec![]),
        ("binary.bin", vec![0]),
        ("large.txt", vec![b'a'; 256 * 1024 + 1]),
    ] {
        fs::write(root.join(name), data).unwrap();
    }
    let binding = discover(&root).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    for (name, state) in [
        ("empty.txt", "text"),
        ("binary.bin", "binary"),
        ("large.txt", "too_large"),
    ] {
        let diff = reader
            .diff(&binding, &request(name, DiffSource::Untracked))
            .unwrap();
        assert_eq!(diff.after.state, state);
    }
}

#[test]
fn historical_paths_cannot_bypass_file_preview_exclusions_or_scope() {
    for path in [
        "../outside",
        "/etc/passwd",
        "a\\b",
        "C:/file",
        ".env",
        "src/.ENV.local",
        ".git/config",
        "a/secret.key",
        "a/../b",
        "a//b",
        "a/\u{202e}b",
    ] {
        assert!(visible_path(path).is_err(), "{path}");
    }
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    fs::create_dir(root.join("inside")).unwrap();
    fs::write(root.join("outside.txt"), "private").unwrap();
    fs::write(root.join("inside/safe.txt"), "visible").unwrap();
    let id = commit(&repo);
    let binding = discover(&root.join("inside")).unwrap().unwrap();
    let reader = Reader::open(&binding).unwrap();
    let mut input = request("outside.txt", DiffSource::Commit);
    input.commit_id = Some(id);
    assert!(reader.diff(&binding, &input).is_err());
    input.path = "safe.txt".into();
    assert_eq!(
        reader
            .diff(&binding, &input)
            .unwrap()
            .after
            .content
            .as_deref(),
        Some("visible")
    );
}

#[test]
fn display_bounds_never_claim_a_truncated_diff_is_complete() {
    let mut result = FileDiff {
        path: "many.txt".into(),
        previous_path: None,
        language: "text".into(),
        before: version("absent", None),
        after: text_version("new\n".repeat(3000).as_bytes()),
        hunks: vec![],
        additions: 0,
        deletions: 0,
        truncated: false,
        note: None,
    };
    build_hunks(&mut result).unwrap();
    assert!(result.truncated);
    assert_eq!(result.additions, 3000);
    assert_eq!(
        result.hunks.iter().map(|h| h.lines.len()).sum::<usize>(),
        MAX_DIFF_LINES
    );
}
