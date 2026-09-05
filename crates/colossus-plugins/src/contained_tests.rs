use super::*;

#[cfg(unix)]
#[test]
fn bound_reads_reject_replaced_roots_ancestors_links_and_special_files() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().expect("temporary");
    let root = temporary.path().join("plugin");
    let outside = temporary.path().join("outside");
    fs::create_dir_all(root.join("resources")).expect("resources");
    fs::create_dir(&outside).expect("outside");
    fs::write(outside.join("secret"), "must never be returned").expect("secret");
    let reader = ReadRoot::bind(&root).expect("root binding");
    fs::rename(root.join("resources"), root.join("original")).expect("move ancestor");
    symlink(&outside, root.join("resources")).expect("ancestor symlink");
    assert!(reader.read(Path::new("resources/secret"), 1024).is_err());
    symlink(outside.join("secret"), root.join("leaf")).expect("leaf symlink");
    assert!(reader.read(Path::new("leaf"), 1024).is_err());
    assert!(reader.entries(Path::new("")).is_err());
    assert!(reader.read(Path::new("original"), 1024).is_err());
    for path in ["../outside/secret", "/etc/passwd", "resources/../leaf"] {
        assert!(reader.read(Path::new(path), 1024).is_err());
    }
    fs::rename(&root, temporary.path().join("retained")).expect("move original root");
    fs::create_dir(&root).expect("replacement root");
    fs::write(root.join("replacement"), "different tree").expect("replacement content");
    assert!(reader.read(Path::new("replacement"), 1024).is_err());
    assert!(reader.entries(Path::new("")).is_err());
}

#[test]
fn bounded_reads_use_opened_file_size_and_preserve_source_permissions() {
    let temporary = tempfile::tempdir().expect("temporary");
    fs::write(temporary.path().join("small"), "1234").expect("source");
    let before = fs::metadata(temporary.path())
        .expect("metadata")
        .permissions();
    let reader = ReadRoot::bind(temporary.path()).expect("read-only root");
    assert_eq!(
        reader.read(Path::new("small"), 4).expect("exact bound"),
        b"1234"
    );
    assert!(reader.read(Path::new("small"), 3).is_err());
    assert_eq!(
        before,
        fs::metadata(temporary.path())
            .expect("metadata")
            .permissions()
    );
}
