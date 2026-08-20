use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::{
    desktop_settings::{
        AccessProfileSetting, ExecutionBoundarySetting, ModelSetting, ProviderSetting,
        WorkspaceProfile,
    },
    dto::CommandErrorDto,
};

const MAX_CATALOG_ENTRIES: usize = 256;
const MAX_CATALOG_REVISIONS: usize = 64;
const MAX_FIELD_OVERRIDES: usize = 512;
const MAX_FIELD_ID_BYTES: usize = 160;
const MAX_LABEL_BYTES: usize = 96;
const MAX_IMPORT_PATH_BYTES: usize = 2_048;
const MAX_VALUE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FieldOverrideSetting {
    pub(crate) field_id: String,
    pub(crate) value: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CatalogRevisionSetting<T> {
    pub(crate) revision: u64,
    pub(crate) value: T,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CatalogEntrySetting<T> {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) current_revision: u64,
    #[serde(default)]
    pub(crate) archived: bool,
    pub(crate) revisions: Vec<CatalogRevisionSetting<T>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialKindSetting {
    ApiKey,
    BearerToken,
    ClientSecret,
    GenericSecret,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialBackendSetting {
    Desktop,
    LegacyProvider,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CredentialMetadataSetting {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) kind: CredentialKindSetting,
    pub(crate) backend: CredentialBackendSetting,
    pub(crate) created_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum McpTransportSetting {
    Stdio,
    StreamableHttp,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct McpCredentialHeaderSetting {
    #[serde(default)]
    pub(crate) scheme: Option<String>,
    pub(crate) credential_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct McpOAuthSetting {
    pub(crate) client_id: String,
    #[serde(default)]
    pub(crate) client_secret_credential_id: Option<String>,
    pub(crate) callback_port: u16,
    #[serde(default)]
    pub(crate) scopes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct McpResearchToolSetting {
    pub(crate) tool: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default = "empty_object")]
    pub(crate) arguments: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct McpServerSetting {
    pub(crate) name: String,
    pub(crate) transport: McpTransportSetting,
    #[serde(default)]
    pub(crate) command: Option<String>,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) working_directory: Option<String>,
    #[serde(default)]
    pub(crate) environment_credentials: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) credential_headers: BTreeMap<String, McpCredentialHeaderSetting>,
    #[serde(default)]
    pub(crate) allow_stateless: bool,
    #[serde(default)]
    pub(crate) oauth: Option<McpOAuthSetting>,
    #[serde(default)]
    pub(crate) allowed_tools: Vec<String>,
    #[serde(default)]
    pub(crate) research_tools: Vec<McpResearchToolSetting>,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) max_output_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SearchProviderKindSetting {
    Searxng,
    SerpApi,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SearchProviderSetting {
    pub(crate) profile: String,
    pub(crate) kind: SearchProviderKindSetting,
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) credential_id: Option<String>,
    #[serde(default)]
    pub(crate) auth_header: Option<String>,
    pub(crate) timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OtlpProtocolSetting {
    Grpc,
    HttpProtobuf,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalPayloadSetting {
    Disabled,
    Metadata,
    Full,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TelemetryProfileSetting {
    pub(crate) name: String,
    pub(crate) endpoint: Option<String>,
    pub(crate) protocol: OtlpProtocolSetting,
    pub(crate) timeout_ms: u64,
    pub(crate) traces_enabled: bool,
    /// Trace sampling ratio in millionths, avoiding non-finite persisted values.
    pub(crate) trace_sample_ratio_millionths: u32,
    pub(crate) metrics_enabled: bool,
    pub(crate) metric_export_interval_ms: u64,
    pub(crate) logs_otlp: bool,
    pub(crate) logs_stdout_json: bool,
    pub(crate) journal_payloads: JournalPayloadSetting,
    pub(crate) acknowledge_sensitive_content: bool,
    pub(crate) acknowledge_insecure_transport: bool,
    #[serde(default)]
    pub(crate) resource_attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CatalogReferenceSetting {
    pub(crate) resource_id: String,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DefaultOverridesSetting {
    #[serde(default)]
    pub(crate) access_profile: Option<AccessProfileSetting>,
    #[serde(default)]
    pub(crate) execution_boundary: Option<ExecutionBoundarySetting>,
    #[serde(default)]
    pub(crate) terminal_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) field_overrides: Vec<FieldOverrideSetting>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GlobalDefaultsSetting {
    pub(crate) current_revision: u64,
    pub(crate) revisions: Vec<CatalogRevisionSetting<DefaultOverridesSetting>>,
}

impl Default for GlobalDefaultsSetting {
    fn default() -> Self {
        Self {
            current_revision: 1,
            revisions: vec![CatalogRevisionSetting {
                revision: 1,
                value: DefaultOverridesSetting::default(),
            }],
        }
    }
}

impl GlobalDefaultsSetting {
    pub(crate) fn revision(&self, revision: u64) -> Option<&DefaultOverridesSetting> {
        self.revisions
            .iter()
            .find(|candidate| candidate.revision == revision)
            .map(|candidate| &candidate.value)
    }

    pub(crate) fn current(&self) -> Option<&DefaultOverridesSetting> {
        self.revision(self.current_revision)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GlobalConfigurationSetting {
    pub(crate) revision: u64,
    #[serde(default)]
    pub(crate) providers: Vec<CatalogEntrySetting<ProviderSetting>>,
    #[serde(default)]
    pub(crate) models: Vec<CatalogEntrySetting<ModelSetting>>,
    #[serde(default)]
    pub(crate) mcp_servers: Vec<CatalogEntrySetting<McpServerSetting>>,
    #[serde(default)]
    pub(crate) search_providers: Vec<CatalogEntrySetting<SearchProviderSetting>>,
    #[serde(default)]
    pub(crate) telemetry_profiles: Vec<CatalogEntrySetting<TelemetryProfileSetting>>,
    #[serde(default)]
    pub(crate) credentials: Vec<CredentialMetadataSetting>,
    #[serde(default)]
    pub(crate) defaults: GlobalDefaultsSetting,
}

impl Default for GlobalConfigurationSetting {
    fn default() -> Self {
        Self {
            revision: 1,
            providers: Vec::new(),
            models: Vec::new(),
            mcp_servers: Vec::new(),
            search_providers: Vec::new(),
            telemetry_profiles: Vec::new(),
            credentials: Vec::new(),
            defaults: GlobalDefaultsSetting::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportProvenanceSetting {
    pub(crate) relative_path: String,
    pub(crate) sha256: String,
    pub(crate) imported_at_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SpaceConfigurationSetting {
    pub(crate) accepted_global_revision: u64,
    #[serde(default)]
    pub(crate) catalog_revisions: BTreeMap<String, CatalogReferenceSetting>,
    #[serde(default)]
    pub(crate) credential_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) access_profile_override: Option<AccessProfileSetting>,
    #[serde(default)]
    pub(crate) execution_boundary_override: Option<ExecutionBoundarySetting>,
    #[serde(default)]
    pub(crate) terminal_enabled_override: Option<bool>,
    #[serde(default)]
    pub(crate) field_overrides: Vec<FieldOverrideSetting>,
    #[serde(default)]
    pub(crate) import: Option<ImportProvenanceSetting>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedSpaceConfiguration {
    pub(crate) access_profile: AccessProfileSetting,
    pub(crate) execution_boundary: ExecutionBoundarySetting,
    pub(crate) terminal_enabled: bool,
    pub(crate) field_overrides: Vec<FieldOverrideSetting>,
}

pub(crate) fn resolve_space_configuration(
    global: &GlobalConfigurationSetting,
    space: &WorkspaceProfile,
) -> Result<ResolvedSpaceConfiguration, CommandErrorDto> {
    let defaults = global
        .defaults
        .revision(space.configuration.accepted_global_revision)
        .ok_or_else(configuration_error)?;
    let mut field_overrides = defaults
        .field_overrides
        .iter()
        .map(|field| (field.field_id.clone(), field.value.clone()))
        .collect::<BTreeMap<_, _>>();
    for field in &space.configuration.field_overrides {
        field_overrides.insert(field.field_id.clone(), field.value.clone());
    }
    Ok(ResolvedSpaceConfiguration {
        access_profile: space
            .configuration
            .access_profile_override
            .or(defaults.access_profile)
            .unwrap_or(AccessProfileSetting::AllowAll),
        execution_boundary: space
            .configuration
            .execution_boundary_override
            .or(defaults.execution_boundary)
            .unwrap_or(ExecutionBoundarySetting::FullAccess),
        terminal_enabled: space
            .configuration
            .terminal_enabled_override
            .or(defaults.terminal_enabled)
            .unwrap_or(false),
        field_overrides: field_overrides
            .into_iter()
            .map(|(field_id, value)| FieldOverrideSetting { field_id, value })
            .collect(),
    })
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

pub(crate) fn initialize_catalog(
    global: &mut GlobalConfigurationSetting,
    spaces: &mut [WorkspaceProfile],
) {
    if global.revision == 0 {
        global.revision = 1;
    }
    if global.defaults.current_revision == 0 || global.defaults.revisions.is_empty() {
        global.defaults = GlobalDefaultsSetting::default();
    }
    for space in spaces {
        if space.configuration.accepted_global_revision == 0 {
            space.configuration.accepted_global_revision = global.revision;
        }
        for provider in &space.providers {
            let key = format!("provider:{}", provider.profile);
            if !space.configuration.catalog_revisions.contains_key(&key) {
                let reference =
                    ensure_catalog_entry(&mut global.providers, &provider.profile, provider);
                space.configuration.catalog_revisions.insert(key, reference);
            }
            if let Some(credential_id) = provider.credential_id.as_deref()
                && global
                    .credentials
                    .iter()
                    .all(|credential| credential.id != credential_id)
            {
                global.credentials.push(CredentialMetadataSetting {
                    id: credential_id.to_owned(),
                    label: format!("{} API key", provider.profile),
                    kind: CredentialKindSetting::ApiKey,
                    backend: CredentialBackendSetting::LegacyProvider,
                    created_at_ms: space.last_opened_at_ms,
                });
            }
        }
        for model in &space.models {
            let key = format!("model:{}", model.profile);
            if !space.configuration.catalog_revisions.contains_key(&key) {
                let reference = ensure_catalog_entry(&mut global.models, &model.profile, model);
                space.configuration.catalog_revisions.insert(key, reference);
            }
        }
    }
}

fn ensure_catalog_entry<T: Clone + PartialEq>(
    entries: &mut Vec<CatalogEntrySetting<T>>,
    label: &str,
    value: &T,
) -> CatalogReferenceSetting {
    for entry in entries.iter() {
        if let Some(revision) = entry
            .revisions
            .iter()
            .find(|revision| &revision.value == value)
        {
            return CatalogReferenceSetting {
                resource_id: entry.id.clone(),
                revision: revision.revision,
            };
        }
    }
    let id = Uuid::now_v7().to_string();
    entries.push(CatalogEntrySetting {
        id: id.clone(),
        label: label.to_owned(),
        current_revision: 1,
        archived: false,
        revisions: vec![CatalogRevisionSetting {
            revision: 1,
            value: value.clone(),
        }],
    });
    CatalogReferenceSetting {
        resource_id: id,
        revision: 1,
    }
}

pub(crate) fn validate_configuration(
    global: &GlobalConfigurationSetting,
    spaces: &[WorkspaceProfile],
) -> Result<(), CommandErrorDto> {
    if global.revision == 0
        || global.defaults.current_revision == 0
        || global.providers.len() > MAX_CATALOG_ENTRIES
        || global.models.len() > MAX_CATALOG_ENTRIES
        || global.mcp_servers.len() > MAX_CATALOG_ENTRIES
        || global.search_providers.len() > MAX_CATALOG_ENTRIES
        || global.telemetry_profiles.len() > MAX_CATALOG_ENTRIES
        || global.credentials.len() > MAX_CATALOG_ENTRIES
    {
        return Err(configuration_error());
    }
    validate_default_revisions(&global.defaults)?;
    validate_entries(&global.providers)?;
    validate_entries(&global.models)?;
    validate_entries(&global.mcp_servers)?;
    validate_entries(&global.search_providers)?;
    validate_telemetry_entries(&global.telemetry_profiles)?;

    let mut credential_ids = BTreeSet::new();
    for credential in &global.credentials {
        if !valid_id(&credential.id)
            || !valid_label(&credential.label)
            || !credential_ids.insert(credential.id.as_str())
        {
            return Err(configuration_error());
        }
    }
    for space in spaces {
        if space.configuration.accepted_global_revision == 0
            || validate_overrides(&space.configuration.field_overrides).is_err()
            || global
                .defaults
                .revision(space.configuration.accepted_global_revision)
                .is_none()
            || space.configuration.catalog_revisions.iter().any(|(key, reference)| {
                reference.revision == 0
                    || !valid_catalog_reference(global, key, reference)
            })
            || space
                .configuration
                .credential_overrides
                .values()
                .any(|id| !credential_ids.contains(id.as_str()))
            || space.configuration.import.as_ref().is_some_and(|import| {
                import.relative_path.is_empty()
                    || import.relative_path.len() > MAX_IMPORT_PATH_BYTES
                    || import.sha256.len() != 64
                    || !import.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(configuration_error());
        }
    }
    Ok(())
}

fn validate_default_revisions(defaults: &GlobalDefaultsSetting) -> Result<(), CommandErrorDto> {
    if defaults.revisions.is_empty()
        || defaults.revisions.len() > MAX_CATALOG_REVISIONS
        || defaults.current().is_none()
    {
        return Err(configuration_error());
    }
    let mut revisions = BTreeSet::new();
    if defaults.revisions.iter().any(|revision| {
        revision.revision == 0
            || !revisions.insert(revision.revision)
            || validate_overrides(&revision.value.field_overrides).is_err()
    }) {
        return Err(configuration_error());
    }
    Ok(())
}

fn valid_catalog_reference(
    global: &GlobalConfigurationSetting,
    key: &str,
    reference: &CatalogReferenceSetting,
) -> bool {
    if key.starts_with("provider:") {
        catalog_contains_revision(&global.providers, reference)
    } else if key.starts_with("model:") {
        catalog_contains_revision(&global.models, reference)
    } else if key.starts_with("mcp:") {
        catalog_contains_revision(&global.mcp_servers, reference)
    } else if key.starts_with("search:") {
        catalog_contains_revision(&global.search_providers, reference)
    } else if key.starts_with("telemetry:") {
        catalog_contains_revision(&global.telemetry_profiles, reference)
    } else {
        false
    }
}

fn catalog_contains_revision<T>(
    entries: &[CatalogEntrySetting<T>],
    reference: &CatalogReferenceSetting,
) -> bool {
    entries.iter().any(|entry| {
        entry.id == reference.resource_id
            && entry
                .revisions
                .iter()
                .any(|revision| revision.revision == reference.revision)
    })
}

fn validate_entries<T>(entries: &[CatalogEntrySetting<T>]) -> Result<(), CommandErrorDto> {
    let mut ids = BTreeSet::new();
    for entry in entries {
        if !valid_id(&entry.id)
            || !valid_label(&entry.label)
            || entry.current_revision == 0
            || entry.revisions.is_empty()
            || entry.revisions.len() > MAX_CATALOG_REVISIONS
            || !ids.insert(entry.id.as_str())
            || !entry
                .revisions
                .iter()
                .any(|revision| revision.revision == entry.current_revision)
        {
            return Err(configuration_error());
        }
        let mut revisions = BTreeSet::new();
        if entry
            .revisions
            .iter()
            .any(|revision| revision.revision == 0 || !revisions.insert(revision.revision))
        {
            return Err(configuration_error());
        }
    }
    Ok(())
}

fn validate_telemetry_entries(
    entries: &[CatalogEntrySetting<TelemetryProfileSetting>],
) -> Result<(), CommandErrorDto> {
    validate_entries(entries)?;
    if entries
        .iter()
        .flat_map(|entry| &entry.revisions)
        .any(|revision| revision.value.trace_sample_ratio_millionths > 1_000_000)
    {
        return Err(configuration_error());
    }
    Ok(())
}

fn validate_overrides(overrides: &[FieldOverrideSetting]) -> Result<(), CommandErrorDto> {
    if overrides.len() > MAX_FIELD_OVERRIDES {
        return Err(configuration_error());
    }
    let mut ids = BTreeSet::new();
    for field in overrides {
        if field.field_id.is_empty()
            || field.field_id.len() > MAX_FIELD_ID_BYTES
            || field.field_id.split('.').any(|segment| {
                segment.is_empty()
                    || !segment.as_bytes()[0].is_ascii_lowercase()
                    || !segment
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
            || !ids.insert(field.field_id.as_str())
            || serde_json::to_vec(&field.value).map_or(true, |value| value.len() > MAX_VALUE_BYTES)
        {
            return Err(configuration_error());
        }
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| !id.is_nil())
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LABEL_BYTES
        && !value.chars().any(char::is_control)
}

fn configuration_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "desktop_configuration",
        "The Desktop configuration catalog is invalid.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_settings::{ProviderKindSetting, WorkspaceSetting};
    use std::path::PathBuf;

    fn provider(base_url: &str) -> ProviderSetting {
        ProviderSetting {
            profile: "primary-provider".into(),
            kind: ProviderKindSetting::Compatible,
            base_url: base_url.into(),
            credential_id: None,
            timeout_ms: Some(120_000),
        }
    }

    fn space(id: &str, provider: ProviderSetting) -> WorkspaceProfile {
        WorkspaceProfile {
            id: id.into(),
            display_name: id.into(),
            archived: false,
            last_opened_at_ms: 42,
            workspace: WorkspaceSetting {
                id: id.into(),
                path: PathBuf::from(format!(r"C:\{id}")),
                identity: None,
                display_name: id.into(),
                display_path: format!(r"C:\{id}"),
            },
            providers: vec![provider],
            models: Vec::new(),
            model_roles: BTreeMap::new(),
            access_profile: AccessProfileSetting::AllowAll,
            execution_boundary: ExecutionBoundarySetting::FullAccess,
            terminal_enabled: false,
            configuration: SpaceConfigurationSetting::default(),
        }
    }

    #[test]
    fn catalog_initialization_deduplicates_only_exact_definitions() {
        let shared = provider("https://example.test/v1");
        let mut spaces = vec![
            space("one", shared.clone()),
            space("two", shared),
            space("three", provider("https://other.test/v1")),
        ];
        let mut global = GlobalConfigurationSetting::default();

        initialize_catalog(&mut global, &mut spaces);

        assert_eq!(global.providers.len(), 2);
        assert_eq!(
            spaces[0].configuration.catalog_revisions["provider:primary-provider"],
            spaces[1].configuration.catalog_revisions["provider:primary-provider"]
        );
        assert_ne!(
            spaces[0].configuration.catalog_revisions["provider:primary-provider"].resource_id,
            spaces[2].configuration.catalog_revisions["provider:primary-provider"].resource_id
        );
    }

    #[test]
    fn catalog_initialization_preserves_an_existing_revision_pin() {
        let original = provider("https://example.test/v1");
        let mut spaces = vec![space("one", original)];
        let mut global = GlobalConfigurationSetting::default();
        initialize_catalog(&mut global, &mut spaces);
        let pinned = spaces[0].configuration.catalog_revisions
            ["provider:primary-provider"]
            .clone();
        global.providers[0].revisions.push(CatalogRevisionSetting {
            revision: 2,
            value: provider("https://changed.test/v1"),
        });
        global.providers[0].current_revision = 2;

        initialize_catalog(&mut global, &mut spaces);

        assert_eq!(
            spaces[0].configuration.catalog_revisions["provider:primary-provider"],
            pinned
        );
    }

    #[test]
    fn accepted_global_revision_controls_effective_defaults() {
        let mut global = GlobalConfigurationSetting::default();
        global.revision = 2;
        global.defaults.current_revision = 2;
        global.defaults.revisions[0].value.access_profile = Some(AccessProfileSetting::Minimal);
        global.defaults.revisions.push(CatalogRevisionSetting {
            revision: 2,
            value: DefaultOverridesSetting {
                access_profile: Some(AccessProfileSetting::Development),
                field_overrides: vec![FieldOverrideSetting {
                    field_id: "research.maxSources".into(),
                    value: Value::from(20),
                }],
                ..DefaultOverridesSetting::default()
            },
        });
        let mut pinned = space("one", provider("https://example.test/v1"));
        pinned.configuration.accepted_global_revision = 1;

        let resolved = resolve_space_configuration(&global, &pinned).expect("revision one");

        assert_eq!(resolved.access_profile, AccessProfileSetting::Minimal);
        assert!(resolved.field_overrides.is_empty());
    }

    #[test]
    fn validation_rejects_unknown_or_cross_catalog_revisions() {
        let mut spaces = vec![space(
            "one",
            provider("https://example.test/v1"),
        )];
        let mut global = GlobalConfigurationSetting::default();
        initialize_catalog(&mut global, &mut spaces);
        spaces[0]
            .configuration
            .catalog_revisions
            .get_mut("provider:primary-provider")
            .expect("provider reference")
            .revision = 99;
        assert!(validate_configuration(&global, &spaces).is_err());

        spaces[0]
            .configuration
            .catalog_revisions
            .get_mut("provider:primary-provider")
            .expect("provider reference")
            .revision = 1;
        let reference = spaces[0]
            .configuration
            .catalog_revisions
            .remove("provider:primary-provider")
            .expect("provider reference");
        spaces[0]
            .configuration
            .catalog_revisions
            .insert("model:primary".into(), reference);
        assert!(validate_configuration(&global, &spaces).is_err());
    }
}
