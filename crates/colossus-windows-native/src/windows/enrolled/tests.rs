use super::*;
use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

#[test]
fn mutable_directory_reopen_coexists_with_the_retained_delete_allocation() {
    let profile = crate::tests::native_test_root();
    let temporary = tempfile::tempdir_in(profile).expect("fresh private fixture parent");
    let parent = temporary.path().join("owned");
    crate::create_private_directory(&parent).expect("protected parent");
    let path = parent.join("profile");
    let mut creation = PrivateDirectoryCreation::create(&path).expect("exact allocation");
    let identity = FileIdentity::of(creation.allocation_file().unwrap()).unwrap();
    let access = appcontainer::PACKAGE_PROFILE_RIGHTS | SYNCHRONIZE;

    // Keep the metadata-only root open exactly as enrolled::open does before
    // reopening its leaf. Core's original DELETE-capable allocation stays live.
    let current_root = open_exact(&path, BoundKind::Directory, GENERIC_READ, true, true)
        .expect("metadata root coexists with the retained allocation");
    let reopened = open_leaf(&path, BoundKind::Directory, access, false)
        .expect("mutable enrolled directory shares the retained allocation");
    assert_eq!(FileIdentity::of(&reopened).unwrap(), identity);

    // Descendant mutation also retains the root metadata/mutation handles, as
    // transfer staging does when it allocates upload/download subdirectories.
    let child_path = path.join("child");
    let mut child = PrivateDirectoryCreation::create(&child_path).expect("exact child allocation");
    let ancestor = open_exact(&path, BoundKind::Directory, GENERIC_READ, true, true)
        .expect("retained enrolled ancestor");
    let child_reopened = open_leaf(&child_path, BoundKind::Directory, access, false)
        .expect("mutable descendant shares its retained allocation");
    assert_eq!(
        FileIdentity::of(&child_reopened).unwrap(),
        FileIdentity::of(child.allocation_file().unwrap()).unwrap()
    );

    // Explicit immutability still refuses this conflicting sharing contract.
    let error = open_leaf(&path, BoundKind::Directory, access, true)
        .expect_err("immutable directory cannot coexist with a DELETE-capable owner");
    assert!(matches!(
        error,
        WindowsNativeError::Io { source, .. }
            if source.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32)
    ));

    drop(child_reopened);
    child
        .remove_empty()
        .expect("positive exact child retirement");
    drop((ancestor, reopened, current_root));
    creation
        .remove_empty()
        .expect("positive exact allocation retirement");
    assert!(!path.exists());
}
