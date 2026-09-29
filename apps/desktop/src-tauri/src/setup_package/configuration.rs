use super::{
    archive::PackageSource,
    types::{PackageDto, ProviderDto, SavedSetupPackage, SetupProvider, invalid},
};
use crate::{
    desktop_settings::{
        CODEX_BASE_URL, DesktopSettings, ModelSetting, ProviderKindSetting, ProviderSetting,
    },
    dto::CommandErrorDto,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

/// Limit the portable payload to connection/model declarations. Validation-only grants
/// are derived for the existing runtime parser and are never persisted or applied.
pub(super) fn inspection_yaml(source: &PackageSource) -> Result<String, CommandErrorDto> {
    let mut value: Value = serde_saphyr::from_str(&source.config_yaml)
        .map_err(|_| invalid("config.yaml is invalid."))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("config.yaml must be a mapping."))?;
    if object
        .keys()
        .any(|key| !["schemaVersion", "providers", "models"].contains(&key.as_str()))
    {
        return Err(invalid(
            "Setup config supports schemaVersion, providers, and models only.",
        ));
    }
    let providers = value
        .pointer("/providers/profiles")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("Setup config needs provider profiles."))?;
    if providers.keys().collect::<BTreeSet<_>>() != source.manifest.providers.keys().collect() {
        return Err(invalid(
            "Manifest provider IDs must exactly match config.yaml provider profiles.",
        ));
    }
    let mut origins = BTreeSet::new();
    for provider in providers.values() {
        let object = provider
            .as_object()
            .ok_or_else(|| invalid("A provider profile is invalid."))?;
        if object.keys().any(|key| {
            !["kind", "baseUrl", "credentialReference", "timeoutMs"].contains(&key.as_str())
        }) {
            return Err(invalid(
                "A provider field is not supported by Desktop setup.",
            ));
        }
        let kind = provider
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(
            kind,
            "open_ai_compatible"
                | "openai_compatible"
                | "open_ai_responses"
                | "openai_responses"
                | "open_ai_codex"
        ) {
            return Err(invalid("Choose a supported Desktop provider protocol."));
        }
        let reference = provider.get("credentialReference").and_then(Value::as_str);
        if kind == "open_ai_codex" {
            if reference != Some("codex:default")
                || provider.get("baseUrl").is_some_and(|v| !v.is_null())
            {
                return Err(invalid(
                    "Codex requires codex:default and its fixed service endpoint.",
                ));
            }
            origins.extend([
                "https://chatgpt.com".to_owned(),
                "https://auth.openai.com".to_owned(),
            ]);
        } else {
            if reference.is_some_and(|r| !valid_slot(r)) {
                return Err(invalid(
                    "Portable credentials must be env:NAME placeholders; never embed keys or host IDs.",
                ));
            }
            let url = provider
                .get("baseUrl")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("A provider URL is required."))?;
            crate::provider_catalog::validate_setup_endpoint(ProviderKindSetting::Compatible, url)?;
            origins.insert(
                url::Url::parse(url)
                    .map_err(|_| invalid("A provider URL is invalid."))?
                    .origin()
                    .ascii_serialization(),
            );
        }
    }
    value["sandbox"] = json!({"networkDestinations": origins});
    value["storage"] = json!({"adapter":"ephemeral","path":"setup-validation.redb"});
    // Provider-only packages are valid. A private echo route lets the canonical parser
    // inspect their connections without inventing a remote model or making requests.
    if value.get("models").is_none() || value["models"] == json!({"profiles":{},"roles":{}}) {
        let mut validation_id = "__setup_validation".to_owned();
        while source.manifest.providers.contains_key(&validation_id) {
            validation_id.push('_');
        }
        value["providers"]["profiles"][&validation_id] = json!({"kind":"echo"});
        value["models"] = json!({
            "profiles": {&validation_id: {"providerProfile":validation_id, "model":"echo",
                "contextWindowTokens":32768,"maxOutputTokens":4096,"capabilities":{"toolCalls":false,"streaming":false}}},
            "roles":{"primary":validation_id}
        });
    }
    serde_json::to_string(&value)
        .map_err(|_| invalid("Setup configuration could not be inspected."))
}

