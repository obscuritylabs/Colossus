#[cfg(test)]
use colossus_sdk::SdkError;
use colossus_sdk::{
    ApiMajor, Colossus, DaemonConnectOptions, InstanceId, KeyringCredentialProvider, TlsFingerprint,
};
use directories::BaseDirs;
use serde::Deserialize;
use std::{
    fs,
    io::Read as _,
    path::{Component, Path, PathBuf},
    str::FromStr as _,
    sync::Arc,
};
use uuid::Uuid;

#[cfg(not(windows))]
use std::fs::File;

use crate::{
    desktop_settings::{
        EXTERNAL_KEYRING_SERVICE, ExternalTargetSetting, external_credential_account,
        valid_external_label,
    },
    dto::CommandErrorDto,
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;

const COMPILED_CONNECTION: &str = include_str!(concat!(env!("OUT_DIR"), "/connection.json"));
#[cfg(test)]
const TEMPLATE_CONNECTION: &str = include_str!("../connection.json");
const ENDPOINT_FILE: &str = "endpoint.json";
const CERTIFICATE_FILE: &str = "certificate.pem";
const MAX_CONNECTION_FILE_BYTES: u64 = 16 * 1024;
const PLACEHOLDER_FINGERPRINT: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const LEGACY_DESKTOP_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.desktop";
const LEGACY_DESKTOP_KEYRING_ACCOUNT: &str = "colossus-public-api";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledConnectionConfig {
    #[serde(default)]
    label: Option<String>,
    instance_id: String,
    certificate_sha256: String,
    public_api_dir: String,
    credential_service: String,
    credential_account: String,
}

struct PreparedConnectionConfig {
    instance_id: InstanceId,
    certificate_sha256: TlsFingerprint,
    public_api_dir: PathBuf,
    credential_service: String,
    credential_account: String,
}

struct PreparedConnectionIdentity {
    instance_id: InstanceId,
    certificate_sha256: TlsFingerprint,
    public_api_dir: PathBuf,
}

pub(crate) fn compiled_target() -> Option<ExternalTargetSetting> {
    let home = home_dir();
    target_from_source(COMPILED_CONNECTION, "external-default", home.as_deref())
        .or_else(|_| {
            legacy_compiled_target_from_source(
                COMPILED_CONNECTION,
                "external-default",
                home.as_deref(),
            )
        })
        .ok()
}

pub(crate) fn import_target(path: &Path) -> Result<ExternalTargetSetting, CommandErrorDto> {
    let metadata = fs::symlink_metadata(path).map_err(|_| connection_file_error())?;
    if !path.is_absolute()
        || path.parent().is_none()
        || !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_CONNECTION_FILE_BYTES
    {
        return Err(connection_file_error());
    }
    #[cfg(unix)]
    if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o022 != 0 {
        return Err(connection_file_error());
    }
    let canonical = fs::canonicalize(path).map_err(|_| connection_file_error())?;
    if canonical != path {
        return Err(connection_file_error());
    }
    #[cfg(windows)]
    let binding = colossus_windows_native::BoundPath::open_file(&canonical)
        .map_err(|_| connection_file_error())?;
    #[cfg(windows)]
    binding
        .validate_private_owner_dacl()
        .map_err(|_| connection_file_error())?;
    #[cfg(windows)]
    let file = binding
        .try_clone_file()
        .map_err(|_| connection_file_error())?;
    #[cfg(not(windows))]
    let file = File::open(&canonical).map_err(|_| connection_file_error())?;
    let opened_metadata = file.metadata().map_err(|_| connection_file_error())?;
    if !same_file_metadata(&metadata, &opened_metadata) {
        return Err(connection_file_error());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    (&file)
        .take(MAX_CONNECTION_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| connection_file_error())?;
    let final_metadata = file.metadata().map_err(|_| connection_file_error())?;
    if !same_file_metadata(&opened_metadata, &final_metadata) {
        return Err(connection_file_error());
    }
    #[cfg(windows)]
    binding.revalidate().map_err(|_| connection_file_error())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CONNECTION_FILE_BYTES {
        return Err(connection_file_error());
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| connection_file_error())?;
    let home = home_dir();
    target_from_source(source, &Uuid::now_v7().to_string(), home.as_deref())
        .map_err(|_| connection_file_error())
}

fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    if !left.file_type().is_file() || !right.file_type().is_file() || left.len() != right.len() {
        return false;
    }
    #[cfg(unix)]
    {
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.uid() == right.uid()
            && left.mode() == right.mode()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn same_connection(left: &ExternalTargetSetting, right: &ExternalTargetSetting) -> bool {
    left.instance_id == right.instance_id
        && left
            .certificate_sha256
            .eq_ignore_ascii_case(&right.certificate_sha256)
        && left.public_api_dir == right.public_api_dir
        && left.credential_service == right.credential_service
        && left.credential_account == right.credential_account
}

pub(crate) async fn connect(target: &ExternalTargetSetting) -> Result<Colossus, CommandErrorDto> {
    let prepared = prepare_saved_connection(target)?;
    let credential_provider =
        KeyringCredentialProvider::new(prepared.credential_service, prepared.credential_account)
            .map_err(CommandErrorDto::from_sdk)?;
    let options = DaemonConnectOptions::new(
        prepared.instance_id,
        prepared.public_api_dir.join(ENDPOINT_FILE),
        prepared.certificate_sha256,
        ApiMajor::new(1).map_err(CommandErrorDto::from_sdk)?,
        Arc::new(credential_provider),
    )
    .and_then(|options| {
        options.with_certificate_path(prepared.public_api_dir.join(CERTIFICATE_FILE))
    })
    .map_err(CommandErrorDto::from_sdk)?;

    Colossus::connect_installed(options)
        .await
        .map_err(CommandErrorDto::from_sdk)
}

fn target_from_source(
    source: &str,
    target_id: &str,
    home: Option<&Path>,
) -> Result<ExternalTargetSetting, CommandErrorDto> {
    let raw: CompiledConnectionConfig =
        serde_json::from_str(source).map_err(|_| CommandErrorDto::not_configured())?;
    let prepared = prepare_raw_connection(&raw, home)?;
    let label = raw.label.as_deref().map(str::trim).map_or_else(
        || format!("External {}", &raw.instance_id[..8]),
        str::to_owned,
    );
    if !valid_external_label(&label) {
        return Err(CommandErrorDto::not_configured());
    }
    Ok(ExternalTargetSetting {
        target_id: target_id.to_owned(),
        label,
        instance_id: raw.instance_id,
        certificate_sha256: raw.certificate_sha256.to_ascii_lowercase(),
        public_api_dir: prepared.public_api_dir,
        credential_service: prepared.credential_service,
        credential_account: prepared.credential_account,
        requires_credential_enrollment: false,
    })
}

fn legacy_compiled_target_from_source(
    source: &str,
    target_id: &str,
    home: Option<&Path>,
) -> Result<ExternalTargetSetting, CommandErrorDto> {
    let raw: CompiledConnectionConfig =
        serde_json::from_str(source).map_err(|_| CommandErrorDto::not_configured())?;
    let prepared = prepare_raw_identity(&raw, home)?;
    if raw.credential_service != LEGACY_DESKTOP_KEYRING_SERVICE
        || raw.credential_account != LEGACY_DESKTOP_KEYRING_ACCOUNT
    {
        return Err(CommandErrorDto::not_configured());
    }
    let label = raw.label.as_deref().map(str::trim).map_or_else(
        || format!("External {}", &raw.instance_id[..8]),
        str::to_owned,
    );
    if !valid_external_label(&label) {
        return Err(CommandErrorDto::not_configured());
    }
    let certificate_sha256 = raw.certificate_sha256.to_ascii_lowercase();
    let credential_account = external_credential_account(&raw.instance_id, &certificate_sha256)
        .ok_or_else(CommandErrorDto::not_configured)?;
    Ok(ExternalTargetSetting {
        target_id: target_id.to_owned(),
        label,
        instance_id: raw.instance_id,
        certificate_sha256,
        public_api_dir: prepared.public_api_dir,
        credential_service: EXTERNAL_KEYRING_SERVICE.to_owned(),
        credential_account,
        // Never read or copy the prior bearer. The retained trust anchors stay
        // visible, but connection fails closed until the user explicitly enrolls
        // the new identity-bound destination.
        requires_credential_enrollment: true,
    })
}

fn home_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.home_dir().to_owned())
}

#[cfg(test)]
fn prepare_connection(
    source: &str,
    home: Option<&Path>,
) -> Result<PreparedConnectionConfig, CommandErrorDto> {
    let raw: CompiledConnectionConfig =
        serde_json::from_str(source).map_err(|_| CommandErrorDto::not_configured())?;
    prepare_raw_connection(&raw, home)
}

fn prepare_raw_connection(
    raw: &CompiledConnectionConfig,
    home: Option<&Path>,
) -> Result<PreparedConnectionConfig, CommandErrorDto> {
    let identity = prepare_raw_identity(raw, home)?;
    let credential_account = external_credential_account(&raw.instance_id, &raw.certificate_sha256)
        .ok_or_else(CommandErrorDto::not_configured)?;
    if raw.credential_service != EXTERNAL_KEYRING_SERVICE
        || raw.credential_account != credential_account
    {
        return Err(CommandErrorDto::not_configured());
    }
    KeyringCredentialProvider::new(EXTERNAL_KEYRING_SERVICE, credential_account.clone())
        .map_err(|_| CommandErrorDto::not_configured())?;

    Ok(PreparedConnectionConfig {
        instance_id: identity.instance_id,
        certificate_sha256: identity.certificate_sha256,
        public_api_dir: identity.public_api_dir,
        credential_service: EXTERNAL_KEYRING_SERVICE.to_owned(),
        credential_account,
    })
}

fn prepare_raw_identity(
    raw: &CompiledConnectionConfig,
    home: Option<&Path>,
) -> Result<PreparedConnectionIdentity, CommandErrorDto> {
    let instance_id =
        InstanceId::from_str(&raw.instance_id).map_err(|_| CommandErrorDto::not_configured())?;
    if instance_id.as_uuid().is_nil() || raw.certificate_sha256 == PLACEHOLDER_FINGERPRINT {
        return Err(CommandErrorDto::not_configured());
    }
    let certificate_sha256 = TlsFingerprint::from_hex(&raw.certificate_sha256)
        .map_err(|_| CommandErrorDto::not_configured())?;
    let public_api_dir = expand_private_path(&raw.public_api_dir, home)?;
    Ok(PreparedConnectionIdentity {
        instance_id,
        certificate_sha256,
        public_api_dir,
    })
}

fn prepare_saved_connection(
    target: &ExternalTargetSetting,
) -> Result<PreparedConnectionConfig, CommandErrorDto> {
    if target.requires_credential_enrollment {
        return Err(CommandErrorDto::local_sanitized(
            "credential_reenrollment_required",
            "Re-enroll this daemon into the Desktop-bound keychain entry, then import its updated connection file.",
            false,
        ));
    }
    let instance_id =
        InstanceId::from_str(&target.instance_id).map_err(|_| CommandErrorDto::not_configured())?;
    if instance_id.as_uuid().is_nil()
        || target.certificate_sha256 == PLACEHOLDER_FINGERPRINT
        || !valid_external_label(&target.label)
    {
        return Err(CommandErrorDto::not_configured());
    }
    let certificate_sha256 = TlsFingerprint::from_hex(&target.certificate_sha256)
        .map_err(|_| CommandErrorDto::not_configured())?;
    let public_api_dir = expand_private_path(&target.public_api_dir.to_string_lossy(), None)?;
    let credential_account =
        external_credential_account(&target.instance_id, &target.certificate_sha256)
            .ok_or_else(CommandErrorDto::not_configured)?;
    if target.credential_service != EXTERNAL_KEYRING_SERVICE
        || target.credential_account != credential_account
    {
        return Err(CommandErrorDto::not_configured());
    }
    KeyringCredentialProvider::new(EXTERNAL_KEYRING_SERVICE, credential_account.clone())
        .map_err(|_| CommandErrorDto::not_configured())?;
    Ok(PreparedConnectionConfig {
        instance_id,
        certificate_sha256,
        public_api_dir,
        credential_service: EXTERNAL_KEYRING_SERVICE.to_owned(),
        credential_account,
    })
}

fn connection_file_error() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "connectionFile",
        "Choose a valid owner-controlled Colossus desktop connection JSON file.",
    )
}

