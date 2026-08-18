use std::{
    fs,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    process::Command,
};

#[must_use = "keep the isolated home alive until the spawned process exits"]
pub struct IsolatedUserHome {
    path: PathBuf,
    #[cfg(windows)]
    _directory: tempfile::TempDir,
}

impl IsolatedUserHome {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn colossus_home(&self) -> PathBuf {
        self.path.join(".colossus-home")
    }

    pub fn temporary_directory(&self) -> PathBuf {
        self.path.join("tmp")
    }

    #[cfg(windows)]
    pub fn local_app_data(&self) -> PathBuf {
        self.path.join("AppData").join("Local")
    }
}

#[allow(dead_code)]
pub struct IsolatedCommand {
    command: Command,
    _home: IsolatedUserHome,
}

#[allow(dead_code)]
impl IsolatedCommand {
    pub fn new(command: Command, home: IsolatedUserHome) -> Self {
        Self {
            command,
            _home: home,
        }
    }
}

impl Deref for IsolatedCommand {
    type Target = Command;

    fn deref(&self) -> &Self::Target {
        &self.command
    }
}

impl DerefMut for IsolatedCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.command
    }
}

pub fn tempdir() -> std::io::Result<tempfile::TempDir> {
    #[cfg(windows)]
    {
        let directory = tempfile::Builder::new()
            .prefix("colossus-cli-workspace-")
            .tempdir_in(current_user_profile())?;
        make_windows_private_test_home(directory.path());
        Ok(directory)
    }

    #[cfg(not(windows))]
    {
        tempfile::tempdir()
    }
}

/// Isolate a spawned Colossus process from the developer's real user home.
///
/// The test root must already exist. Colossus creates the application home itself,
/// which also exercises the runtime creation path without touching ambient state.
pub fn isolate_user_home(command: &mut Command, test_root: &Path) -> IsolatedUserHome {
    let home = isolated_user_home(test_root);
    command
        .env("HOME", home.path())
        .env("COLOSSUS_HOME", home.colossus_home());
    #[cfg(windows)]
    command
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.local_app_data())
        .env("TEMP", home.temporary_directory())
        .env("TMP", home.temporary_directory());
    home
}

pub fn isolated_user_home(test_root: &Path) -> IsolatedUserHome {
    let test_root = fs::canonicalize(test_root).expect("canonical isolated test root");
    assert!(
        test_root.is_absolute(),
        "isolated test root must be absolute"
    );

    #[cfg(windows)]
    {
        let directory = windows_private_test_home();
        let user_home = directory.path().to_path_buf();
        make_windows_private_test_home(&user_home);
        let temporary_directory = user_home.join("tmp");
        colossus_windows_native::create_private_directory(&temporary_directory)
            .expect("private isolated Windows temporary directory");
        let local_app_data = user_home.join("AppData").join("Local");
        colossus_windows_native::create_private_directory(&user_home.join("AppData"))
            .expect("private isolated Windows app-data root");
        colossus_windows_native::create_private_directory(&local_app_data)
            .expect("private isolated Windows local app-data root");
        colossus_windows_native::create_private_directory(&local_app_data.join("Packages"))
            .expect("private isolated Windows AppContainer packages root");
        IsolatedUserHome {
            path: user_home,
            _directory: directory,
        }
    }

    #[cfg(not(windows))]
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let temporary_root =
                fs::canonicalize(std::env::temp_dir()).expect("canonical temporary root");
            if let Ok(relative_home) = test_root.strip_prefix(&temporary_root) {
                let mut private_parent = temporary_root;
                for component in relative_home.components() {
                    private_parent.push(component);
                    fs::set_permissions(&private_parent, fs::Permissions::from_mode(0o700))
                        .expect("private isolated test home ancestry permissions");
                }
            } else {
                fs::set_permissions(&test_root, fs::Permissions::from_mode(0o700))
                    .expect("private isolated test home permissions");
            }
        }
        IsolatedUserHome { path: test_root }
    }
}

#[cfg(windows)]
fn windows_private_test_home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("colossus-cli-home-")
        .tempdir_in(current_user_profile())
        .expect("private isolated Windows test home")
}

#[cfg(windows)]
fn current_user_profile() -> PathBuf {
    let account = current_user_account();
    let user_name = account
        .rsplit('\\')
        .next()
        .filter(|value| !value.is_empty())
        .expect("current Windows account name");
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let users_root = PathBuf::from(format!("{system_drive}\\Users"));
    for entry in fs::read_dir(&users_root).expect("Windows users directory") {
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
fn make_windows_private_test_home(user_home: &Path) {
    let current_user_sid = current_user_sid();
    let grant_current_user = format!("*{current_user_sid}:(OI)(CI)F");
    let icacls_path = path_for_icacls(user_home);
    let output = Command::new("icacls.exe")
        .arg(&icacls_path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(grant_current_user)
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
    let binding_path = Path::new(&icacls_path);
    let binding =
        colossus_windows_native::BoundPath::open_directory(binding_path).unwrap_or_else(|error| {
            let acl = Command::new("icacls.exe").arg(&icacls_path).output().ok();
            panic!(
                "bind private isolated Windows test home failed for {}\nerror={error:?}\n\
                 icacls stdout={}\nicacls stderr={}\npost-acl stdout={}\npost-acl stderr={}",
                icacls_path,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                acl.as_ref()
                    .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                    .unwrap_or_else(|| "<icacls query failed>".to_owned()),
                acl.as_ref()
                    .map(|output| String::from_utf8_lossy(&output.stderr).into_owned())
                    .unwrap_or_default()
            )
        });
    binding
        .validate_namespace_authority()
        .and_then(|()| binding.validate_private_owner_dacl())
        .and_then(|()| binding.revalidate())
        .expect("private isolated Windows test home");
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

#[cfg(windows)]
fn current_user_sid() -> String {
    current_windows_user().1
}

#[cfg(windows)]
fn current_user_account() -> String {
    current_windows_user().0
}

#[cfg(windows)]
fn current_windows_user() -> (String, String) {
    let output = Command::new("whoami.exe")
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
