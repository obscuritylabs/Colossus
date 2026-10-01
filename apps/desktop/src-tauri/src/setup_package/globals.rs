//! Portable views of the existing Desktop settings, without history or host identities.
use super::{
    configuration::valid_slot,
    types::{invalid, valid_id},
};
use crate::{
    desktop_settings::{AccessProfileSetting, ExecutionBoundarySetting},
    dto::CommandErrorDto,
    managed_configuration::{
        CatalogEntrySetting, CatalogRevisionSetting, CredentialBackendSetting,
        CredentialKindSetting, CredentialMetadataSetting, DefaultOverridesSetting,
        GlobalConfigurationSetting, McpServerSetting, ResolvedSpaceConfiguration,
        SearchProviderSetting, TelemetryProfileSetting, validate_configuration,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SetupResource<T> {
    pub id: String,
    pub label: String,
    pub configuration: T,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SetupGlobals {
    /// Presentation only: tokens and machine-specific credential IDs are never portable.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub credentials: BTreeMap<String, super::credential_metadata::PortableCredential>,
    #[serde(default)]
    pub defaults: Option<DefaultOverridesSetting>,
    #[serde(default)]
    pub mcp_servers: Vec<SetupResource<McpServerSetting>>,
    #[serde(default)]
    pub search_providers: Vec<SetupResource<SearchProviderSetting>>,
    #[serde(default)]
    pub telemetry_profiles: Vec<SetupResource<TelemetryProfileSetting>>,
}

pub(super) fn from_yaml(yaml: &str) -> Result<SetupGlobals, CommandErrorDto> {
    let value: Value =
        serde_saphyr::from_str(yaml).map_err(|_| invalid("config.yaml is invalid."))?;
    value
        .get("desktop")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map(Option::unwrap_or_default)
        .map_err(|_| invalid("The desktop settings in config.yaml are invalid."))
}

impl SetupGlobals {
    pub(super) fn resource_keys(&self) -> impl Iterator<Item = String> + '_ {
        self.mcp_servers
            .iter()
            .map(|r| format!("mcp:{}", r.id))
            .chain(
                self.search_providers
                    .iter()
                    .map(|r| format!("search:{}", r.id)),
            )
            .chain(
                self.telemetry_profiles
                    .iter()
                    .map(|r| format!("telemetry:{}", r.id)),
            )
    }

    /// Traverse only typed secret references, never arbitrary strings or JSON values.
    pub(super) fn map_credentials(&mut self, mut map: impl FnMut(&str) -> String) {
        for resource in &mut self.search_providers {
            if let Some(id) = &mut resource.configuration.credential_id {
                *id = map(id);
            }
        }
        for resource in &mut self.mcp_servers {
            let server = &mut resource.configuration;
            for id in server.environment_credentials.values_mut() {
                *id = map(id);
            }
            for header in server.credential_headers.values_mut() {
                header.credential_id = map(&header.credential_id);
            }
            if let Some(id) = server
                .oauth
                .as_mut()
                .and_then(|o| o.client_secret_credential_id.as_mut())
            {
                *id = map(id);
            }
        }
    }

    pub(super) fn credential_slots(&self) -> BTreeSet<String> {
        let mut slots = BTreeSet::new();
        self.clone().map_credentials(|id| {
            slots.insert(id.to_owned());
            id.to_owned()
        });
        slots
    }

    pub(super) fn credential_consumers(
        &self,
        slot: &str,
    ) -> (
        Vec<&SetupResource<McpServerSetting>>,
        Vec<&SetupResource<SearchProviderSetting>>,
    ) {
        let mcp = self
            .mcp_servers
            .iter()
            .filter(|r| {
                let server = &r.configuration;
                server.environment_credentials.values().any(|id| id == slot)
                    || server
                        .credential_headers
                        .values()
                        .any(|h| h.credential_id == slot)
                    || server
                        .oauth
                        .as_ref()
                        .and_then(|o| o.client_secret_credential_id.as_deref())
                        == Some(slot)
            })
            .collect();
        let search = self
            .search_providers
            .iter()
            .filter(|r| r.configuration.credential_id.as_deref() == Some(slot))
            .collect();
        (mcp, search)
    }

    pub(super) fn validate(&self) -> Result<(), CommandErrorDto> {
        validate_portable_defaults(self.defaults.as_ref())?;
        let slots = self.credential_slots();
        super::credential_metadata::validate(&self.credentials, &slots)?;
        if slots.len() > 128 || slots.iter().any(|id| !valid_slot(id)) {
            return Err(invalid(
                "Use env:NAME placeholders for portable MCP and search credentials.",
            ));
        }
        let mut local = self.clone();
        let bindings = slots
            .into_iter()
            .map(|slot| (slot, uuid::Uuid::now_v7().to_string()))
            .collect::<BTreeMap<_, _>>();
        local.map_credentials(|slot| bindings[slot].clone());
        let mut global = GlobalConfigurationSetting {
            mcp_servers: validation_entries(&local.mcp_servers)?,
            search_providers: validation_entries(&local.search_providers)?,
            telemetry_profiles: validation_entries(&local.telemetry_profiles)?,
            credentials: bindings
                .iter()
                .map(|(slot, id)| missing_credential(slot, id))
                .collect(),
            ..GlobalConfigurationSetting::default()
        };
        if let Some(defaults) = &self.defaults {
            global.defaults.revisions[0].value = defaults.clone();
        }
        validate_configuration(&global, &[])?;
        // Reuse the managed transport's bounds and protocol validation. Validate each
        // reusable definition independently: alternative catalog entries may share names.
        let mut resolved = ResolvedSpaceConfiguration {
            access_profile: AccessProfileSetting::Minimal,
            execution_boundary: ExecutionBoundarySetting::FullAccess,
            terminal_enabled: false,
            field_overrides: vec![],
            providers: vec![],
            models: vec![],
            model_roles: BTreeMap::new(),
            search_roles: BTreeMap::new(),
            search_providers: vec![],
            mcp_servers: vec![],
            telemetry: None,
        };
        for resource in &local.mcp_servers {
            if !resource.configuration.headers.is_empty() {
                return Err(invalid(
                    "Portable MCP headers must use credentialHeaders; literal headers are not exported.",
                ));
            }
            resolved.mcp_servers = vec![resource.configuration.clone()];
            validate_transport(&resolved)?;
        }
        resolved.mcp_servers.clear();
        for resource in &local.search_providers {
            crate::provider_catalog::validate_setup_endpoint(
                crate::desktop_settings::ProviderKindSetting::Compatible,
                &resource.configuration.endpoint,
            )?;
            resolved.search_providers = vec![resource.configuration.clone()];
            validate_transport(&resolved)?;
        }
        resolved.search_providers.clear();
        for resource in &local.telemetry_profiles {
            resolved.telemetry = Some(resource.configuration.clone());
            validate_transport(&resolved)?;
        }
        Ok(())
    }

    /// Only inspect the defaults through the runtime parser. These grants are never
    /// applied here; the normal workspace approval flow owns activation.
    pub(super) fn apply_validation_defaults(
        &self,
        config: &mut Value,
    ) -> Result<(), CommandErrorDto> {
        config["sandbox"] = json!({});
        if let Some(defaults) = &self.defaults {
            if let Some(profile) = defaults.access_profile {
                config["access"] = json!({"profile":profile});
            }
            for field in &defaults.field_overrides {
                let segments = field.field_id.split('.').collect::<Vec<_>>();
                let mut target = &mut *config;
                for segment in &segments[..segments.len() - 1] {
                    if target.get(*segment).is_none() {
                        target[*segment] = json!({});
                    }
                    if !target[*segment].is_object() {
                        return Err(invalid("Conflicting global default field paths."));
                    }
                    target = &mut target[*segment];
                }
                target[segments[segments.len() - 1]] = field.value.clone();
            }
        }
        Ok(())
    }
}

/// Advanced runtime overrides do not participate in Desktop's credential catalog.
/// Inspect only their schema-defined authentication positions; arbitrary arguments
/// and user strings must not be rewritten or mistaken for credential references.
fn validate_portable_defaults(
    defaults: Option<&DefaultOverridesSetting>,
) -> Result<(), CommandErrorDto> {
    let Some(defaults) = defaults else {
        return Ok(());
    };
    let present = |value: &Value, path: &str| value.pointer(path).is_some_and(|v| !v.is_null());
    let populated = |value: &Value, path: &str| {
        value
            .pointer(path)
            .is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|m| !m.is_empty()))
    };
    for field in &defaults.field_overrides {
        let value = &field.value;
        let needs_local_credentials = match field.field_id.as_str() {
            "audit.exporter" => present(value, "/credentialReference"),
            "memory.semantic" => {
                present(value, "/credentialReference")
                    || present(value, "/embedding/credentialReference")
            }
            "plugins.registries" => value.as_object().is_some_and(|registries| {
                registries.values().any(|registry| {
                    present(registry, "/auth/credentialReference")
                        || registry.pointer("/auth/kind").and_then(Value::as_str) == Some("docker")
                })
            }),
            "plugins.mcpServers" => value.as_object().is_some_and(|servers| {
                servers.values().any(|server| {
                    populated(server, "/environment")
                        || populated(server, "/credentialHeaders")
                        || present(server, "/oauth/clientSecretReference")
                })
            }),
            _ => false,
        };
        if needs_local_credentials {
            return Err(CommandErrorDto::invalid(
                "setupPackage",
                &format!(
                    "Global default {} uses credentials that setup files cannot bind. Remove its authentication settings before sharing and configure them locally on each computer. Use the MCP or search catalog for portable credential placeholders.",
                    field.field_id
                ),
            ));
        }
    }
    Ok(())
}