fn expand_private_path(value: &str, home: Option<&Path>) -> Result<PathBuf, CommandErrorDto> {
    if value.is_empty() || value.len() > 2_048 || value.chars().any(char::is_control) {
        return Err(CommandErrorDto::not_configured());
    }
    let path = if value == "~" {
        home.map(Path::to_owned)
            .ok_or_else(CommandErrorDto::not_configured)?
    } else if let Some(relative) = value.strip_prefix("~/") {
        home.ok_or_else(CommandErrorDto::not_configured)?
            .join(relative)
    } else {
        PathBuf::from(value)
    };

    if !valid_local_absolute_path(&path) {
        return Err(CommandErrorDto::not_configured());
    }
    Ok(path)
}

fn valid_local_absolute_path(path: &Path) -> bool {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|component| {
            matches!(component, Component::ParentDir | Component::CurDir)
        })
    {
        return false;
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;

        path.components().next().is_some_and(|component| {
            matches!(
                component,
                Component::Prefix(prefix)
                    if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
            )
        })
    }
    #[cfg(not(windows))]
    {
        !path
            .components()
            .any(|component| matches!(component, Component::Prefix(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "instanceId":"01968a3e-0ab3-7f10-bb27-4eadbd550007",
        "certificateSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "publicApiDir":"~/.colossus-public-api",
        "credentialService":"com.obscuritylabs.colossus.desktop.external",
        "credentialAccount":"daemon-01968a3e-0ab3-7f10-bb27-4eadbd550007-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }"#;
    const LEGACY: &str = r#"{
        "instanceId":"01968a3e-0ab3-7f10-bb27-4eadbd550007",
        "certificateSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "publicApiDir":"~/.colossus-public-api",
        "credentialService":"com.obscuritylabs.colossus.desktop",
        "credentialAccount":"colossus-public-api"
    }"#;

    fn test_home() -> &'static Path {
        #[cfg(windows)]
        {
            Path::new(r"C:\Users\test")
        }
        #[cfg(not(windows))]
        {
            Path::new("/Users/test")
        }
    }

    #[test]
    fn accepts_compiled_trust_anchor_and_expands_home() {
        let config = prepare_connection(VALID, Some(test_home()))
            .expect("valid connection config");
        assert_eq!(
            config.public_api_dir,
            test_home().join(".colossus-public-api")
        );
    }

    #[test]
    fn legacy_compiled_target_is_retained_without_copying_credential_authority() {
        let target = legacy_compiled_target_from_source(
            LEGACY,
            "external-default",
            Some(test_home()),
        )
        .expect("legacy target metadata");
        assert!(target.requires_credential_enrollment);
        assert_eq!(target.credential_service, EXTERNAL_KEYRING_SERVICE);
        assert_eq!(
            target.credential_account,
            external_credential_account(&target.instance_id, &target.certificate_sha256)
                .expect("identity-bound account")
        );
        assert_ne!(target.credential_account, LEGACY_DESKTOP_KEYRING_ACCOUNT);

        let arbitrary = LEGACY.replace(LEGACY_DESKTOP_KEYRING_ACCOUNT, "unrelated-keychain-entry");
        assert!(
            legacy_compiled_target_from_source(
                &arbitrary,
                "external-default",
                Some(test_home()),
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn connection_paths_reject_remote_windows_namespaces() {
        assert!(valid_local_absolute_path(Path::new(
            r"C:\Users\test\.colossus-public-api"
        )));
        assert!(!valid_local_absolute_path(Path::new(
            r"\\server\share\.colossus-public-api"
        )));
        assert!(!valid_local_absolute_path(Path::new(
            r"\\?\UNC\server\share\.colossus-public-api"
        )));
    }

    #[test]
    fn rejects_placeholder_identity_and_fingerprint() {
        assert!(prepare_connection(TEMPLATE_CONNECTION, Some(test_home())).is_err());
        let nil = VALID.replace(
            "01968a3e-0ab3-7f10-bb27-4eadbd550007",
            "00000000-0000-0000-0000-000000000000",
        );
        assert!(prepare_connection(&nil, Some(test_home())).is_err());
        let zero_pin = VALID.replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            PLACEHOLDER_FINGERPRINT,
        );
        assert!(prepare_connection(&zero_pin, Some(test_home())).is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_unsafe_paths() {
        let unknown = VALID.replace("\n    }", ",\n        \"credential\":\"secret\"\n    }");
        assert!(prepare_connection(&unknown, Some(test_home())).is_err());

        for path in ["relative/api", "~/../other", "/tmp/../other", "/"] {
            let source = VALID.replace("~/.colossus-public-api", path);
            assert!(
                prepare_connection(&source, Some(test_home())).is_err(),
                "accepted unsafe path {path}"
            );
        }

        let arbitrary_service = VALID.replace(
            "com.obscuritylabs.colossus.desktop.external",
            "com.example.unrelated-secret",
        );
        assert!(prepare_connection(&arbitrary_service, Some(test_home())).is_err());
        let arbitrary_account = VALID.replace(
            "daemon-01968a3e-0ab3-7f10-bb27-4eadbd550007-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "another-keychain-account",
        );
        assert!(prepare_connection(&arbitrary_account, Some(test_home())).is_err());
    }

    #[test]
    fn setup_errors_do_not_disclose_native_configuration() {
        let source = VALID.replace("~/.colossus-public-api", "/Users/private/secret/../daemon");
        let error = prepare_connection(&source, Some(test_home()))
            .err()
            .expect("unsafe path rejected");
        let serialized = serde_json::to_string(&error).expect("error serializes");
        assert!(!serialized.contains("/Users/private"));
        assert!(!serialized.contains("certificateSha256"));
        assert!(!serialized.contains("credential"));
    }

    #[test]
    fn sdk_configuration_errors_map_without_details() {
        let error = CommandErrorDto::from_sdk(SdkError::PathNotAbsolute(PathBuf::from(
            "/private/native/path",
        )));
        let serialized = serde_json::to_string(&error).expect("error serializes");
        assert!(!serialized.contains("/private/native/path"));
    }

    #[cfg(unix)]
    #[test]
    fn imports_only_owner_controlled_regular_connection_files() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = tempfile::tempdir().expect("root");
        let config_path = root.path().join("connection.json");
        let source = VALID.replace("~/.colossus-public-api", "/private/tmp/colossus-api");
        fs::write(&config_path, &source).expect("write config");
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))
            .expect("config permissions");
        let config_path = fs::canonicalize(config_path).expect("canonical config");

        let imported = import_target(&config_path).expect("import target");
        assert!(Uuid::parse_str(&imported.target_id).is_ok());
        assert_eq!(imported.label, "External 01968a3e");

        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o622))
            .expect("unsafe permissions");
        assert!(import_target(&config_path).is_err());
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))
            .expect("restore permissions");

        let link_path = root.path().join("connection-link.json");
        symlink(&config_path, &link_path).expect("symlink");
        assert!(import_target(&link_path).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn imports_owner_private_windows_connection_files() {
        let parent = tempfile::tempdir().expect("root");
        let root = parent.path().join("private");
        colossus_windows_native::create_private_directory(&root).expect("private root");
        let config_path = root.join("connection.json");
        let source = VALID.replace(
            "~/.colossus-public-api",
            r"C:\Users\test\AppData\Local\colossus-api",
        );
        fs::write(&config_path, source).expect("write config");
        let config_path = fs::canonicalize(config_path).expect("canonical config");

        let imported = import_target(&config_path).expect("import target");
        assert!(Uuid::parse_str(&imported.target_id).is_ok());
        assert_eq!(imported.label, "External 01968a3e");
    }

    #[cfg(unix)]
    #[test]
    fn opened_file_identity_must_match_the_validated_path() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().expect("root");
        let first = root.path().join("first.json");
        let second = root.path().join("second.json");
        fs::write(&first, VALID).expect("first");
        fs::write(&second, VALID).expect("second");
        fs::set_permissions(&first, fs::Permissions::from_mode(0o600)).expect("first mode");
        fs::set_permissions(&second, fs::Permissions::from_mode(0o600)).expect("second mode");
        let first = fs::metadata(first).expect("first metadata");
        let second = fs::metadata(second).expect("second metadata");
        assert!(!same_file_metadata(&first, &second));
    }
}
