use super::{
    globals::{SetupGlobals, SetupResource},
    types::{SavedSetupPackage, invalid},
};
use crate::{
    desktop_settings::DesktopSettings,
    dto::CommandErrorDto,
    managed_configuration::{CatalogEntrySetting, McpCredentialHeaderSetting},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn current<T>(entry: &CatalogEntrySetting<T>) -> Option<&T> {
    (!entry.archived)
        .then(|| {
            entry
                .revisions
                .iter()
                .find(|r| r.revision == entry.current_revision)
        })
        .flatten()
        .map(|r| &r.value)
}

/// Export current global definitions, without archived entries or revision history.
pub(super) fn snapshot(settings: &DesktopSettings) -> Result<DesktopSettings, CommandErrorDto> {
    let mut snapshot = settings.clone();
    snapshot.providers = settings
        .global_configuration
        .providers
        .iter()
        .filter_map(current)
        .cloned()
        .collect();
    snapshot.models = settings
        .global_configuration
        .models
        .iter()
        .filter_map(current)
        .cloned()
        .collect();
    if settings.global_configuration.providers.is_empty() {
        snapshot.providers.clone_from(&settings.providers);
    }
    if settings.global_configuration.models.is_empty() {
        snapshot.models.clone_from(&settings.models);
    }
    let providers = snapshot
        .providers
        .iter()
        .map(|p| &p.profile)
        .collect::<BTreeSet<_>>();
    let models = snapshot
        .models
        .iter()
        .map(|m| &m.profile)
        .collect::<BTreeSet<_>>();
    if providers.len() != snapshot.providers.len() || models.len() != snapshot.models.len() {
        return Err(invalid(
            "Rename duplicate provider or model profiles in global settings before exporting.",
        ));
    }
    if snapshot
        .models
        .iter()
        .any(|m| !providers.contains(&m.provider_profile))
    {
        return Err(invalid(
            "A global model references a missing or archived provider. Restore its provider or archive the model before exporting.",
        ));
    }
    snapshot
        .model_roles
        .retain(|_, target| snapshot.models.iter().any(|m| &m.profile == target));
    if !snapshot.model_roles.contains_key("primary")
        && let Some(model) = snapshot.models.first()
    {
        snapshot
            .model_roles
            .insert("primary".into(), model.profile.clone());
    }
    Ok(snapshot)
}

fn resources<T: Clone>(entries: &[CatalogEntrySetting<T>], prefix: &str) -> Vec<SetupResource<T>> {
    entries
        .iter()
        .filter_map(|entry| current(entry).map(|value| (entry, value)))
        .enumerate()
        .map(|(i, (entry, value))| SetupResource {
            id: format!("{prefix}-{}", i + 1),
            label: entry.label.clone(),
            configuration: value.clone(),
        })
        .collect()
}

pub(super) fn complete(
    settings: &DesktopSettings,
    mut package: SavedSetupPackage,
) -> Result<SavedSetupPackage, CommandErrorDto> {
    let global = &settings.global_configuration;
    let mut portable = SetupGlobals {
        credentials: BTreeMap::new(),
        defaults: global.defaults.current().cloned(),
        mcp_servers: resources(&global.mcp_servers, "mcp"),
        search_providers: resources(&global.search_providers, "search"),
        telemetry_profiles: resources(&global.telemetry_profiles, "telemetry"),
    };
    // Literal HTTP headers can contain authentication. Export requirements, never
    // their values; recipients fill the missing slots using the native secret dialog.
    let mut literal_labels = BTreeMap::new();
    for resource in &mut portable.mcp_servers {
        let server = &mut resource.configuration;
        for (name, _) in std::mem::take(&mut server.headers) {
            literal_labels.insert(
                format!("literal:{}:{name}", resource.id),
                format!("{} {name}", resource.label),
            );
            server
                .credential_headers
                .entry(name.clone())
                .or_insert_with(|| McpCredentialHeaderSetting {
                    scheme: None,
                    credential_id: format!("literal:{}:{name}", resource.id),
                });
        }
    }
    let mut slots = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    portable.map_credentials(|id| {
        if let Some(slot) = slots.get(id) {
            return String::clone(slot);
        }
        let credential = super::credential_metadata::for_export(
            global
                .credentials
                .iter()
                .find(|credential| credential.id == id),
            literal_labels.get(id).map_or("Credential", String::as_str),
        );
        let slot = super::credential_metadata::unique_slot(&credential.label, &metadata);
        metadata.insert(slot.clone(), credential);
        slots.insert(id.to_owned(), slot.clone());
        slot
    });
    portable.credentials = metadata;
    portable.validate()?;
    let mut config: serde_json::Value = serde_saphyr::from_str(&package.config_yaml)
        .map_err(|_| invalid("Configuration could not be exported."))?;
    config["desktop"] = serde_json::to_value(&portable)
        .map_err(|_| invalid("Desktop settings could not be exported."))?;
    package.config_yaml = serde_saphyr::to_string(&config)
        .map_err(|_| invalid("Configuration could not be exported."))?;
    package.manifest.schema_version = 2;
    for (i, provider) in package.providers.iter().enumerate() {
        let profile = &provider.connection.profile;
        if let Some(entry) = global
            .providers
            .iter()
            .find(|entry| current(entry).is_some_and(|p| p.profile == *profile))
        {
            let value = super::provider_presentation(settings, &entry.id).unwrap_or_default();
            let mut presentation = super::types::Presentation {
                display_name: entry.label.clone(),
                description_markdown: value.description_markdown,
                icon: None,
                dark_icon: None,
            };
            for (theme, data, path) in [
                ("light", value.icon, &mut presentation.icon),
                ("dark", value.dark_icon, &mut presentation.dark_icon),
            ] {
                if let Some(data) = data {
                    let target = format!("assets/provider-{i}-{theme}.png");
                    package.icons.insert(target.clone(), data);
                    *path = Some(target);
                }
            }
            package
                .manifest
                .providers
                .insert(profile.clone(), presentation);
        }
    }
    Ok(package)
}

pub(super) fn include_ca(
    store: &crate::desktop_settings::SettingsStore,
    settings: &DesktopSettings,
    package: &mut SavedSetupPackage,
) -> Result<(), CommandErrorDto> {
    use std::io::Read as _;
    if let Some(bundle) = &settings.additional_ca_bundle {
        let path = store.ca_bundle_path(bundle)?;
        let mut pem = String::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(256 * 1024 + 1).read_to_string(&mut pem))
            .map_err(|_| invalid("The current CA bundle could not be exported."))?;
        let roots = super::archive::validate_ca(&pem)?;
        if roots.fingerprints_sha256() != bundle.fingerprints_sha256 {
            return Err(invalid("The current CA bundle changed. Retry export."));
        }
        package.manifest.ca_bundle = Some("certificates/desktop-ca.pem".into());
        package.ca_pem = Some(pem);
    }
    Ok(())
}
