use super::*;

#[test]
fn profile_path_requires_a_complete_bounded_absolute_native_value() {
    let value = "C:\\Users\\native-profile\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    assert_eq!(
        profile_path(&value, value.len() as u32).expect("complete profile path"),
        PathBuf::from("C:\\Users\\native-profile")
    );
    for written in [0, 1, value.len() as u32 - 1, value.len() as u32 + 1] {
        assert!(profile_path(&value, written).is_err());
    }
    let mut embedded_nul = value.clone();
    embedded_nul[5] = 0;
    assert!(profile_path(&embedded_nul, embedded_nul.len() as u32).is_err());
    for relative in ["relative\0", "C:relative\0", "\\rooted\0"] {
        let value = relative.encode_utf16().collect::<Vec<_>>();
        assert!(profile_path(&value, value.len() as u32).is_err());
    }
}

#[test]
fn token_profile_supports_the_existing_exclusive_private_allocator() {
    let profile = current_user_profile_directory().expect("current token's protected profile");
    let parent = crate::BoundPath::open_directory(&profile).expect("bind token profile");
    parent
        .validate_immutable_directory_dacl()
        .expect("profile denies untrusted namespace mutation");
    parent
        .validate_ancestor_namespace_authority()
        .expect("profile ancestry retains namespace authority");
    parent.revalidate().expect("same token profile identity");
    let temporary = tempfile::Builder::new()
        .prefix("colossus-native-token-profile-")
        .tempdir_in(&profile)
        .expect("owned temporary fixture beneath token profile");
    let basename = temporary.path().file_name().unwrap().to_os_string();
    temporary.close().expect("retire unused name reservation");
    let mut creation = crate::PrivateDirectoryCreation::create_in_current_user_profile(&basename)
        .expect("token parent allocator creates one owner-private child");
    let bound = creation.bind().expect("bind exact created child");
    bound
        .validate_private_owner_dacl()
        .expect("child retains private DACL");
    assert!(crate::PrivateDirectoryCreation::create_in_current_user_profile(&basename).is_err());
    let child = creation.path().join("private");
    let mut nested = crate::PrivateDirectoryCreation::create(&child)
        .expect("existing allocator still accepts the owner-private child as parent");
    nested.remove_empty().expect("retire exact nested fixture");
    drop(nested);
    drop(bound);
    creation.remove_empty().expect("retire exact empty fixture");
}

#[test]
fn token_profile_allocation_rejects_non_leaf_names() {
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "parent\\child",
        "C:\\absolute",
        "a:b",
        "nul\0",
    ] {
        assert!(
            crate::PrivateDirectoryCreation::create_in_current_user_profile(std::ffi::OsStr::new(
                name
            ))
            .is_err()
        );
    }
    assert!(
        crate::PrivateDirectoryCreation::create_in_current_user_profile(std::ffi::OsStr::new(
            &"a".repeat(256)
        ))
        .is_err()
    );
}

#[test]
fn token_profile_ignores_ambient_directory_overrides() {
    const FIXTURE: &str = "COLOSSUS_NATIVE_TOKEN_PROFILE_FIXTURE";
    const UNTRUSTED: &str = r"C:\ColossusAmbientProfileMustNotSelectParent";
    if std::env::var_os(FIXTURE).is_some() {
        assert_eq!(std::env::var("USERPROFILE").unwrap(), UNTRUSTED);
        let profile =
            current_user_profile_directory().expect("native token profile ignores override");
        assert!(profile.is_absolute());
        assert!(
            !profile
                .to_string_lossy()
                .contains("ColossusAmbientProfileMustNotSelectParent")
        );
        return;
    }
    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "windows::profile::tests::token_profile_ignores_ambient_directory_overrides",
            "--nocapture",
        ])
        .env(FIXTURE, "1");
    for name in [
        "USERPROFILE",
        "HOME",
        "LOCALAPPDATA",
        "APPDATA",
        "TEMP",
        "TMP",
    ] {
        command.env(name, UNTRUSTED);
    }
    assert!(
        command
            .status()
            .expect("isolated environment fixture")
            .success()
    );
}
