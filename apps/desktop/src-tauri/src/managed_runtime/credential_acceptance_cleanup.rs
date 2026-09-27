//! Teardown of this test's generated macOS directories after all sidecars have closed.

use std::{io, path::Path};

pub(super) fn prepare_removal(root: &Path) -> io::Result<()> {
    super::validate_fixture_root(root);
    crate::managed_runtime::test_directory_cleanup::prepare_removal(root)
}

#[test]
fn generated_read_only_plugin_tree_is_removed() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = super::PrivateAcceptanceRoot::new();
    let root = fixture.0.clone();
    let snapshot = root.join("plugins/immutable/nested");
    std::fs::create_dir_all(&snapshot).unwrap();
    let content = snapshot.join("manifest.json");
    std::fs::write(&content, b"{}").unwrap();
    std::fs::set_permissions(&content, std::fs::Permissions::from_mode(0o400)).unwrap();
    for directory in [
        &snapshot,
        &root.join("plugins/immutable"),
        &root.join("plugins"),
    ] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    }
    drop(fixture);
    assert!(
        !root.exists(),
        "generated immutable plugin tree must be removed"
    );
}

#[test]
fn generated_cleanup_refuses_links_without_changing_the_target() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let fixture = super::PrivateAcceptanceRoot::new();
    let outside = super::PrivateAcceptanceRoot::new();
    std::fs::set_permissions(&outside.0, std::fs::Permissions::from_mode(0o500)).unwrap();
    let link = fixture.0.join("outside");
    symlink(&outside.0, &link).unwrap();
    assert_eq!(
        prepare_removal(&fixture.0).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        std::fs::metadata(&outside.0).unwrap().permissions().mode() & 0o777,
        0o500
    );
    std::fs::remove_file(link).unwrap();
}
