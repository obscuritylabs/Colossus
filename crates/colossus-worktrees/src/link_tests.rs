use super::*;
use crate::tests::Fixture;
use git2::{IndexEntry, IndexTime, Repository, Signature};
use std::{fs, path::Path};

fn commit_files(root: &Path, files: &[(&str, &[u8], u32)], materialize_links: bool) {
    let repo = Repository::open(root).unwrap();
    #[cfg(windows)]
    repo.config()
        .unwrap()
        .set_bool("core.symlinks", false)
        .unwrap();
    let mut index = repo.index().unwrap();
    for (path, content, mode) in files {
        let leaf = root.join(path);
        fs::create_dir_all(leaf.parent().unwrap()).unwrap();
        if *mode != 0o120_000 || cfg!(windows) || !materialize_links {
            fs::write(&leaf, content).unwrap();
        }
        #[cfg(unix)]
        if *mode == 0o120_000 && materialize_links {
            std::os::unix::fs::symlink(std::str::from_utf8(content).unwrap(), &leaf).unwrap();
        }
        index
            .add(&IndexEntry {
                ctime: IndexTime::new(0, 0),
                mtime: IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: *mode,
                uid: 0,
                gid: 0,
                file_size: u32::try_from(content.len()).unwrap(),
                id: repo.blob(content).unwrap(),
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            })
            .unwrap();
    }
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let signature = Signature::now("Colossus fixture", "fixture@example.invalid").unwrap();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "tracked link fixture",
        &tree,
        &[&parent],
    )
    .unwrap();
}

