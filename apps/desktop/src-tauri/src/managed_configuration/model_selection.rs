//! Commit explicit setup selections to the same catalog pins used by the runtime.
use super::{ensure_catalog_entry, validate_configuration};
use crate::{desktop_settings::DesktopSettings, dto::CommandErrorDto};
use std::collections::BTreeMap;

pub(crate) fn select_configured_models(
    settings: &mut DesktopSettings,
) -> Result<(), CommandErrorDto> {
    let Some(space_id) = settings.selected_space_id.clone() else {
        return Ok(());
    };
    let mut references = BTreeMap::new();
    for provider in &settings.providers {
        let reference = ensure_catalog_entry(
            &mut settings.global_configuration.providers,
            &provider.profile,
            provider,
        );
        references.insert(format!("provider:{}", reference.resource_id), reference);
    }
    for model in &settings.models {
        let reference = ensure_catalog_entry(
            &mut settings.global_configuration.models,
            &model.model,
            model,
        );
        references.insert(format!("model:{}", reference.resource_id), reference);
    }
    settings.sync_selected_space_projection()?;
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| {
            CommandErrorDto::invalid("workspaceId", "The workspace selection changed.")
        })?;
    // Keep MCP, search, telemetry, and unrelated workspace overrides intact.
    space
        .configuration
        .catalog_revisions
        .retain(|key, _| !key.starts_with("provider:") && !key.starts_with("model:"));
    space.configuration.catalog_revisions.extend(references);
    space
        .configuration
        .model_roles
        .clone_from(&settings.model_roles);
    space.configuration.access_profile_override = Some(settings.access_profile);
    space.configuration.execution_boundary_override = Some(settings.execution_boundary);
    if space.configuration.accepted_global_revision == 0 {
        space.configuration.accepted_global_revision = settings.global_configuration.revision;
    }
    validate_configuration(&settings.global_configuration, &settings.spaces)
}

pub(super) fn require_provider_credentials(
    providers: &[crate::desktop_settings::ProviderSetting],
) -> Result<(), CommandErrorDto> {
    if providers
        .iter()
        .any(|provider| provider.credential_required && provider.credential_id.is_none())
    {
        return Err(CommandErrorDto::invalid(
            "provider",
            "Add an API key to the provider before using it in a workspace.",
        ));
    }
    Ok(())
}