fn valid_slot(reference: &str) -> bool {
    reference.strip_prefix("env:").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 128
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !name.as_bytes()[0].is_ascii_digit()
    })
}

pub(super) fn inspected(
    source: PackageSource,
    canonical: &Value,
    bytes: &[u8],
) -> Result<SavedSetupPackage, CommandErrorDto> {
    let mut providers = Vec::new();
    for profile in source.manifest.providers.keys() {
        let value = canonical
            .pointer("/providers/profiles")
            .and_then(|v| v.get(profile))
            .ok_or_else(|| invalid("A provider could not be inspected."))?;
        let kind = match value["kind"].as_str() {
            Some("open_ai_compatible" | "openai_compatible") => ProviderKindSetting::Compatible,
            Some("open_ai_responses" | "openai_responses") => ProviderKindSetting::Responses,
            Some("open_ai_codex") => ProviderKindSetting::Codex,
            _ => return Err(invalid("Unsupported provider protocol.")),
        };
        let connection = ProviderSetting {
            credential_required: value["credentialReference"]
                .as_str()
                .is_some_and(|v| v.starts_with("env:")),
            profile: profile.clone(),
            kind,
            base_url: if kind == ProviderKindSetting::Codex {
                CODEX_BASE_URL.into()
            } else {
                value["baseUrl"].as_str().unwrap_or_default().into()
            },
            credential_id: None,
            timeout_ms: value["timeoutMs"].as_u64(),
        };
        if !crate::desktop_settings::managed_provider_setting_is_valid(&connection) {
            return Err(invalid(
                "A provider is unsupported by this Desktop version.",
            ));
        }
        providers.push(SetupProvider {
            credential_slot: value["credentialReference"]
                .as_str()
                .filter(|v| v.starts_with("env:"))
                .map(str::to_owned),
            connection,
        });
    }
    let ids = providers
        .iter()
        .map(|p| p.connection.profile.clone())
        .collect::<BTreeSet<_>>();
    let mut models = Vec::new();
    if let Some(profiles) = canonical
        .pointer("/models/profiles")
        .and_then(Value::as_object)
    {
        for (profile, value) in profiles {
            if !ids.contains(value["providerProfile"].as_str().unwrap_or_default()) {
                continue;
            }
            let mut value = value.clone();
            value["profile"] = json!(profile);
            let model: ModelSetting = serde_json::from_value(value)
                .map_err(|_| invalid("A model field is unsupported by Desktop."))?;
            if !crate::desktop_settings::managed_model_setting_is_valid(
                &model,
                &ids.iter().map(String::as_str).collect(),
            ) {
                return Err(invalid("A model is unsupported by this Desktop version."));
            }
            models.push(model);
        }
    }
    if models.len() > 64 {
        return Err(invalid("A setup package supports at most 64 models."));
    }
    let roles = canonical
        .pointer("/models/roles")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(role, target)| {
            target
                .as_str()
                .filter(|id| models.iter().any(|m| m.profile == *id))
                .map(|id| (role.clone(), id.to_owned()))
        })
        .collect();
    Ok(SavedSetupPackage {
        manifest: source.manifest,
        config_yaml: source.config_yaml,
        icons: source.icons,
        ca_pem: source.ca_pem,
        catalog_resources: None,
        sha256: hex::encode(Sha256::digest(bytes)),
        providers,
        models,
        roles,
    })
}

