use colossus_sdk::{Sha256Digest, VerifiedExecutable};
use serde::Deserialize;
#[cfg(any(test, not(debug_assertions)))]
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

use crate::dto::CommandErrorDto;

const COMPILED_BUNDLE_MANIFEST: &str =
    include_str!(concat!(env!("OUT_DIR"), "/bundle-manifest.json"));
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
#[cfg(any(test, not(debug_assertions)))]
const RELEASE_MANIFEST_BINDING_PREFIX: &str = "COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256_V1=";
#[cfg(any(test, not(debug_assertions)))]
const RELEASE_MANIFEST_BINDING_SUFFIX: &str = ":END_COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256";
#[cfg(not(debug_assertions))]
#[used]
static RELEASE_MANIFEST_BINDING: &str = concat!(
    "COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256_V1=",
    "0000000000000000000000000000000000000000000000000000000000000000",
    ":END_COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256"
);
#[cfg(not(debug_assertions))]
const SEALED_RELEASE_MANIFEST: &str = "colossus-bundle-manifest.json";
#[cfg(not(debug_assertions))]
const EXPECTED_RELEASE_TEAM_ID: &str = env!("COLOSSUS_DESKTOP_TEAM_ID");
#[cfg(not(debug_assertions))]
const DESKTOP_CODE_IDENTIFIER: &str = "com.obscuritylabs.colossus.desktop";
#[cfg(not(debug_assertions))]
const SIDECAR_CODE_IDENTIFIER: &str = "com.obscuritylabs.colossus.desktop.sidecar";
#[cfg(not(debug_assertions))]
const CLI_CODE_IDENTIFIER: &str = "com.obscuritylabs.colossus.desktop.cli";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleManifest {
    schema_version: u16,
    target_triple: String,
    profile: BundleProfile,
    sidecar: BundledExecutable,
    cli: BundledExecutable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum BundleProfile {
    Debug,
    Release,
    UnsealedRelease,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundledExecutable {
    file_name: String,
    sha256: String,
    #[serde(default)]
    development_path: Option<PathBuf>,
}

pub(crate) struct VerifiedBundle {
    pub(crate) sidecar: VerifiedExecutable,
    pub(crate) cli_path: PathBuf,
    pub(crate) cli_sha256: [u8; 32],
}

impl VerifiedBundle {
    pub(crate) fn load() -> Result<Self, CommandErrorDto> {
        let compiled = decode_manifest(COMPILED_BUNDLE_MANIFEST.as_bytes())?;
        #[cfg(debug_assertions)]
        {
            if compiled.profile != BundleProfile::Debug {
                return Err(integrity_error());
            }
            verified_manifest(&compiled, release_directory().as_deref())
        }
        #[cfg(not(debug_assertions))]
        {
            if compiled.profile != BundleProfile::UnsealedRelease {
                return Err(integrity_error());
            }
            let app_root = release_app_root().ok_or_else(integrity_error)?;
            verify_outer_app_signature(&app_root)?;
            let manifest_path = app_root
                .join("Contents")
                .join("Resources")
                .join(SEALED_RELEASE_MANIFEST);
            let source = read_bounded_manifest(&manifest_path)?;
            verify_release_manifest_binding(
                &source,
                std::hint::black_box(&RELEASE_MANIFEST_BINDING).as_bytes(),
            )?;
            let release = decode_manifest(&source)?;
            if release.profile != BundleProfile::Release {
                return Err(integrity_error());
            }
            verified_manifest(&release, release_directory().as_deref())
        }
    }
}

fn verified_manifest(
    manifest: &BundleManifest,
    release_directory: Option<&Path>,
) -> Result<VerifiedBundle, CommandErrorDto> {
    if manifest.schema_version != 1
        || manifest.target_triple != env!("COLOSSUS_DESKTOP_TARGET_TRIPLE")
    {
        return Err(integrity_error());
    }
    #[cfg(debug_assertions)]
    if manifest.profile != BundleProfile::Debug {
        return Err(integrity_error());
    }
    #[cfg(not(debug_assertions))]
    if manifest.profile != BundleProfile::Release {
        return Err(integrity_error());
    }

    let sidecar = resolve_executable(&manifest.sidecar, manifest.profile, release_directory)?;
    let cli = resolve_executable(&manifest.cli, manifest.profile, release_directory)?;
    let sidecar_digest =
        Sha256Digest::from_hex(&manifest.sidecar.sha256).map_err(|_| integrity_error())?;
    let cli_digest = Sha256Digest::from_hex(&manifest.cli.sha256).map_err(|_| integrity_error())?;
    let sidecar =
        VerifiedExecutable::new(sidecar, sidecar_digest).map_err(|_| integrity_error())?;

    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    {
        verify_release_code_identity(sidecar.path(), SIDECAR_CODE_IDENTIFIER)?;
        verify_release_code_identity(&cli, CLI_CODE_IDENTIFIER)?;
    }
    Ok(VerifiedBundle {
        sidecar,
        cli_path: cli,
        cli_sha256: *cli_digest.as_bytes(),
    })
}

fn decode_manifest(source: &[u8]) -> Result<BundleManifest, CommandErrorDto> {
    if source.is_empty() || source.len() > MAX_MANIFEST_BYTES {
        return Err(integrity_error());
    }
    serde_json::from_slice(source).map_err(|_| integrity_error())
}

#[cfg(any(test, not(debug_assertions)))]
fn verify_release_manifest_binding(source: &[u8], binding: &[u8]) -> Result<(), CommandErrorDto> {
    let digest = binding
        .strip_prefix(RELEASE_MANIFEST_BINDING_PREFIX.as_bytes())
        .and_then(|value| value.strip_suffix(RELEASE_MANIFEST_BINDING_SUFFIX.as_bytes()))
        .filter(|value| value.len() == 64)
        .ok_or_else(integrity_error)?;
    if digest.iter().all(|byte| *byte == b'0')
        || !digest
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(integrity_error());
    }
    let actual: [u8; 32] = Sha256::digest(source).into();
    let expected =
        Sha256Digest::from_hex(std::str::from_utf8(digest).map_err(|_| integrity_error())?)
            .map_err(|_| integrity_error())?;
    if actual == *expected.as_bytes() {
        Ok(())
    } else {
        Err(integrity_error())
    }
}

fn resolve_executable(
    executable: &BundledExecutable,
    profile: BundleProfile,
    release_directory: Option<&Path>,
) -> Result<PathBuf, CommandErrorDto> {
    if !valid_file_name(&executable.file_name)
        || executable.sha256.len() != 64
        || !executable
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(integrity_error());
    }
    match profile {
        BundleProfile::Debug => executable
            .development_path
            .as_ref()
            .filter(|path| path.is_absolute() && path.file_name().is_some())
            .cloned()
            .ok_or_else(integrity_error),
        BundleProfile::Release => {
            if executable.development_path.is_some() {
                return Err(integrity_error());
            }
            release_directory
                .map(|directory| directory.join(&executable.file_name))
                .ok_or_else(integrity_error)
        }
        BundleProfile::UnsealedRelease => Err(integrity_error()),
    }
}

