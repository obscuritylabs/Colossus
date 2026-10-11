use super::*;

#[test]
fn retained_private_allocation_accepts_low_label_without_general_sacl_privilege() {
    let profile = std::env::var_os("USERPROFILE").expect("native user profile");
    let temporary = tempfile::tempdir_in(profile).expect("fresh private fixture parent");
    let parent = temporary.path().join("owned");
    crate::create_private_directory(&parent).expect("protected parent");
    let directory = parent.join("profile");
    let mut creation = PrivateDirectoryCreation::create(&directory).expect("exact allocation");
    LowLabel::new()
        .unwrap()
        .apply(creation.allocation_file().unwrap())
        .expect("mandatory label uses LABEL access, not general SACL privilege");
    let bound = creation
        .bind()
        .expect("same private allocation after label change");
    bound
        .validate_private_owner_dacl()
        .expect("package access not granted yet");
    validate_low_label(creation.allocation_file().unwrap()).expect("actual Low label readback");
    let child = directory.join("child");
    crate::create_private_directory(&child).expect("child inherits admitted Low label");
    let child_binding = open_bound(&child, BoundKind::Directory).unwrap();
    validate_low_label(&child_binding.file).expect("inherited directory label");
    let leaf = child.join("fixture.txt");
    crate::create_private_file(&leaf, b"owned fixture").expect("Low-labelled fixture file");
    let leaf_binding = open_bound(&leaf, BoundKind::File).unwrap();
    validate_low_label(&leaf_binding.file).expect("inherited file label");
    drop((leaf_binding, child_binding, bound));
    std::fs::remove_file(leaf).unwrap();
    std::fs::remove_dir(child).unwrap();
    creation
        .remove_empty()
        .expect("positive exact allocation retirement");
    assert!(!directory.exists());
}
