use super::*;
use std::io::Read as _;

fn deletion(path: &Path) -> BoundPathInner {
    open_bound_with_access(path, BoundKind::File, GENERIC_READ | DELETE, false)
        .expect("exact DELETE-capable file")
}

#[test]
fn unlink_proves_zero_links_while_original_readers_remain_open() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("transfer");
    fs::write(&path, b"original bytes").unwrap();
    let bound = open_bound(&path, BoundKind::File).unwrap();
    let reader = bound.file.try_clone().unwrap();
    let identity = bound.identity;

    unlink(&bound, deletion(&path)).expect("close deletion handle before proving unlink");
    assert_eq!(file_information(&reader).unwrap().nNumberOfLinks, 0);
    assert!(matches!(
        file_link_count(&reader),
        Err(WindowsNativeError::InvalidInput)
    ));
    assert_eq!(file_identity(&reader).unwrap(), identity);
    assert!(!path.exists());
    let mut bytes = Vec::new();
    (&reader).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original bytes");

    fs::write(&path, b"replacement bytes").unwrap();
    assert!(matches!(
        unlink(&bound, deletion(&path)),
        Err(WindowsNativeError::IdentityChanged)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"replacement bytes");
}

#[test]
fn unlink_rejects_multiple_links_without_removing_either_name() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("transfer");
    let alias = fixture.path().join("alias");
    fs::write(&path, b"original bytes").unwrap();
    let bound = open_bound(&path, BoundKind::File).unwrap();
    fs::hard_link(&path, &alias).unwrap();

    assert!(matches!(
        unlink(&bound, deletion(&path)),
        Err(WindowsNativeError::IdentityChanged)
    ));
    assert_eq!(file_link_count(&bound.file).unwrap(), 2);
    assert_eq!(fs::read(&path).unwrap(), b"original bytes");
    assert_eq!(fs::read(&alias).unwrap(), b"original bytes");
}
