use super::{
    archive, catalog, configuration,
    types::{PackageDto, SavedSetupPackage, invalid},
};
use crate::{
    desktop_commands::{connect_guard, credential_parent, settings_store},
    desktop_dto::{
        ApplyManagedModelConfigurationInput, CredentialActionInput, ManagedModelInput,
        ManagedProviderInput,
    },
    desktop_settings::{DesktopSettings, ProviderKindSetting},
    dto::CommandErrorDto,
    provider_enrollment::DialogAppearanceInput,
    state::AppState,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    sync::Mutex,
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt as _;

#[derive(Default)]
pub(crate) struct SetupReviewState(pub(super) Mutex<Option<Review>>);

impl SetupReviewState {
    pub(super) fn cancel(&self, sha256: &str) -> Result<(), CommandErrorDto> {
        let mut pending = self
            .0
            .lock()
            .map_err(|_| invalid("Setup review is unavailable."))?;
        if pending
            .as_ref()
            .is_some_and(|review| review.package.sha256 == sha256)
        {
            *pending = None;
        }
        Ok(())
    }

    pub(super) fn instructions(
        &self,
        settings: &DesktopSettings,
        id: &str,
    ) -> Result<SavedSetupPackage, CommandErrorDto> {
        let pending = self
            .0
            .lock()
            .map_err(|_| invalid("Setup review is unavailable."))?;
        pending
            .as_ref()
            .map(|review| &review.package)
            .filter(|package| package.manifest.id == id)
            .or_else(|| {
                settings
                    .setup_packages
                    .iter()
                    .find(|package| package.manifest.id == id)
            })
            .cloned()
            .ok_or_else(|| invalid("The setup instructions are unavailable."))
    }
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri command arguments are owned.
pub(crate) fn cancel_setup_package_review(
    reviews: State<'_, SetupReviewState>,
    sha256: String,
) -> Result<(), CommandErrorDto> {
    reviews.cancel(&sha256)
}

pub(super) struct Review {
    pub(super) package: SavedSetupPackage,
    pub(super) previous_sha256: Option<String>,
    pub(super) certificate_fingerprints: Vec<String>,
    pub(super) global_revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackageIdentity {
    id: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplyPackageInput {
    sha256: String,
    trust_certificates: bool,
    replace_existing: bool,
    #[serde(default)]
    apply_defaults: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CredentialInput {
    id: String,
    sha256: String,
    profile: String,
    #[serde(default)]
    credential_id: Option<String>,
}

fn find_package<'a>(
    settings: &'a DesktopSettings,
    id: &str,
    sha256: &str,
) -> Result<&'a SavedSetupPackage, CommandErrorDto> {
    settings
        .setup_packages
        .iter()
        .find(|p| p.manifest.id == id && p.sha256 == sha256)
        .ok_or_else(|| invalid("The saved setup package changed. Refresh and retry."))
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri command arguments are owned.
pub(crate) fn list_setup_packages(
    state: State<'_, AppState>,
) -> Result<Vec<PackageDto>, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let settings = settings_store()?.load()?;
    settings
        .setup_packages
        .iter()
        .map(|p| configuration::dto(p, &settings))
        .collect()
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn inspect_setup_package(
    app: AppHandle,
    state: State<'_, AppState>,
    reviews: State<'_, SetupReviewState>,
    package_id: Option<String>,
) -> Result<Option<PackageDto>, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    *reviews
        .0
        .lock()
        .map_err(|_| invalid("Setup review is unavailable."))? = None;
    let settings = settings_store()?.load()?;
    let package = if let Some(id) = package_id {
        settings
            .setup_packages
            .iter()
            .find(|p| p.manifest.id == id)
            .cloned()
            .ok_or_else(|| invalid("The saved setup package is unavailable."))?
    } else {
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("Colossus Desktop setup", &["colossus-setup", "zip"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = file
            .into_path()
            .map_err(|_| invalid("The selected setup file is unavailable."))?;
        let file = File::open(path)
            .map_err(|_| invalid("The selected setup file could not be opened."))?;
        let metadata = file
            .metadata()
            .map_err(|_| invalid("The selected setup file is unavailable."))?;
        if !metadata.is_file() || metadata.len() > archive::MAX_ARCHIVE_BYTES as u64 {
            return Err(invalid("Choose a regular setup file no larger than 2 MiB."));
        }
        let mut bytes = Vec::new();
        file.take(archive::MAX_ARCHIVE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("The setup file could not be read."))?;
        let source = archive::read(&bytes)?;
        let yaml = configuration::inspection_yaml(&source)?;
        let bundle = crate::bundle::VerifiedBundle::load()?;
        let inspected = colossus_sdk::inspect_sidecar_configuration(&bundle.sidecar, yaml)
            .await
            .map_err(CommandErrorDto::from_sdk)?;
        let canonical = inspected.canonical_config.ok_or_else(|| {
            invalid("Provider/model YAML failed Colossus configuration validation.")
        })?;
        configuration::inspected(source, &canonical, &bytes)?
    };
    let dto = configuration::dto(&package, &settings)?;
    *reviews
        .0
        .lock()
        .map_err(|_| invalid("Setup review is unavailable."))? = Some(Review {
        previous_sha256: settings
            .setup_packages
            .iter()
            .find(|p| p.manifest.id == package.manifest.id)
            .map(|p| p.sha256.clone()),
        certificate_fingerprints: dto.existing_certificate_fingerprints.clone(),
        global_revision: settings.global_configuration.revision,
        package,
    });
    Ok(Some(dto))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn apply_setup_package(
    state: State<'_, AppState>,
    reviews: State<'_, SetupReviewState>,
    request: ApplyPackageInput,
) -> Result<(), CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let review = reviews
        .0
        .lock()
        .map_err(|_| invalid("Setup review is unavailable."))?
        .as_ref()
        .map(|r| {
            (
                r.package.clone(),
                r.previous_sha256.clone(),
                r.certificate_fingerprints.clone(),
                r.global_revision,
            )
        })
        .ok_or_else(|| invalid("Inspect a setup package before applying it."))?;
    let (mut package, previous_sha256, fingerprints, global_revision) = review;
    if package.sha256 != request.sha256 {
        return Err(invalid("The setup review changed. Inspect it again."));
    }
    let store = settings_store()?;
    let mut settings = store.load()?;
    if request.apply_defaults && settings.global_configuration.revision != global_revision {
        return Err(invalid(
            "Global settings changed. Review the setup package again before applying defaults.",
        ));
    }
    let old = settings
        .setup_packages
        .iter()
        .find(|p| p.manifest.id == package.manifest.id);
    if old.map(|p| &p.sha256) != previous_sha256.as_ref()
        || (old.is_some() && !request.replace_existing)
    {
        return Err(invalid(
            "Review and confirm replacement of the existing setup package.",
        ));
    }
    if let Some(old) = old {
        configuration::preserve_credentials(&mut package, old);
    }
    let old = old.cloned();
    let previous = settings.clone();
    catalog::import_catalog(&mut settings, &mut package, old.as_ref())?;
    if request.apply_defaults {
        catalog::apply_defaults(&mut settings, &package)?;
    }
    settings
        .setup_packages
        .retain(|p| p.manifest.id != package.manifest.id);
    settings.setup_packages.push(package.clone());
    let mut staged_ca = None;
    if request.trust_certificates {
        if settings
            .additional_ca_bundle
            .as_ref()
            .map_or_else(Vec::new, |ca| ca.fingerprints_sha256.clone())
            != fingerprints
        {
            return Err(invalid(
                "The trusted certificates changed. Inspect the package again.",
            ));
        }
        crate::desktop_commands::reject_active_managed_runs(&state).await?;
        let pem = package
            .ca_pem
            .as_deref()
            .ok_or_else(|| invalid("This package does not include CA certificates."))?;
        let staged = store.stage_ca_bundle_bytes(pem.as_bytes())?;
        settings.additional_ca_bundle = Some(staged.clone());
        staged_ca = Some(staged);
    }
    if let Err(error) = store.save(&settings) {
        if let Some(ca) = &staged_ca {
            let _ = store.delete_ca_bundle(ca);
        }
        return Err(error);
    }
    if staged_ca.is_some()
        && crate::desktop_commands::has_managed_configuration(&settings)
        && let Err(error) = crate::managed_runtime::start(&state, &store, &settings, true).await
    {
        store.save(&previous)?;
        if let Some(ca) = &staged_ca {
            let _ = store.delete_ca_bundle(ca);
        }
        crate::desktop_commands::restore_managed_after_rollback(&state, &store, &previous).await?;
        return Err(error);
    }
    if staged_ca.is_some()
        && let Some(ca) = previous.additional_ca_bundle
    {
        store.delete_ca_bundle(&ca)?;
    }
    *reviews
        .0
        .lock()
        .map_err(|_| invalid("Setup review is unavailable."))? = None;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn configure_setup_credential(
    app: AppHandle,
    state: State<'_, AppState>,
    request: CredentialInput,
    appearance: DialogAppearanceInput,
) -> Result<(), CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let package = find_package(&settings, &request.id, &request.sha256)?;
    let provider = package
        .providers
        .iter()
        .find(|p| p.connection.profile == request.profile)
        .ok_or_else(|| invalid("The setup provider is unknown."))?;
    if provider.credential_slot.is_none() {
        return Err(invalid("This provider does not use an API key."));
    }
    let label = package.manifest.providers[&request.profile]
        .display_name
        .clone();
    let previous = package.clone();
    let credential_id = if let Some(id) = request.credential_id {
        crate::provider_catalog::validate_provider_credential(&settings, &id)?;
        id
    } else {
        crate::managed_configuration_commands::enroll_credential(
            credential_parent(&app)?,
            &state,
            &store,
            &mut settings,
            &label,
            crate::managed_configuration::CredentialKindSetting::ApiKey,
            appearance,
        )
        .await?
    };
    let package = settings
        .setup_packages
        .iter_mut()
        .find(|p| p.manifest.id == request.id)
        .ok_or_else(|| invalid("The setup package changed."))?;
    let provider = package
        .providers
        .iter_mut()
        .find(|p| p.connection.profile == request.profile)
        .ok_or_else(|| invalid("The setup provider changed."))?;
    provider.connection.credential_id = Some(credential_id);
    let mut updated = package.clone();
    catalog::update_credentials(&mut settings, &mut updated, &previous)?;
    let saved = settings
        .setup_packages
        .iter_mut()
        .find(|p| p.manifest.id == request.id)
        .ok_or_else(|| invalid("The setup package changed."))?;
    *saved = updated;
    store.save(&settings)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UseSetupModelInput {
    pub(super) id: String,
    pub(super) sha256: String,
    pub(super) profile: String,
    pub(super) model_profile: String,
    pub(super) workspace_id: String,
    pub(super) replace_conflicts: bool,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn use_setup_model(
    app: AppHandle,
    state: State<'_, AppState>,
    request: UseSetupModelInput,
    appearance: DialogAppearanceInput,
) -> Result<crate::desktop_dto::DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let settings = settings_store()?.load()?;
    let configuration = prepare_model(&settings, &request)?;
    crate::desktop_commands::apply_managed_model_configuration_locked(
        &app,
        &state,
        configuration,
        appearance,
    )
    .await
}

fn model_input(m: crate::desktop_settings::ModelSetting) -> ManagedModelInput {
    ManagedModelInput {
        profile: m.profile,
        provider_profile: m.provider_profile,
        model: m.model,
        context_window_tokens: m.context_window_tokens,
        max_output_tokens: m.max_output_tokens,
        capabilities: m.capabilities,
        reasoning_effort: m.reasoning_effort,
    }
}

pub(super) fn prepare_model(
    settings: &DesktopSettings,
    request: &UseSetupModelInput,
) -> Result<ApplyManagedModelConfigurationInput, CommandErrorDto> {
    if settings
        .workspace
        .as_ref()
        .is_none_or(|w| w.id != request.workspace_id)
    {
        return Err(invalid(
            "Choose the workspace where this model will be used.",
        ));
    }
    let package = find_package(settings, &request.id, &request.sha256)?;
    let provider = package
        .providers
        .iter()
        .find(|p| p.connection.profile == request.profile)
        .ok_or_else(|| invalid("The setup provider is unknown."))?;
    let model = package
        .models
        .iter()
        .find(|m| m.provider_profile == request.profile && m.profile == request.model_profile)
        .ok_or_else(|| invalid("Choose a model included in this package."))?;
    if provider.credential_slot.is_some()
        && provider.connection.credential_id.as_ref().is_none_or(|id| {
            !settings
                .global_configuration
                .credentials
                .iter()
                .any(|c| &c.id == id)
        })
    {
        return Err(invalid("Add an API key before using this provider."));
    }
    let imported_models = package
        .models
        .iter()
        .filter(|m| m.provider_profile == request.profile)
        .cloned()
        .collect::<Vec<_>>();
    if !request.replace_conflicts
        && (settings
            .providers
            .iter()
            .any(|p| p.profile == request.profile && p != &provider.connection)
            || settings.models.iter().any(|m| {
                imported_models
                    .iter()
                    .any(|new| new.profile == m.profile && new != m)
            }))
    {
        return Err(invalid(
            "Matching workspace profiles already exist. Review and enable replacement, or rename the profiles in your setup file.",
        ));
    }
    let mut providers = settings
        .providers
        .iter()
        .filter(|p| p.profile != request.profile)
        .cloned()
        .collect::<Vec<_>>();
    providers.push(provider.connection.clone());
    let mut models = settings
        .models
        .iter()
        .filter(|m| !imported_models.iter().any(|new| new.profile == m.profile))
        .cloned()
        .collect::<Vec<_>>();
    models.extend(imported_models);
    let mut roles = settings.model_roles.clone();
    for (role, target) in &package.roles {
        if models
            .iter()
            .any(|m| m.profile == *target && m.provider_profile == request.profile)
        {
            roles.entry(role.clone()).or_insert_with(|| target.clone());
        }
    }
    roles.insert("primary".into(), model.profile.clone());
    let configuration = ApplyManagedModelConfigurationInput {
        workspace_id: request.workspace_id.clone(),
        access_profile: settings.access_profile,
        execution_boundary: settings.execution_boundary,
        providers: providers
            .into_iter()
            .map(|p| ManagedProviderInput {
                profile: p.profile,
                provider_kind: p.kind,
                base_url: p.base_url,
                timeout_ms: p.timeout_ms,
                credential_action: if p.credential_id.is_some() {
                    CredentialActionInput::Reuse
                } else {
                    CredentialActionInput::None
                },
                credential_id: p.credential_id,
            })
            .collect(),
        models: models.into_iter().map(model_input).collect(),
        roles,
    };
    configuration.validate()?;
    Ok(configuration)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri command arguments are owned.
pub(crate) fn remove_setup_package(
    state: State<'_, AppState>,
    request: PackageIdentity,
) -> Result<(), CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    find_package(&settings, &request.id, &request.sha256)?;
    settings
        .setup_packages
        .retain(|p| p.manifest.id != request.id);
    store.save(&settings)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri command arguments are owned.
pub(crate) fn export_setup_package(
    app: AppHandle,
    state: State<'_, AppState>,
    package_id: Option<String>,
) -> Result<bool, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let settings = store.load()?;
    let package = if let Some(id) = package_id {
        settings
            .setup_packages
            .iter()
            .find(|p| p.manifest.id == id)
            .cloned()
            .ok_or_else(|| invalid("The setup package is unknown."))?
    } else {
        let mut package = export_current(&settings)?;
        super::export::include_ca(&store, &settings, &mut package)?;
        package
    };
    let bytes = archive::write(&package)?;
    archive::read(&bytes)?;
    let Some(file) = app
        .dialog()
        .file()
        .set_file_name(format!("{}.colossus-setup", package.manifest.id))
        .add_filter("Colossus Desktop setup", &["colossus-setup"])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = file
        .into_path()
        .map_err(|_| invalid("The export destination is unavailable."))?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("The export destination is unavailable."))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| invalid("The setup package could not be exported."))?;
    staged
        .write_all(&bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|_| invalid("The setup package could not be exported."))?;
    staged
        .persist(path)
        .map_err(|_| invalid("The setup package could not be exported."))?;
    Ok(true)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SetupLinkInput {
    id: String,
    #[serde(default)]
    provider_resource_id: Option<String>,
    url: String,
}

/// Instruction links are opened only on a human click and only when the exact address
/// belongs to the saved/reviewed package. Guests receive no Desktop capabilities.
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn open_setup_link(
    app: AppHandle,
    state: State<'_, AppState>,
    reviews: State<'_, SetupReviewState>,
    request: SetupLinkInput,
) -> Result<(), CommandErrorDto> {
    use tauri::Manager as _;
    let _guard = connect_guard(&state)?;
    let settings = settings_store()?.load()?;
    let belongs_to_instructions = if let Some(id) = &request.provider_resource_id {
        super::provider_presentation(&settings, id)
            .is_some_and(|value| value.description_markdown.contains(&request.url))
    } else {
        let package = reviews.instructions(&settings, &request.id)?;
        package.manifest.description_markdown.contains(&request.url)
            || package
                .manifest
                .providers
                .values()
                .any(|p| p.description_markdown.contains(&request.url))
    };
    let url =
        url::Url::parse(&request.url).map_err(|_| invalid("The instruction link is invalid."))?;
    if request.url.len() > 2048
        || !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || !belongs_to_instructions
    {
        return Err(invalid(
            "Choose an HTTP(S) link from the setup instructions.",
        ));
    }
    let view = app
        .get_webview("main")
        .ok_or_else(|| invalid("The Desktop window is unavailable."))?;
    colossus_native_browser::open_external(&view, &request.url)
        .await
        .map_err(|_| invalid("The instruction link could not be opened."))
}

pub(super) fn export_current(
    settings: &DesktopSettings,
) -> Result<SavedSetupPackage, CommandErrorDto> {
    use super::types::{Manifest, Presentation, SetupProvider};
    use serde_json::json;
    let snapshot = super::export::snapshot(settings)?;
    let mut profiles = serde_json::Map::new();
    let mut presentation = BTreeMap::new();
    let mut providers = Vec::new();
    for (i, provider) in snapshot.providers.iter().enumerate() {
        let slot = (provider.credential_required || provider.credential_id.is_some())
            .then(|| format!("env:COLOSSUS_PROVIDER_{}_TOKEN", i + 1));
        let kind = match provider.kind {
            ProviderKindSetting::Compatible => "open_ai_compatible",
            ProviderKindSetting::Responses => "open_ai_responses",
            ProviderKindSetting::Codex => "open_ai_codex",
        };
        let reference = if provider.kind == ProviderKindSetting::Codex {
            Some("codex:default".to_owned())
        } else {
            slot.clone()
        };
        let url =
            (provider.kind != ProviderKindSetting::Codex).then_some(provider.base_url.clone());
        profiles.insert(provider.profile.clone(), json!({"kind":kind,"baseUrl":url,"credentialReference":reference,"timeoutMs":provider.timeout_ms}));
        presentation.insert(
            provider.profile.clone(),
            Presentation {
                display_name: provider.profile.clone(),
                description_markdown: String::new(),
                icon: None,
                dark_icon: None,
            },
        );
        let mut connection = provider.clone();
        connection.credential_id = None;
        connection.credential_required = slot.is_some();
        providers.push(SetupProvider {
            connection,
            credential_slot: slot,
        });
    }
    let models = snapshot
        .models
        .iter()
        .map(|model| {
            let mut value = serde_json::to_value(model)
                .map_err(|_| invalid("A model could not be exported."))?;
            value
                .as_object_mut()
                .ok_or_else(|| invalid("A model could not be exported."))?
                .remove("profile");
            Ok((model.profile.clone(), value))
        })
        .collect::<Result<BTreeMap<_, _>, CommandErrorDto>>()?;
    let config = json!({"schemaVersion":3,"providers":{"profiles":profiles},"models":{"profiles":models,"roles":snapshot.model_roles}});
    super::export::complete(
        settings,
        SavedSetupPackage {
            manifest: Manifest {
                schema_version: 1,
                id: "desktop-setup".into(),
                name: "Desktop setup".into(),
                version: "1".into(),
                description_markdown: String::new(),
                providers: presentation,
                ca_bundle: None,
            },
            config_yaml: serde_saphyr::to_string(&config)
                .map_err(|_| invalid("Configuration could not be exported."))?,
            sha256: String::new(),
            providers,
            models: snapshot.models,
            roles: snapshot.model_roles,
            icons: BTreeMap::new(),
            ca_pem: None,
            catalog_resources: None,
            credential_bindings: BTreeMap::new(),
        },
    )
}
