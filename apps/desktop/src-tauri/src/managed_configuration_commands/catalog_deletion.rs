//! Deletion of unused provider and model catalog entries, including their revisions.

use super::{
    AppState, CommandErrorDto, DesktopSettings, ManagedSettingsSnapshotDto, State,
    advance_unaffected_spaces, bump_global_revision, connect_guard, current_value,
    ensure_global_revision, settings_store, snapshot,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteGlobalCatalogEntryInput {
    expected_revision: u64,
    resource_id: String,
}

#[derive(Clone, Copy, Debug)]
enum CatalogKind {
    Model,
    Provider,
}

impl CatalogKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Provider => "provider",
        }
    }

    const fn prefix(self) -> &'static str {
        match self {
            Self::Model => "model:",
            Self::Provider => "provider:",
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn delete_global_model(
    state: State<'_, AppState>,
    request: DeleteGlobalCatalogEntryInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    delete_entry(state, request, CatalogKind::Model).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn delete_global_provider(
    state: State<'_, AppState>,
    request: DeleteGlobalCatalogEntryInput,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    delete_entry(state, request, CatalogKind::Provider).await
}

async fn delete_entry(
    state: State<'_, AppState>,
    request: DeleteGlobalCatalogEntryInput,
    kind: CatalogKind,
) -> Result<ManagedSettingsSnapshotDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    apply_catalog_deletion(&mut settings, &request, kind)?;
    store.save(&settings)?;
    snapshot(state.inner(), &settings).await
}

fn apply_catalog_deletion(
    settings: &mut DesktopSettings,
    request: &DeleteGlobalCatalogEntryInput,
    kind: CatalogKind,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    let global = &settings.global_configuration;
    let exists = match kind {
        CatalogKind::Model => global
            .models
            .iter()
            .any(|entry| entry.id == request.resource_id),
        CatalogKind::Provider => global
            .providers
            .iter()
            .any(|entry| entry.id == request.resource_id),
    };
    if !exists {
        return Err(CommandErrorDto::invalid(
            "resourceId",
            &format!("The {} is unknown.", kind.name()),
        ));
    }
    let mut dependents = BTreeSet::new();
    let provider_profiles = global
        .providers
        .iter()
        .filter(|entry| matches!(kind, CatalogKind::Provider) && entry.id == request.resource_id)
        .flat_map(|entry| &entry.revisions)
        .map(|revision| revision.value.profile.as_str())
        .collect::<BTreeSet<_>>();
    // Current model definitions must remain usable even before a workspace selects them.
    for entry in global.models.iter().filter(|entry| !entry.archived) {
        if current_value(entry)
            .is_some_and(|model| provider_profiles.contains(model.provider_profile.as_str()))
        {
            dependents.insert(format!("Model {}", entry.label));
        }
    }
    for space in &settings.spaces {
        let referenced = space
            .configuration
            .catalog_revisions
            .iter()
            .any(|(key, reference)| {
                if key.starts_with(kind.prefix()) && reference.resource_id == request.resource_id {
                    return true;
                }
                // A workspace may still use an older model revision after its provider changes.
                key.starts_with("model:")
                    && global
                        .models
                        .iter()
                        .find(|entry| entry.id == reference.resource_id)
                        .and_then(|entry| {
                            entry
                                .revisions
                                .iter()
                                .find(|revision| revision.revision == reference.revision)
                        })
                        .is_some_and(|revision| {
                            provider_profiles.contains(revision.value.provider_profile.as_str())
                        })
            });
        if referenced {
            dependents.insert(format!(
                "Workspace {}{}",
                space.display_name,
                if space.archived { " (archived)" } else { "" }
            ));
        }
    }
    if !dependents.is_empty() {
        return Err(CommandErrorDto::invalid(
            "resourceId",
            &format!(
                "This {} is still used by: {}. Update or remove those references and apply workspace changes before deleting it. Restore archived workspaces first.",
                kind.name(),
                dependents.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    let previous_revision = global.revision;
    // Validate the revision increment before removing any saved configuration.
    bump_global_revision(&mut settings.global_configuration)?;
    match kind {
        CatalogKind::Model => settings
            .global_configuration
            .models
            .retain(|entry| entry.id != request.resource_id),
        CatalogKind::Provider => settings
            .global_configuration
            .providers
            .retain(|entry| entry.id != request.resource_id),
    }
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::new());
    Ok(())
}

#[cfg(test)]
mod tests;