pub(super) fn dto(
    package: &SavedSetupPackage,
    settings: &DesktopSettings,
) -> Result<PackageDto, CommandErrorDto> {
    Ok(PackageDto {
        id: package.manifest.id.clone(),
        name: package.manifest.name.clone(),
        version: package.manifest.version.clone(),
        sha256: package.sha256.clone(),
        description_markdown: package.manifest.description_markdown.clone(),
        roles: package.roles.clone(),
        certificate_fingerprints: package
            .ca_pem
            .as_deref()
            .map(super::archive::validate_ca)
            .transpose()?
            .map_or_else(Vec::new, |ca| ca.fingerprints_sha256()),
        existing_certificate_fingerprints: settings
            .additional_ca_bundle
            .as_ref()
            .map_or_else(Vec::new, |ca| ca.fingerprints_sha256.clone()),
        replaces_version: settings
            .setup_packages
            .iter()
            .find(|p| p.manifest.id == package.manifest.id)
            .map(|p| p.manifest.version.clone()),
        providers: package
            .providers
            .iter()
            .map(|p| {
                let presentation = &package.manifest.providers[&p.connection.profile];
                ProviderDto {
                    catalog_resource_id: package
                        .catalog_resources
                        .as_ref()
                        .and_then(|ids| ids.get(&format!("provider:{}", p.connection.profile)))
                        .cloned(),
                    profile: p.connection.profile.clone(),
                    display_name: presentation.display_name.clone(),
                    description_markdown: presentation.description_markdown.clone(),
                    kind: p.connection.kind,
                    base_url: p.connection.base_url.clone(),
                    timeout_ms: p.connection.timeout_ms,
                    credential_required: p.credential_slot.is_some(),
                    credential_id: p.connection.credential_id.clone().filter(|id| {
                        settings
                            .global_configuration
                            .credentials
                            .iter()
                            .any(|c| &c.id == id)
                    }),
                    icon: presentation
                        .icon
                        .as_ref()
                        .and_then(|path| package.icons.get(path))
                        .cloned(),
                    dark_icon: presentation
                        .dark_icon
                        .as_ref()
                        .and_then(|path| package.icons.get(path))
                        .cloned(),
                    models: package
                        .models
                        .iter()
                        .filter(|m| m.provider_profile == p.connection.profile)
                        .cloned()
                        .collect(),
                }
            })
            .collect(),
    })
}

pub(super) fn preserve_credentials(next: &mut SavedSetupPackage, previous: &SavedSetupPackage) {
    for provider in &mut next.providers {
        if let Some(old) = previous.providers.iter().find(|p| {
            p.connection.profile == provider.connection.profile
                && p.connection.kind == provider.connection.kind
                && p.connection.base_url == provider.connection.base_url
                && p.credential_slot == provider.credential_slot
        }) {
            provider
                .connection
                .credential_id
                .clone_from(&old.connection.credential_id);
        }
    }
}

/// Validate persisted bounds before a saved package can reach renderer DTOs or export.
pub(crate) fn validate_saved(packages: &[SavedSetupPackage]) -> Result<(), CommandErrorDto> {
    if packages.len() > 4 {
        return Err(invalid(
            "Desktop supports at most four saved setup packages.",
        ));
    }
    let mut ids = BTreeSet::new();
    for package in packages {
        if !ids.insert(&package.manifest.id)
            || package.sha256.len() != 64
            || !package.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("Saved setup package metadata is invalid."));
        }
        validate_saved_profiles(package)?;
        super::catalog::validate_resources(package)?;
        // Reuse the same package reader to enforce metadata, asset, and certificate bounds.
        let source = super::archive::read(&super::archive::write(package)?)?;
        inspection_yaml(&source)?;
    }
    Ok(())
}

fn validate_saved_profiles(package: &SavedSetupPackage) -> Result<(), CommandErrorDto> {
    let provider_ids = package
        .providers
        .iter()
        .map(|p| p.connection.profile.as_str())
        .collect::<BTreeSet<_>>();
    let model_ids = package
        .models
        .iter()
        .map(|m| m.profile.as_str())
        .collect::<BTreeSet<_>>();
    if provider_ids.len() != package.providers.len()
        || provider_ids
            != package
                .manifest
                .providers
                .keys()
                .map(String::as_str)
                .collect()
        || model_ids.len() != package.models.len()
        || package.models.len() > 64
        || package.providers.iter().any(|p| {
            !crate::desktop_settings::managed_provider_setting_is_valid(&p.connection)
                || p.credential_slot
                    .as_deref()
                    .is_some_and(|slot| !valid_slot(slot))
        })
        || package
            .models
            .iter()
            .any(|m| !crate::desktop_settings::managed_model_setting_is_valid(m, &provider_ids))
        || package.roles.iter().any(|(role, target)| {
            !super::types::valid_id(role) || !model_ids.contains(target.as_str())
        })
    {
        return Err(invalid(
            "Saved setup provider or model metadata is invalid.",
        ));
    }
    Ok(())
}
