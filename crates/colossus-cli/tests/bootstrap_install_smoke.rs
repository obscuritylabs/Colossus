//! Network-free acceptance for the public Unix bootstrap installer.

#![cfg(unix)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::{TempDir, tempdir};

fn release_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        host => panic!("unsupported Unix release test host: {host:?}"),
    }
}

fn release_channel() -> &'static str {
    if env!("CARGO_PKG_VERSION").contains("-preview.") {
        "preview"
    } else {
        "stable"
    }
}

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    bin: PathBuf,
    package: String,
    package_root: PathBuf,
    archive: PathBuf,
    checksum: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempdir().expect("fixture directory");
        let root = fs::canonicalize(directory.path()).expect("canonical fixture root");
        let bin = root.join("bin");
        let assets = root.join("assets");
        fs::create_dir_all(&bin).expect("fake command directory");
        fs::create_dir_all(&assets).expect("asset directory");

        let version = env!("CARGO_PKG_VERSION");
        let target = release_target();
        let package = format!("colossus-{version}-{target}");
        let stage = root.join(&package);
        fs::create_dir(&stage).expect("package stage");
        let packaged_binary = stage.join("colossus");
        fs::copy(env!("CARGO_BIN_EXE_colossus"), &packaged_binary).expect("package binary");
        fs::set_permissions(&packaged_binary, fs::Permissions::from_mode(0o755))
            .expect("binary permissions");
        for (source, destination, mode) in [
            ("release/install.sh", "install.sh", 0o755),
            ("LICENSE", "LICENSE", 0o644),
            ("README.md", "README.md", 0o644),
        ] {
            fs::copy(repository_root().join(source), stage.join(destination))
                .unwrap_or_else(|error| panic!("copy {source}: {error}"));
            fs::set_permissions(stage.join(destination), fs::Permissions::from_mode(mode))
                .expect("package file permissions");
        }
        if std::env::consts::OS == "linux" {
            for (source, destination, mode) in [
                ("release/install-apparmor.sh", "install-apparmor.sh", 0o755),
                (
                    "release/colossus.apparmor.in",
                    "colossus.apparmor.in",
                    0o644,
                ),
            ] {
                fs::copy(repository_root().join(source), stage.join(destination))
                    .unwrap_or_else(|error| panic!("copy {source}: {error}"));
                fs::set_permissions(stage.join(destination), fs::Permissions::from_mode(mode))
                    .expect("Linux package file permissions");
            }
        }
        fs::write(
            stage.join("install-metadata"),
            format!(
                "schema_version=1\nversion={version}\ntarget={target}\nchannel={}\ndistribution_origin=https://github.com/obscuritylabs/Colossus/releases\ninstaller_kind=direct\n",
                release_channel()
            ),
        )
        .expect("install metadata");

        let archive_name = format!("{package}.tar.gz");
        let archive = assets.join(&archive_name);
        let status = Command::new("tar")
            .args(["-C"])
            .arg(&root)
            .args(["-czf"])
            .arg(&archive)
            .arg(&package)
            .status()
            .expect("create fixture archive");
        assert!(status.success());
        let digest = hex::encode(Sha256::digest(fs::read(&archive).expect("archive bytes")));
        let checksum = assets.join(format!("{archive_name}.sha256"));
        fs::write(&checksum, format!("{digest}  {archive_name}\n")).expect("checksum sidecar");

        let targets = [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
            "aarch64-pc-windows-msvc",
            "x86_64-pc-windows-msvc",
        ];
        let mut asset_names = Vec::new();
        for release_target in targets {
            let extension = if release_target.ends_with("windows-msvc") {
                "zip"
            } else {
                "tar.gz"
            };
            let name = format!("colossus-{version}-{release_target}.{extension}");
            asset_names.push(json!({"name": name}));
            asset_names.push(json!({"name": format!("{name}.sha256")}));
        }
        let release = json!({
            "tag_name": format!("v{version}"),
            "draft": false,
            "prerelease": release_channel() == "preview",
            "assets": asset_names,
        });
        fs::write(
            root.join("release.json"),
            serde_json::to_vec_pretty(&release).expect("release JSON"),
        )
        .expect("release fixture");

        let fake_curl = bin.join("curl");
        fs::write(
            &fake_curl,
            r#"#!/bin/sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output)
            output=$2
            shift 2
            ;;
        --proto|--proto-redir|--noproxy|--max-redirs|--connect-timeout|--max-time|--max-filesize|--header|--user-agent|--write-out)
            shift 2
            ;;
        -fsS|--location)
            shift
            ;;
        https://*)
            url=$1
            shift
            ;;
        *)
            exit 64
            ;;
    esac
done
[ -n "$output" ] && [ -n "$url" ]
case "$url" in
    https://api.github.com/*)
        source=$COLOSSUS_BOOTSTRAP_FIXTURES/release.json
        ;;
    *.sha256)
        source=$COLOSSUS_BOOTSTRAP_FIXTURES/assets/$(basename "$url")
        ;;
    *.tar.gz)
        source=$COLOSSUS_BOOTSTRAP_FIXTURES/assets/$(basename "$url")
        ;;
    *)
        exit 65
        ;;