fn validation_entries<T: Clone>(
    resources: &[SetupResource<T>],
) -> Result<Vec<CatalogEntrySetting<T>>, CommandErrorDto> {
    let mut ids = BTreeSet::new();
    if resources.len() > 64
        || resources
            .iter()
            .any(|r| !valid_id(&r.id) || !ids.insert(&r.id))
    {
        return Err(invalid(
            "Setup catalogs need unique portable IDs and at most 64 entries each.",
        ));
    }
    Ok(resources
        .iter()
        .map(|r| CatalogEntrySetting {
            id: uuid::Uuid::now_v7().to_string(),
            label: r.label.clone(),
            archived: false,
            current_revision: 1,
            revisions: vec![CatalogRevisionSetting {
                revision: 1,
                value: r.configuration.clone(),
            }],
        })
        .collect())
}

fn validate_transport(resolved: &ResolvedSpaceConfiguration) -> Result<(), CommandErrorDto> {
    let mut config = crate::managed_runtime::managed_runtime_config(resolved);
    let echo =
        colossus_sdk::ManagedRuntimeConfig::echo(colossus_sdk::ManagedAccessProfile::Minimal);
    config.providers = echo.providers;
    config.models = echo.models;
    config.roles = echo.roles;
    config
        .validate()
        .map_err(|_| invalid("An included MCP, search, or telemetry definition is invalid."))
}

pub(super) fn missing_credential(slot: &str, id: &str) -> CredentialMetadataSetting {
    CredentialMetadataSetting {
        id: id.into(),
        label: slot
            .strip_prefix("env:")
            .unwrap_or(slot)
            .chars()
            .take(96)
            .collect(),
        kind: CredentialKindSetting::GenericSecret,
        backend: CredentialBackendSetting::Desktop,
        created_at_ms: 0,
    }
}
