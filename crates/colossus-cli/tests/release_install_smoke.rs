//! Offline installation acceptance for packaged native Rust executables.

#[path = "support/process.rs"]
mod process_support;

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7171717171717171717171717171717171717171717171717171717171717171";
const SIGNING_KEY: &str = "8282828282828282828282828282828282828282828282828282828282828282";

#[cfg(unix)]
fn create_private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    fs::create_dir_all(path).expect("private test directory");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .expect("private test directory permissions");
}

fn release_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        host => panic!("unsupported release installer test host: {host:?}"),
    }
}

fn release_channel() -> &'static str {
    if env!("CARGO_PKG_VERSION").contains("-preview.") {
        "preview"
    } else {
        "stable"
    }
}

fn write_install_metadata(package: &Path) {
    fs::write(
        package.join("install-metadata"),
        format!(
            "schema_version=1\nversion={}\ntarget={}\nchannel={}\ndistribution_origin=https://github.com/obscuritylabs/Colossus/releases\ninstaller_kind=direct\n",
            env!("CARGO_PKG_VERSION"),
            release_target(),
            release_channel()
        ),
    )
    .expect("package installation metadata");
}

#[cfg(unix)]
fn prepare_package(binary: &Path, package: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let packaged_binary = package.join("colossus");
    fs::copy(binary, &packaged_binary).expect("package binary");
    fs::set_permissions(&packaged_binary, fs::Permissions::from_mode(0o755))
        .expect("binary permissions");
    let installer = package.join("install.sh");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/install.sh"),
        &installer,
    )
    .expect("package installer");
    fs::set_permissions(&installer, fs::Permissions::from_mode(0o755))
        .expect("installer permissions");
    write_install_metadata(package);
    installer
}

#[cfg(windows)]
fn prepare_package(binary: &Path, package: &Path) -> PathBuf {
    fs::copy(binary, package.join("colossus.exe")).expect("package binary");
    let installer = package.join("install.ps1");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/install.ps1"),
        &installer,
    )
    .expect("package installer");
    write_install_metadata(package);
    installer
}

#[cfg(unix)]
fn install(installer: &Path, prefix: &Path) -> Output {
    Command::new("/bin/sh")
        .arg(installer)
        .arg("--prefix")
        .arg(prefix)
        .env("COLOSSUS_HOME", prefix.join("home/.colossus"))
        .env("XDG_DATA_HOME", prefix.join("data"))
        .output()
        .expect("run installer")
}

#[cfg(windows)]
fn install(installer: &Path, prefix: &Path) -> Output {
    Command::new("pwsh")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
        ])
        .arg("-File")
        .arg(installer)
        .arg("-Prefix")
        .arg(prefix)
        .env("COLOSSUS_HOME", prefix.join("home/.colossus"))
        .env("LOCALAPPDATA", prefix.join("data"))
        .output()
        .expect("run installer")
}

fn installed_binary(prefix: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        prefix.join("bin/colossus")
    }
    #[cfg(windows)]
    {
        prefix.join("bin/colossus.exe")
    }
}

fn offline_command(binary: &Path, working_directory: &Path) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(working_directory).env_clear();
    process_support::isolate_user_home(&mut command, working_directory);
    command
        .env("COLOSSUS_RELEASE_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_RELEASE_SIGNING_KEY", SIGNING_KEY);
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