fn valid_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && Path::new(value)
            .file_name()
            .is_some_and(|name| name == value)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn release_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_owned))
}

#[cfg(not(debug_assertions))]
fn release_app_root() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension().is_some_and(|extension| extension == "app"))
    .then(|| app.to_owned())
}

#[cfg(all(not(debug_assertions), target_family = "unix"))]
fn read_bounded_manifest(path: &Path) -> Result<Vec<u8>, CommandErrorDto> {
    use std::io::Read as _;

    let opened = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| integrity_error())?;
    let file = std::fs::File::from(opened);
    let metadata = file.metadata().map_err(|_| integrity_error())?;
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > u64::try_from(MAX_MANIFEST_BYTES).unwrap_or(u64::MAX)
    {
        return Err(integrity_error());
    }
    let mut source = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(u64::try_from(MAX_MANIFEST_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut source)
        .map_err(|_| integrity_error())?;
    if source.is_empty() || source.len() > MAX_MANIFEST_BYTES {
        return Err(integrity_error());
    }
    Ok(source)
}

#[cfg(all(not(debug_assertions), not(target_family = "unix")))]
fn read_bounded_manifest(_path: &Path) -> Result<Vec<u8>, CommandErrorDto> {
    Err(integrity_error())
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
fn verify_outer_app_signature(app_root: &Path) -> Result<(), CommandErrorDto> {
    let expected_team = expected_release_team_id()?;
    verify_signed_code_identity(app_root, DESKTOP_CODE_IDENTIFIER, expected_team)?;
    let main = std::env::current_exe().map_err(|_| integrity_error())?;
    verify_signed_code_identity(&main, DESKTOP_CODE_IDENTIFIER, expected_team)?;
    let status = std::process::Command::new("/usr/bin/codesign")
        .env_clear()
        .args(["--verify", "--strict", "--deep", "--verbose=0"])
        .arg(app_root)
        .status()
        .map_err(|_| integrity_error())?;
    if status.success() {
        Ok(())
    } else {
        Err(integrity_error())
    }
}

#[cfg(all(not(target_os = "macos"), not(debug_assertions)))]
fn verify_outer_app_signature(_app_root: &Path) -> Result<(), CommandErrorDto> {
    Err(integrity_error())
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
fn verify_release_code_identity(
    path: &Path,
    expected_identifier: &str,
) -> Result<(), CommandErrorDto> {
    let expected_team = expected_release_team_id()?;
    verify_signed_code_identity(path, expected_identifier, expected_team)?;
    let parent = std::env::current_exe().map_err(|_| integrity_error())?;
    verify_signed_code_identity(&parent, DESKTOP_CODE_IDENTIFIER, expected_team)
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
fn verify_signed_code_identity(
    path: &Path,
    expected_identifier: &str,
    expected_team: &str,
) -> Result<(), CommandErrorDto> {
    let status = std::process::Command::new("/usr/bin/codesign")
        .env_clear()
        .args(["--verify", "--strict", "--verbose=0"])
        .arg(path)
        .status()
        .map_err(|_| integrity_error())?;
    if !status.success() {
        return Err(integrity_error());
    }
    let output = std::process::Command::new("/usr/bin/codesign")
        .env_clear()
        .args(["-d", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|_| integrity_error())?;
    if !output.status.success() || output.stderr.is_empty() || output.stderr.len() > 16 * 1024 {
        return Err(integrity_error());
    }
    let details = std::str::from_utf8(&output.stderr).map_err(|_| integrity_error())?;
    let mut identifiers = details
        .lines()
        .filter_map(|line| line.strip_prefix("Identifier="));
    let identifier = identifiers.next().ok_or_else(integrity_error)?;
    let mut teams = details
        .lines()
        .filter_map(|line| line.strip_prefix("TeamIdentifier="));
    let team = teams.next().ok_or_else(integrity_error)?;
    if identifiers.next().is_some()
        || teams.next().is_some()
        || identifier != expected_identifier
        || team != expected_team
    {
        return Err(integrity_error());
    }
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(debug_assertions)))]
fn verify_release_code_identity(
    _path: &Path,
    _expected_identifier: &str,
) -> Result<(), CommandErrorDto> {
    Err(integrity_error())
}

#[cfg(not(debug_assertions))]
fn expected_release_team_id() -> Result<&'static str, CommandErrorDto> {
    if canonical_apple_team_id(EXPECTED_RELEASE_TEAM_ID) {
        Ok(EXPECTED_RELEASE_TEAM_ID)
    } else {
        // ADHOC is a packaging-only validation sentinel. The runtime must never
        // accept a bundle compiled without a real expected Apple TeamIdentifier.
        Err(integrity_error())
    }
}

#[cfg(any(test, not(debug_assertions)))]
fn canonical_apple_team_id(value: &str) -> bool {
    value.len() == 10
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn integrity_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "runtime_integrity",
        "The bundled Colossus runtime could not be verified.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_binding(digest: &str) -> Vec<u8> {
        format!("{RELEASE_MANIFEST_BINDING_PREFIX}{digest}{RELEASE_MANIFEST_BINDING_SUFFIX}")
            .into_bytes()
    }

    #[test]
    fn rejects_paths_and_noncanonical_digests() {
        assert!(!valid_file_name("../colossus"));
        assert!(!valid_file_name("folder/colossus"));
        assert!(!valid_file_name("colossus sidecar"));
        assert!(valid_file_name("colossus-sidecar"));
        assert!(Sha256Digest::from_hex(&"a".repeat(64)).is_ok());
        assert!(Sha256Digest::from_hex(&"g".repeat(64)).is_err());
        assert!(canonical_apple_team_id("A1B2C3D4E5"));
        assert!(!canonical_apple_team_id("ADHOC"));
        assert!(!canonical_apple_team_id("a1b2c3d4e5"));
    }

    #[test]
    fn rejects_unset_release_manifest_binding() {
        let binding = release_binding(&"0".repeat(64));
        assert!(verify_release_manifest_binding(b"manifest", &binding).is_err());
    }

    #[test]
    fn rejects_mismatched_release_manifest_binding() {
        let binding = release_binding(&"a".repeat(64));
        assert!(verify_release_manifest_binding(b"manifest", &binding).is_err());
    }

    #[test]
    fn accepts_exact_release_manifest_binding() {
        let source = b"exact manifest bytes\n";
        let binding = release_binding(&hex::encode(Sha256::digest(source)));
        assert!(verify_release_manifest_binding(source, &binding).is_ok());
        assert!(verify_release_manifest_binding(b"exact manifest bytes", &binding).is_err());
    }
}
