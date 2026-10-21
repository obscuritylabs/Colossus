use super::*;
use colossus_observability::{LocalLogLevel, LocalLogSink};

fn native_log_root(home: &Path, sidecar: bool) -> std::path::PathBuf {
    let relative = if sidecar {
        Path::new("workspaces").join("a".repeat(64)).join("desktop")
    } else {
        Path::new("desktop").to_owned()
    };
    let mut root = home.to_owned();
    for part in &relative {
        root.push(part);
        create_private_directory(&root).unwrap();
    }
    root
}

fn logger(root: &Path, sidecar: bool) -> LocalLogSink {
    LocalLogSink::open(
        &ConfinedRoot::bind(root).unwrap(),
        if sidecar { "sidecar" } else { "desktop" },
        LocalLogLevel::Info,
    )
    .unwrap()
}

#[test]
fn cleanup_requires_native_log_consumers_to_close_before_removing_the_home() {
    for sidecar in [false, true] {
        let (_guard, home) = fixture();
        create_private_file(&home.join("AGENTS.md"), b"keep until idle").unwrap();
        let root = native_log_root(&home, sidecar);
        let sink = logger(&root, sidecar);
        assert_eq!(cleanup(&home), Err(CleanupError::Busy));
        assert!(home.join("AGENTS.md").is_file());
        assert!(root.join("logs").is_dir());
        drop(sink);
        cleanup(&home).unwrap();
        assert!(!home.exists());
    }
}

#[test]
fn cleanup_preserves_unknown_files_in_native_log_directories() {
    for sidecar in [false, true] {
        let (_guard, home) = fixture();
        create_private_file(&home.join("AGENTS.md"), b"keep on unknown ownership").unwrap();
        let root = native_log_root(&home, sidecar);
        drop(logger(&root, sidecar));
        let unknown = root.join("logs/operator-notes.txt");
        create_private_file(&unknown, b"unrecognized data").unwrap();
        assert_eq!(cleanup(&home), Err(CleanupError::UnsafeData));
        assert!(home.join("AGENTS.md").is_file());
        assert_eq!(fs::read(unknown).unwrap(), b"unrecognized data");
    }
}

#[test]
fn cleanup_preserves_native_log_files_with_an_external_hard_link() {
    for sidecar in [false, true] {
        let (guard, home) = fixture();
        let root = native_log_root(&home, sidecar);
        drop(logger(&root, sidecar));
        let log = root.join(if sidecar {
            "logs/sidecar-0.jsonl"
        } else {
            "logs/desktop-0.jsonl"
        });
        let external = guard.path().join("outside-log.jsonl");
        fs::hard_link(&log, &external).unwrap();
        assert_eq!(cleanup(&home), Err(CleanupError::UnsafeData));
        assert!(log.is_file());
        assert!(external.is_file());
    }
}