#[test]
fn packaged_installer_places_a_standalone_binary_that_completes_an_offline_echo_run() {
    let source_binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let root = fs::canonicalize(directory.path()).expect("canonical test root");
    #[cfg(unix)]
    create_private_directory(&root);
    let package = root.join("package");
    let prefix = root.join("prefix");
    let smoke = root.join("smoke");
    fs::create_dir_all(&package).expect("package directory");
    fs::create_dir_all(smoke.join("workflows")).expect("smoke workflows");
    #[cfg(unix)]
    for path in [&prefix, &prefix.join("home"), &prefix.join("bin")] {
        create_private_directory(path);
    }
    let installer = prepare_package(source_binary, &package);

    let mut installation_stdout = Vec::new();
    for _ in 0..2 {
        let installed = install(&installer, &prefix);
        assert!(
            installed.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&installed.stdout),
            String::from_utf8_lossy(&installed.stderr)
        );
        installation_stdout = installed.stdout;
    }
    let binary = installed_binary(&prefix);
    assert!(binary.is_file());
    let colossus_home = prefix.join("home/.colossus");
    if colossus_home.is_dir() {
        assert_eq!(
            fs::read_dir(&colossus_home)
                .expect("empty installer-created Colossus home")
                .count(),
            0,
            "the installer must not generate configuration or database files"
        );
    } else {
        assert!(
            String::from_utf8_lossy(&installation_stdout)
                .contains("deferred Colossus home creation")
        );
    }
    #[cfg(unix)]
    if colossus_home.is_dir() {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&colossus_home)
                .expect("Colossus home metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        // Make only the installer's first `id -u` report a privileged package
        // context. Later ownership checks use the real identity, so this exercises
        // deterministic deferral without requiring the test runner itself to be root.
        let fake_bin = root.join("privileged-id-bin");
        fs::create_dir(&fake_bin).expect("fake command directory");
        let marker = root.join("privileged-id.marker");
        let id = fake_bin.join("id");
        fs::write(
            &id,
            "#!/bin/sh\nif [ \"${1:-}\" = -u ] && [ ! -e \"$COLOSSUS_TEST_PRIVILEGED_MARKER\" ]; then\n  : > \"$COLOSSUS_TEST_PRIVILEGED_MARKER\"\n  printf '0\\n'\n  exit 0\nfi\nexec /usr/bin/id \"$@\"\n",
        )
        .expect("fake id command");
        fs::set_permissions(&id, fs::Permissions::from_mode(0o755)).expect("fake id permissions");
        let mut search_path = vec![fake_bin.clone()];
        search_path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        ));
        let deferred_home = root.join("privileged-home/.colossus");
        let deferred_prefix = root.join("privileged-prefix");
        create_private_directory(&deferred_prefix);
        create_private_directory(&deferred_prefix.join("bin"));
        let deferred = Command::new("/bin/sh")
            .arg(&installer)
            .arg("--prefix")
            .arg(&deferred_prefix)
            .env(
                "PATH",
                std::env::join_paths(search_path).expect("test PATH"),
            )
            .env("COLOSSUS_TEST_PRIVILEGED_MARKER", &marker)
            .env("COLOSSUS_HOME", &deferred_home)
            .env("XDG_DATA_HOME", root.join("privileged-data"))
            .output()
            .expect("run simulated privileged installer");
        assert!(
            deferred.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&deferred.stdout),
            String::from_utf8_lossy(&deferred.stderr)
        );
        assert!(!deferred_home.exists());
        assert!(
            String::from_utf8_lossy(&deferred.stdout).contains("deferred Colossus home creation")
        );
    }
    let receipt = if cfg!(windows) {
        prefix.join("data/Colossus/install.json")
    } else {
        prefix.join("data/colossus/install.json")
    };
    let receipt: Value =
        serde_json::from_slice(&fs::read(receipt).expect("install receipt")).expect("receipt JSON");
    assert_eq!(receipt["schemaVersion"], 1);
    assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["target"], release_target());
    assert_eq!(receipt["installerKind"], "direct");
    assert_eq!(
        fs::metadata(&binary).expect("installed metadata").len(),
        fs::metadata(source_binary).expect("source metadata").len()
    );

    fs::write(
        smoke.join("config.yaml"),
        include_str!("../../../release/smoke-config.yaml"),
    )
    .expect("smoke config");

    let version = offline_command(&binary, &smoke)
        .arg("--version")
        .output()
        .expect("version");
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("colossus "));

    let mut update = offline_command(&binary, &smoke);
    #[cfg(unix)]
    update.env("XDG_DATA_HOME", prefix.join("data"));
    #[cfg(windows)]
    update.env("LOCALAPPDATA", prefix.join("data"));
    let update = update
        .args([
            "--output",
            "json",
            "update",
            "--version",
            &format!("v{}", env!("CARGO_PKG_VERSION")),
        ])
        .output()
        .expect("same-version direct update");
    let update_succeeded = update.status.success();
    let update: Value = serde_json::from_slice(&update.stdout).expect("update JSON");
    if release_channel() == "preview" {
        assert!(!update_succeeded);
        assert_eq!(update["status"], "refused");
        assert_eq!(update["refusalReason"], "preview_installation");
    } else {
        assert!(update_succeeded, "stable same-version update must succeed");
        assert_eq!(update["status"], "up_to_date");
    }
    assert_eq!(update["installerKind"], "direct");

    let config = offline_command(&binary, &smoke)
        .args(["--config", "config.yaml", "config", "show"])
        .output()
        .expect("config show");
    assert!(
        config.status.success(),
        "{}",
        String::from_utf8_lossy(&config.stderr)
    );
    let config_text = String::from_utf8_lossy(&config.stdout);
    assert!(config_text.contains("kind: echo"));
    assert!(config_text.contains("networkDestinations: []"));
    assert!(!config_text.contains("credentialReference: env:"));

    let run = offline_command(&binary, &smoke)
        .args(["--config", "config.yaml", "run", "installed-offline"])
        .output()
        .expect("offline run");
    assert!(
        run.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let result: Value = serde_json::from_slice(&run.stdout).expect("run JSON");
    assert_eq!(result["output"], "installed-offline");
    assert_eq!(result["profile"], "echo");
    assert!(
        result["event_count"]
            .as_u64()
            .is_some_and(|count| count >= 3)
    );

    let audit = offline_command(&binary, &smoke)
        .args(["--config", "config.yaml", "audit", "verify"])
        .output()
        .expect("audit verify");
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let audit: Value = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    assert!(
        audit["last_sequence"]
            .as_u64()
            .is_some_and(|value| value >= 1)
    );
    assert_eq!(
        audit["checkpoint"]["global_sequence"],
        audit["last_sequence"]
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let shared_home = root.join("shared-home");
        fs::create_dir(&shared_home).expect("shared Colossus home");
        fs::set_permissions(&shared_home, fs::Permissions::from_mode(0o755))
            .expect("shared home permissions");
        let rejected = Command::new("/bin/sh")
            .arg(&installer)
            .arg("--prefix")
            .arg(root.join("shared-home-prefix"))
            .env("COLOSSUS_HOME", &shared_home)
            .env("XDG_DATA_HOME", root.join("shared-home-data"))
            .output()
            .expect("run installer with shared home");
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("must not grant group or other access")
        );

        let unsafe_parent = root.join("unsafe-home-parent");
        fs::create_dir(&unsafe_parent).expect("unsafe home parent");
        fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777))
            .expect("unsafe parent permissions");
        let rejected = Command::new("/bin/sh")
            .arg(&installer)
            .arg("--prefix")
            .arg(root.join("unsafe-parent-prefix"))
            .env("COLOSSUS_HOME", unsafe_parent.join(".colossus"))
            .env("XDG_DATA_HOME", root.join("unsafe-parent-data"))
            .output()
            .expect("run installer below unsafe parent");
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("writable without sticky protection")
        );

        let real_home = root.join("real-home");
        fs::create_dir(&real_home).expect("real Colossus home");
        fs::set_permissions(&real_home, fs::Permissions::from_mode(0o700))
            .expect("private home permissions");
        let linked_home = root.join("linked-home");
        symlink(&real_home, &linked_home).expect("linked Colossus home");
        let rejected = Command::new("/bin/sh")
            .arg(&installer)
            .arg("--prefix")
            .arg(root.join("linked-home-prefix"))
            .env("COLOSSUS_HOME", &linked_home)
            .env("XDG_DATA_HOME", root.join("linked-home-data"))
            .output()
            .expect("run installer with linked home");
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("refusing to install through linked path component")
        );

        let linked_prefix = root.join("linked-prefix");
        let actual_bin = root.join("actual-bin");
        create_private_directory(&linked_prefix);
        create_private_directory(&linked_prefix.join("home"));
        fs::create_dir(&actual_bin).expect("actual bin");
        symlink(&actual_bin, linked_prefix.join("bin")).expect("linked bin directory");
        let rejected = install(&installer, &linked_prefix);
        assert!(!rejected.status.success());
        let rejected_stderr = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            rejected_stderr.contains("refusing to install through linked path component"),
            "{rejected_stderr}"
        );

        let real_binary = package.join("colossus.real");
        fs::rename(package.join("colossus"), &real_binary).expect("rename package binary");
        symlink(&real_binary, package.join("colossus")).expect("linked package binary");
        let rejected = install(&installer, &root.join("rejected-prefix"));
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("missing, linked, or not executable")
        );
    }
}