#[test]
fn tracked_internal_file_links_support_colossus_and_nested_checkouts() {
    let fixture = Fixture::new();
    commit_files(
        &fixture.repository,
        &[
            ("AGENTS.md", b"owned instructions\n", 0o100_644),
            ("CLAUDE.md", b"AGENTS.md", 0o120_000),
            ("nested/CLAUDE.md", b"../AGENTS.md", 0o120_000),
            ("nested/deeper/CLAUDE.md", b"../../AGENTS.md", 0o120_000),
        ],
        false,
    );
    let entry = fixture.create("links");
    assert_eq!(entry.dirty, Some(false));
    #[cfg(windows)]
    {
        assert!(
            !fs::symlink_metadata(entry.path.join("CLAUDE.md"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(entry.path.join("CLAUDE.md")).unwrap(),
            b"AGENTS.md"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(entry.state, WorktreeState::Active);
        for relative in ["", "nested", "nested/deeper"] {
            assert_eq!(
                fs::metadata(entry.path.join(relative))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
            );
        }
        assert!(
            fs::symlink_metadata(entry.path.join("CLAUDE.md"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(entry.path.join("nested/CLAUDE.md")).unwrap(),
            Path::new("../AGENTS.md")
        );
    }
    fixture.manager().archive(&entry.id).unwrap();
    assert_eq!(
        fixture.manager().list().unwrap().worktrees[0].dirty,
        Some(false)
    );
    assert_eq!(
        fs::read(entry.path.join("AGENTS.md")).unwrap(),
        b"owned instructions\n"
    );
    fixture.manager().remove(&entry.id).unwrap();
    assert_eq!(
        fs::read(fixture.repository.join("AGENTS.md")).unwrap(),
        b"owned instructions\n"
    );
    assert!(
        Repository::open(&fixture.repository)
            .unwrap()
            .find_branch(&entry.branch, git2::BranchType::Local)
            .is_ok()
    );
}

#[test]
fn escape_absolute_drive_chain_directory_and_unbounded_links_are_rejected() {
    for literal in [
        &b"../outside"[..],
        &b"/absolute"[..],
        &b"C:/outside"[..],
        &b"..\\outside"[..],
        &b"AGENTS.md\n"[..],
        &b"missing"[..],
        &b"nested"[..],
        &b"chain"[..],
        &b""[..],
        &b"\xff"[..],
        &vec![b'a'; 2049][..],
    ] {
        let fixture = Fixture::new();
        commit_files(
            &fixture.repository,
            &[
                ("AGENTS.md", b"keep\n", 0o100_644),
                ("nested/regular", b"keep\n", 0o100_644),
                ("chain", b"AGENTS.md", 0o120_000),
                ("CLAUDE.md", literal, 0o120_000),
            ],
            false,
        );
        let result = fixture.manager().create(&CreateWorktree {
            name: "invalid-link".into(),
            branch: None,
            base: None,
            existing_branch: false,
        });
        assert_eq!(
            result.unwrap_err().code,
            WorktreeErrorCode::UnsupportedRepository
        );
        assert!(fixture.manager().list().unwrap().worktrees.is_empty());
    }
}

#[test]
fn removal_revalidates_current_head_links_instead_of_creation_base() {
    let fixture = Fixture::new();
    let entry = fixture.create("new-head");
    commit_files(
        &entry.path,
        &[
            ("AGENTS.md", b"new tracked target\n", 0o100_644),
            ("CLAUDE.md", b"AGENTS.md", 0o120_000),
        ],
        true,
    );
    assert_eq!(
        fixture.manager().list().unwrap().worktrees[0].dirty,
        Some(false)
    );
    fixture.manager().archive(&entry.id).unwrap();
    fixture.manager().remove(&entry.id).unwrap();
    let unsafe_entry = fixture.create("unsafe-head");
    commit_files(
        &unsafe_entry.path,
        &[("CLAUDE.md", b"../../outside", 0o120_000)],
        true,
    );
    fixture.manager().archive(&unsafe_entry.id).unwrap();
    assert_eq!(
        fixture.manager().remove(&unsafe_entry.id).unwrap_err().code,
        WorktreeErrorCode::UnsupportedRepository
    );
    assert!(unsafe_entry.path.is_dir());
}

#[test]
fn tampered_materialized_or_real_link_is_retained() {
    let fixture = Fixture::new();
    commit_files(
        &fixture.repository,
        &[
            ("AGENTS.md", b"keep\n", 0o100_644),
            ("CLAUDE.md", b"AGENTS.md", 0o120_000),
        ],
        false,
    );
    let entry = fixture.create("tampered-link");
    fixture.manager().archive(&entry.id).unwrap();
    fs::remove_file(entry.path.join("CLAUDE.md")).unwrap();
    #[cfg(windows)]
    fs::write(entry.path.join("CLAUDE.md"), "README.md").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("README.md", entry.path.join("CLAUDE.md")).unwrap();
    assert!(fixture.manager().remove(&entry.id).is_err());
    assert!(fs::symlink_metadata(entry.path.join("CLAUDE.md")).is_ok());
    assert_eq!(
        fs::read(fixture.repository.join("AGENTS.md")).unwrap(),
        b"keep\n"
    );
}

#[test]
#[ignore = "explicit read-only source-tree admission proof"]
fn real_colossus_head_admits_its_claude_file_link_without_mutation() {
    let path =
        std::env::var_os("COLOSSUS_WORKTREE_SOURCE_ADMISSION").expect("explicit source root");
    let repo = Repository::open(Path::new(&path)).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    let links = crate::tree::inspect_commit(&repo, &commit).unwrap();
    let link = links
        .get(Path::new("CLAUDE.md"))
        .expect("real tracked CLAUDE link");
    assert_eq!(link.literal, "AGENTS.md");
    assert_eq!(link.target, Path::new("AGENTS.md"));
}

#[cfg(unix)]
#[test]
fn untracked_link_and_replaced_link_parent_are_never_followed_or_removed() {
    let fixture = Fixture::new();
    commit_files(
        &fixture.repository,
        &[
            ("AGENTS.md", b"keep\n", 0o100_644),
            ("nested/CLAUDE.md", b"../AGENTS.md", 0o120_000),
        ],
        false,
    );
    let entry = fixture.create("parent-guard");
    fixture.manager().archive(&entry.id).unwrap();
    std::os::unix::fs::symlink("AGENTS.md", entry.path.join("untracked-link")).unwrap();
    assert_eq!(
        fixture.manager().remove(&entry.id).unwrap_err().code,
        WorktreeErrorCode::Dirty
    );
    assert!(fs::symlink_metadata(entry.path.join("untracked-link")).is_ok());
    fs::remove_file(entry.path.join("untracked-link")).unwrap();
    fs::rename(entry.path.join("nested"), entry.path.join("saved-nested")).unwrap();
    std::os::unix::fs::symlink(&fixture.repository, entry.path.join("nested")).unwrap();
    assert!(fixture.manager().remove(&entry.id).is_err());
    assert!(fs::symlink_metadata(entry.path.join("nested")).is_ok());
    assert_eq!(
        fs::read(fixture.repository.join("AGENTS.md")).unwrap(),
        b"keep\n"
    );
}

#[cfg(unix)]
#[test]
fn leaf_unlink_revalidates_parent_after_preflight_without_following_replacement() {
    let fixture = Fixture::new();
    commit_files(
        &fixture.repository,
        &[
            ("AGENTS.md", b"keep source target\n", 0o100_644),
            ("nested/CLAUDE.md", b"../AGENTS.md", 0o120_000),
        ],
        false,
    );
    let entry = fixture.create("leaf-parent");
    let repo = Repository::open(&entry.path).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    let policy = crate::tree::inspect_commit(&repo, &commit).unwrap();
    let mut leaves = crate::filesystem::inspect_checkout(&entry.path, &policy).unwrap();
    assert_eq!(leaves.len(), 1);
    fs::rename(entry.path.join("nested"), entry.path.join("saved-nested")).unwrap();
    std::os::unix::fs::symlink(&fixture.repository, entry.path.join("nested")).unwrap();
    assert_eq!(
        leaves.pop().unwrap().unlink().unwrap_err().code,
        WorktreeErrorCode::Ownership
    );
    assert!(
        fs::symlink_metadata(entry.path.join("saved-nested/CLAUDE.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(entry.path.join("nested"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(fixture.repository.join("AGENTS.md")).unwrap(),
        b"keep source target\n"
    );
}
