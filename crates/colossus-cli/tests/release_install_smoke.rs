//! Offline installation acceptance for packaged native Rust executables.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7171717171717171717171717171717171717171717171717171717171717171";
const SIGNING_KEY: &str = "8282828282828282828282828282828282828282828282828282828282828282";

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

fn write_install_metadata(package: &Path) {
    fs::write(
        package.join("install-metadata"),
        format!(
            "schema_version=1\nversion={}\ntarget={}\nchannel=stable\ndistribution_origin=https://github.com/obscuritylabs/Colossus/releases\ninstaller_kind=direct\n",
            env!("CARGO_PKG_VERSION"),
            release_target()
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
    command
        .current_dir(working_directory)
        .env_clear()
        .env("HOME", working_directory)
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
    let package = root.join("package");
    let prefix = root.join("prefix");
    let smoke = root.join("smoke");
    fs::create_dir_all(&package).expect("package directory");
    fs::create_dir_all(smoke.join("workflows")).expect("smoke workflows");
    let installer = prepare_package(source_binary, &package);

    for _ in 0..2 {
        let installed = install(&installer, &prefix);
        assert!(
            installed.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&installed.stdout),
            String::from_utf8_lossy(&installed.stderr)
        );
    }
    let binary = installed_binary(&prefix);
    assert!(binary.is_file());
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
    assert!(
        update.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&update.stdout),
        String::from_utf8_lossy(&update.stderr)
    );
    let update: Value = serde_json::from_slice(&update.stdout).expect("update JSON");
    assert_eq!(update["status"], "up_to_date");
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
        use std::os::unix::fs::symlink;

        let linked_prefix = root.join("linked-prefix");
        let actual_bin = root.join("actual-bin");
        fs::create_dir(&linked_prefix).expect("linked prefix");
        fs::create_dir(&actual_bin).expect("actual bin");
        symlink(&actual_bin, linked_prefix.join("bin")).expect("linked bin directory");
        let rejected = install(&installer, &linked_prefix);
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("refusing to install through linked path component")
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
