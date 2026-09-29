use crate::{
    desktop_settings::{ModelSetting, ProviderSetting},
    dto::CommandErrorDto,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u16,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description_markdown: String,
    pub providers: BTreeMap<String, Presentation>,
    #[serde(default)]
    pub ca_bundle: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Presentation {
    pub display_name: String,
    #[serde(default)]
    pub description_markdown: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub dark_icon: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SetupProvider {
    pub connection: ProviderSetting,
    pub credential_slot: Option<String>,
}

/// Native-only saved setup data. DTOs deliberately exclude YAML and certificate bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SavedSetupPackage {
    pub manifest: Manifest,
    pub sha256: String,
    pub config_yaml: String,
    pub providers: Vec<SetupProvider>,
    pub models: Vec<ModelSetting>,
    pub roles: BTreeMap<String, String>,
    pub icons: BTreeMap<String, String>,
    pub ca_pem: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub description_markdown: String,
    pub providers: Vec<ProviderDto>,
    pub roles: BTreeMap<String, String>,
    pub certificate_fingerprints: Vec<String>,
    pub existing_certificate_fingerprints: Vec<String>,
    pub replaces_version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDto {
    pub profile: String,
    pub display_name: String,
    pub description_markdown: String,
    pub kind: crate::desktop_settings::ProviderKindSetting,
    pub base_url: String,
    pub timeout_ms: Option<u64>,
    pub credential_required: bool,
    pub credential_id: Option<String>,
    pub icon: Option<String>,
    pub dark_icon: Option<String>,
    pub models: Vec<ModelSetting>,
}

pub(super) fn invalid(message: &'static str) -> CommandErrorDto {
    CommandErrorDto::invalid("setupPackage", message)
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
