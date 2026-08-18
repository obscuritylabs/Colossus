use std::path::{Path, PathBuf};

pub(crate) fn private_tempdir() -> tempfile::TempDir {
    #[cfg(windows)]
    {
        let directory = tempfile::Builder::new()
            .prefix("colossus-runtime-")
            .tempdir_in(current_user_profile())
            .expect("temporary parent");
        make_windows_private_test_home(directory.path());
        directory
    }

    #[cfg(not(windows))]
    {
        let directory = tempfile::tempdir().expect("private temporary root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .expect("private temporary root permissions");
        }
        directory
    }
}

#[cfg(windows)]
fn current_user_profile() -> PathBuf {
    let account = current_windows_user().0;
    let user_name = account
        .rsplit('\\')
        .next()
        .filter(|value| !value.is_empty())
        .expect("current Windows account name");
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let users_root = PathBuf::from(format!("{system_drive}\\Users"));
    for entry in std::fs::read_dir(&users_root).expect("Windows users directory") {
        let entry = entry.expect("Windows user profile entry");
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(user_name)
        {
            return entry.path();
        }
    }
    panic!("Windows user profile not found for {account}");
}

#[cfg(windows)]
fn make_windows_private_test_home(path: &Path) {
    let current_user_sid = current_windows_user().1;
    let icacls_path = path_for_icacls(path);
    let output = std::process::Command::new("icacls.exe")
        .arg(&icacls_path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("*{current_user_sid}:(OI)(CI)F"))
        .arg("*S-1-5-18:(OI)(CI)F")
        .arg("*S-1-5-32-544:(OI)(CI)F")
        .output()
        .expect("make isolated Windows test home private");
    assert!(
        output.status.success(),
        "failed to make isolated Windows test home private\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let binding = colossus_windows_native::BoundPath::open_directory(Path::new(&icacls_path))
        .expect("bind private isolated Windows test home");
    binding
        .validate_namespace_authority()
        .and_then(|()| binding.validate_private_owner_dacl())
        .and_then(|()| binding.revalidate())
        .expect("private isolated Windows test home");
}

#[cfg(windows)]
fn current_windows_user() -> (String, String) {
    let output = std::process::Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .expect("query current Windows user SID");
    assert!(
        output.status.success(),
        "failed to query current Windows user SID\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("Windows user SID output is UTF-8");
    let mut columns = stdout.split(',');
    let account = columns
        .next()
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
        .expect("Windows account in whoami output");
    let sid_tail = columns
        .next()
        .and_then(|value| value.trim().trim_matches('"').strip_prefix("S-"))
        .expect("Windows user SID in whoami output");
    assert!(
        sid_tail
            .chars()
            .all(|character| character.is_ascii_digit() || character == '-'),
        "unexpected Windows user SID: S-{sid_tail}"
    );
    (account, format!("S-{sid_tail}"))
}

#[cfg(windows)]
fn path_for_icacls(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        path.into_owned()
    }
}
