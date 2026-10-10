use super::*;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _, symlink};
struct Allocation(PathBuf);
impl Drop for Allocation {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn profile() -> Allocation {
    let path = std::env::temp_dir().join(random_name("transfer-test-").unwrap().to_str().unwrap());
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    Allocation(path)
}
#[test]
fn native_upload_names_and_bytes_are_private_bounded_and_retained_until_shutdown() {
    let profile = profile();
    let mut stage = Stage::create(&profile.0).unwrap();
    let upload = stage.upload(b"synthetic", "report.txt").unwrap();
    assert_eq!(upload.path().file_name().unwrap(), "report.txt");
    assert_eq!(std::fs::read(upload.path()).unwrap(), b"synthetic");
    assert_eq!(upload._file.metadata().unwrap().mode() & 0o777, 0o600);
    for name in ["../escape", "\\escape", ".", "..", "new\nline", ""] {
        assert!(stage.upload(b"x", name).is_err());
    }
    assert!(stage.upload(&vec![0; MAX_FILE_BYTES + 1], "large").is_err());
    let path = upload.path().to_owned();
    drop(upload);
    assert!(path.exists());
    stage.finish().unwrap();
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(&profile.0).unwrap().count(), 0);
}
fn completed_file(reservation: &Reservation, bytes: &[u8]) {
    std::fs::write(reservation.path(), bytes).unwrap();
    std::fs::set_permissions(reservation.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn completed_download_binds_the_actual_renamed_inode_and_allows_empty_files() {
    let profile = profile();
    let mut stage = Stage::create(&profile.0).unwrap();
    let reservation = stage.reserve_download().unwrap();
    assert!(!reservation.path().exists());
    let partial = reservation.path().with_extension("crdownload");
    std::fs::write(&partial, b"actual completed bytes").unwrap();
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600)).unwrap();
    let inode = std::fs::metadata(&partial).unwrap().ino();
    std::fs::rename(&partial, reservation.path()).unwrap();
    assert_eq!(
        &*stage.completed(reservation, 22).unwrap(),
        b"actual completed bytes"
    );
    assert_eq!(
        stage.slots[0]
            .file
            .as_ref()
            .unwrap()
            .metadata()
            .unwrap()
            .ino(),
        inode
    );
    let empty = stage.reserve_download().unwrap();
    completed_file(&empty, b"");
    assert!(stage.completed(empty, 0).unwrap().is_empty());
    stage.finish().unwrap();
}
#[test]
fn download_rejects_indirection_shared_inodes_wrong_sizes_and_foreign_reservations() {
    let profile = profile();
    let mut stage = Stage::create(&profile.0).unwrap();
    let reservation = stage.reserve_download().unwrap();
    let outside = profile.0.join("outside");
    std::fs::write(&outside, b"preserve").unwrap();
    symlink(&outside, reservation.path()).unwrap();
    assert!(stage.completed(reservation, 8).is_err());
    let shared = stage.reserve_download().unwrap();
    completed_file(&shared, b"x");
    std::fs::hard_link(shared.path(), profile.0.join("alias")).unwrap();
    assert!(stage.completed(shared, 1).is_err());
    let wrong = stage.reserve_download().unwrap();
    completed_file(&wrong, b"actual");
    assert!(stage.completed(wrong, 1).is_err());
    let foreign = stage.reserve_download().unwrap();
    completed_file(&foreign, b"actual");
    let mut other = Stage::create(&profile.0).unwrap();
    assert!(other.completed(foreign, 6).is_err());
    stage.finish().unwrap();
    other.finish().unwrap();
    assert_eq!(std::fs::read(outside).unwrap(), b"preserve");
}
#[test]
fn stage_limits_reservations_and_preserves_an_unknown_slot_replacement() {
    let profile = profile();
    let mut stage = Stage::create(&profile.0).unwrap();
    for _ in 0..MAX_FILES {
        stage.reserve_download().unwrap();
    }
    assert!(stage.reserve_download().is_err());
    let path = stage.slots[0].directory.path().to_owned();
    let moved = profile.0.join("moved");
    std::fs::rename(&path, &moved).unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    std::fs::write(path.join("unknown"), b"preserve").unwrap();
    let root = stage.directory.path().to_owned();
    assert_eq!(stage.finish(), Err(BrowserDriverError::OutcomeUnknown));
    drop(stage);
    assert!(moved.exists());
    let unknown = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".retired-browser-")
        })
        .unwrap();
    assert_eq!(
        std::fs::read(unknown.path().join("unknown")).unwrap(),
        b"preserve"
    );
}
#[test]
fn dropping_without_a_native_shutdown_acknowledgement_preserves_staged_inputs() {
    let profile = profile();
    let mut stage = Stage::create(&profile.0).unwrap();
    let upload = stage
        .upload(b"preserve until native reap", "report.txt")
        .unwrap();
    let path = upload.path().to_owned();
    drop(upload);
    drop(stage);
    assert_eq!(std::fs::read(path).unwrap(), b"preserve until native reap");
}
