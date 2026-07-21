#[cfg(test)]
use colossus_sdk::SdkError;
use colossus_sdk::{
    ApiMajor, Colossus, DaemonConnectOptions, InstanceId, KeyringCredentialProvider, TlsFingerprint,
};
use directories::BaseDirs;
use serde::Deserialize;
use std::{
    path::{Component, Path, PathBuf},
    str::FromStr as _,
    sync::Arc,
};

use crate::dto::CommandErrorDto;

const COMPILED_CONNECTION: &str = include_str!(concat!(env!("OUT_DIR"), "/connection.json"));
#[cfg(test)]
const TEMPLATE_CONNECTION: &str = include_str!("../connection.json");
const ENDPOINT_FILE: &str = "endpoint.json";
const CERTIFICATE_FILE: &str = "certificate.pem";
const PLACEHOLDER_FINGERPRINT: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledConnectionConfig {
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

pub(crate) fn is_configured() -> bool {
    let home = home_dir();
    prepare_connection(COMPILED_CONNECTION, home.as_deref()).is_ok()
}

pub(crate) async fn connect() -> Result<Colossus, CommandErrorDto> {
    let home = home_dir();
    let prepared = prepare_connection(COMPILED_CONNECTION, home.as_deref())?;
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

fn home_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.home_dir().to_owned())
}

fn prepare_connection(
    source: &str,
    home: Option<&Path>,
) -> Result<PreparedConnectionConfig, CommandErrorDto> {
    let raw: CompiledConnectionConfig =
        serde_json::from_str(source).map_err(|_| CommandErrorDto::not_configured())?;
    let instance_id =
        InstanceId::from_str(&raw.instance_id).map_err(|_| CommandErrorDto::not_configured())?;
    if instance_id.as_uuid().is_nil() || raw.certificate_sha256 == PLACEHOLDER_FINGERPRINT {
        return Err(CommandErrorDto::not_configured());
    }
    let certificate_sha256 = TlsFingerprint::from_hex(&raw.certificate_sha256)
        .map_err(|_| CommandErrorDto::not_configured())?;
    let public_api_dir = expand_private_path(&raw.public_api_dir, home)?;
    KeyringCredentialProvider::new(
        raw.credential_service.clone(),
        raw.credential_account.clone(),
    )
    .map_err(|_| CommandErrorDto::not_configured())?;

    Ok(PreparedConnectionConfig {
        instance_id,
        certificate_sha256,
        public_api_dir,
        credential_service: raw.credential_service,
        credential_account: raw.credential_account,
    })
}

fn expand_private_path(value: &str, home: Option<&Path>) -> Result<PathBuf, CommandErrorDto> {
    let path = if value == "~" {
        home.map(Path::to_owned)
            .ok_or_else(CommandErrorDto::not_configured)?
    } else if let Some(relative) = value.strip_prefix("~/") {
        home.ok_or_else(CommandErrorDto::not_configured)?
            .join(relative)
    } else {
        PathBuf::from(value)
    };

    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err(CommandErrorDto::not_configured());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "instanceId":"01968a3e-0ab3-7f10-bb27-4eadbd550007",
        "certificateSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "publicApiDir":"~/.colossus-public-api",
        "credentialService":"com.example.colossus",
        "credentialAccount":"desktop"
    }"#;

    #[test]
    fn accepts_compiled_trust_anchor_and_expands_home() {
        let config = prepare_connection(VALID, Some(Path::new("/Users/test")))
            .expect("valid connection config");
        assert_eq!(
            config.public_api_dir,
            PathBuf::from("/Users/test/.colossus-public-api")
        );
    }

    #[test]
    fn rejects_placeholder_identity_and_fingerprint() {
        assert!(prepare_connection(TEMPLATE_CONNECTION, Some(Path::new("/tmp"))).is_err());
        let nil = VALID.replace(
            "01968a3e-0ab3-7f10-bb27-4eadbd550007",
            "00000000-0000-0000-0000-000000000000",
        );
        assert!(prepare_connection(&nil, Some(Path::new("/tmp"))).is_err());
        let zero_pin = VALID.replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            PLACEHOLDER_FINGERPRINT,
        );
        assert!(prepare_connection(&zero_pin, Some(Path::new("/tmp"))).is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_unsafe_paths() {
        let unknown = VALID.replace("\n    }", ",\n        \"credential\":\"secret\"\n    }");
        assert!(prepare_connection(&unknown, Some(Path::new("/tmp"))).is_err());

        for path in ["relative/api", "~/../other", "/tmp/../other", "/"] {
            let source = VALID.replace("~/.colossus-public-api", path);
            assert!(
                prepare_connection(&source, Some(Path::new("/Users/test"))).is_err(),
                "accepted unsafe path {path}"
            );
        }
    }

    #[test]
    fn setup_errors_do_not_disclose_native_configuration() {
        let source = VALID.replace("~/.colossus-public-api", "/Users/private/secret/../daemon");
        let error = prepare_connection(&source, Some(Path::new("/Users/test")))
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
}