esac
cp "$source" "$output"
printf '%s' "$url"
"#,
        )
        .expect("fake curl");
        fs::set_permissions(&fake_curl, fs::Permissions::from_mode(0o755))
            .expect("fake curl permissions");

        let fake_uname = bin.join("uname");
        fs::write(
            &fake_uname,
            r#"#!/bin/sh
case "$1" in
    -s) printf '%s\n' "$COLOSSUS_TEST_KERNEL" ;;
    -m) printf '%s\n' "$COLOSSUS_TEST_MACHINE" ;;
    *) exit 64 ;;
esac
"#,
        )
        .expect("fake uname");
        fs::set_permissions(&fake_uname, fs::Permissions::from_mode(0o755))
            .expect("fake uname permissions");

        // GNU stat can emit filesystem output for the valid path operand before
        // returning failure for the BSD-style format operand. Model that behavior so
        // the fallback cannot accidentally combine partial output with the GNU value.
        let fake_stat = bin.join("stat");
        fs::write(
            &fake_stat,
            r#"#!/bin/sh
set -eu
[ "$#" -eq 4 ] && [ "$3" = -- ] || exit 64
case "$1:$2" in
    -f:%u|-f:%Lp)
        printf 'partial GNU filesystem status\n'
        exit 1
        ;;
    -c:%u)
        id -u
        ;;
    -c:%a)
        printf '700\n'
        ;;
    *)
        exit 64
        ;;
esac
"#,
        )
        .expect("fake stat");
        fs::set_permissions(&fake_stat, fs::Permissions::from_mode(0o755))
            .expect("fake stat permissions");

        Self {
            _directory: directory,
            root,
            bin,
            package,
            package_root: stage,
            archive,
            checksum,
        }
    }

    fn rebuild_archive(&self) {
        let status = Command::new("tar")
            .args(["-C"])
            .arg(&self.root)
            .args(["-czf"])
            .arg(&self.archive)
            .arg(&self.package)
            .status()
            .expect("rebuild fixture archive");
        assert!(status.success());
        self.rewrite_checksum();
    }

    fn rewrite_checksum(&self) {
        let digest = hex::encode(Sha256::digest(
            fs::read(&self.archive).expect("updated archive bytes"),
        ));
        fs::write(
            &self.checksum,
            format!(
                "{digest}  {}\n",
                self.archive.file_name().unwrap().to_string_lossy()
            ),
        )
        .expect("updated checksum sidecar");
    }

    fn run(&self, arguments: &[&str], prefix: &Path) -> Output {
        let original_path = std::env::var("PATH").expect("PATH");
        Command::new("/bin/sh")
            .arg(repository_root().join("release/bootstrap/install.sh"))
            .args(arguments)
            .env("PATH", format!("{}:{original_path}", self.bin.display()))
            .env("COLOSSUS_BOOTSTRAP_FIXTURES", &self.root)
            .env("COLOSSUS_TEST_KERNEL", kernel_name())
            .env("COLOSSUS_TEST_MACHINE", machine_name())
            .env("XDG_DATA_HOME", prefix.join("data"))
            .output()
            .expect("run bootstrap installer")
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn kernel_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Darwin",
        "linux" => "Linux",
        os => panic!("unsupported Unix release test OS: {os}"),
    }
}

fn machine_name() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        architecture => panic!("unsupported Unix release test architecture: {architecture}"),
    }
}

#[test]
fn bootstrap_maps_supported_unix_hosts_and_honors_dry_run_flags() {
    let fixture = Fixture::new();
    let prefix = fixture.root.join("dry-run-prefix");
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    for (kernel, machine, expected) in [
        ("Darwin", "arm64", "aarch64-apple-darwin"),
        ("Darwin", "x86_64", "x86_64-apple-darwin"),
        ("Linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("Linux", "x86_64", "x86_64-unknown-linux-musl"),
    ] {
        let original_path = std::env::var("PATH").expect("PATH");
        let output = Command::new("/bin/sh")
            .arg(repository_root().join("release/bootstrap/install.sh"))
            .args([
                "--version",
                &version,
                "--channel",
                release_channel(),
                "--prefix",
            ])
            .arg(&prefix)
            .args(["--dry-run", "--no-modify-path", "--yes"])
            .env("PATH", format!("{}:{original_path}", fixture.bin.display()))
            .env("COLOSSUS_BOOTSTRAP_FIXTURES", &fixture.root)
            .env("COLOSSUS_TEST_KERNEL", kernel)
            .env("COLOSSUS_TEST_MACHINE", machine)
            .output()
            .expect("run dry-run bootstrap");
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(&format!("target: {expected}")), "{stdout}");
        assert!(stdout.contains("dry run complete"), "{stdout}");
    }
    assert!(!prefix.exists());
}

#[test]
fn bootstrap_accepts_compact_github_release_json() {
    let fixture = Fixture::new();
    let release_path = fixture.root.join("release.json");
    let release: Value =
        serde_json::from_slice(&fs::read(&release_path).expect("read pretty release fixture"))
            .expect("release fixture JSON");
    fs::write(
        &release_path,
        serde_json::to_vec(&release).expect("compact release JSON"),
    )
    .expect("write compact release fixture");

    let prefix = fixture.root.join("compact-json-prefix");
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let output = fixture.run(
        &[
            "--version",
            &version,
            "--channel",
            release_channel(),
            "--prefix",
            prefix.to_str().expect("UTF-8 prefix"),
            "--dry-run",
            "--no-modify-path",
            "--yes",
        ],
        &prefix,
    );
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("dry run complete"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!prefix.exists());
}

#[test]
fn bootstrap_verifies_then_installs_and_records_direct_ownership() {
    let fixture = Fixture::new();
    let prefix = fixture.root.join("prefix");
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let output = fixture.run(
        &[
            "--version",
            &version,
            "--channel",
            release_channel(),
            "--prefix",
            prefix.to_str().expect("UTF-8 prefix"),
            "--yes",
        ],
        &prefix,
    );
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(prefix.join("bin/colossus").is_file());
    let receipt: Value = serde_json::from_slice(
        &fs::read(prefix.join("data/colossus/install.json")).expect("install receipt"),
    )
    .expect("receipt JSON");
    assert_eq!(receipt["channel"], release_channel());
    assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["target"], release_target());
    assert_eq!(receipt["installerKind"], "direct");
}

#[test]
fn bootstrap_fails_closed_on_checksum_mismatch_and_unsupported_hosts() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.checksum,
        format!(
            "{}  {}\n",
            "0".repeat(64),
            fixture.archive.file_name().unwrap().to_string_lossy()
        ),
    )
    .expect("corrupt checksum");
    let prefix = fixture.root.join("rejected-prefix");
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let rejected = fixture.run(
        &[
            "--version",
            &version,
            "--channel",
            release_channel(),
            "--prefix",
            prefix.to_str().expect("UTF-8 prefix"),
            "--yes",
        ],
        &prefix,
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("archive checksum mismatch"));
    assert!(!prefix.join("bin/colossus").exists());

    let original_path = std::env::var("PATH").expect("PATH");
    let unsupported = Command::new("/bin/sh")
        .arg(repository_root().join("release/bootstrap/install.sh"))
        .args(["--dry-run", "--yes"])
        .env("PATH", format!("{}:{original_path}", fixture.bin.display()))
        .env("COLOSSUS_BOOTSTRAP_FIXTURES", &fixture.root)
        .env("COLOSSUS_TEST_KERNEL", "FreeBSD")
        .env("COLOSSUS_TEST_MACHINE", "x86_64")
        .output()
        .expect("run unsupported host bootstrap");
    assert!(!unsupported.status.success());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("unsupported host"));
}

#[test]
fn bootstrap_rejects_truncated_and_unexpected_archives_with_matching_checksums() {
    let truncated = Fixture::new();
    let original_length = fs::metadata(&truncated.archive)
        .expect("archive metadata")
        .len();
    fs::OpenOptions::new()
        .write(true)
        .open(&truncated.archive)
        .expect("open archive for truncation")
        .set_len(original_length / 2)
        .expect("truncate archive");
    truncated.rewrite_checksum();
    let truncated_prefix = truncated.root.join("truncated-prefix");
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let rejected = truncated.run(
        &[
            "--version",
            &version,
            "--channel",
            release_channel(),
            "--prefix",
            truncated_prefix.to_str().expect("UTF-8 prefix"),
            "--yes",
        ],
        &truncated_prefix,
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("archive table of contents"));
    assert!(!truncated_prefix.join("bin/colossus").exists());

    let unexpected = Fixture::new();
    fs::write(unexpected.package_root.join("unexpected"), b"not reviewed")
        .expect("unexpected archive member");
    unexpected.rebuild_archive();
    let unexpected_prefix = unexpected.root.join("unexpected-prefix");
    let rejected = unexpected.run(
        &[
            "--version",
            &version,
            "--channel",
            release_channel(),
            "--prefix",
            unexpected_prefix.to_str().expect("UTF-8 prefix"),
            "--yes",
        ],
        &unexpected_prefix,
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("archive layout contains missing or unexpected paths")
    );
    assert!(!unexpected_prefix.join("bin/colossus").exists());
}

#[test]
fn bootstrap_rejects_linked_executables_even_with_a_matching_checksum() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    fs::remove_file(fixture.package_root.join("colossus")).expect("remove packaged executable");
    symlink("/bin/false", fixture.package_root.join("colossus"))
        .expect("linked packaged executable");
    fixture.rebuild_archive();
    let prefix = fixture.root.join("linked-prefix");
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let rejected = fixture.run(
        &[
            "--version",
            &version,
            "--channel",
            release_channel(),
            "--prefix",
            prefix.to_str().expect("UTF-8 prefix"),
            "--yes",
        ],
        &prefix,
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("archive contains a link or special file")
    );
    assert!(!prefix.join("bin/colossus").exists());
}
