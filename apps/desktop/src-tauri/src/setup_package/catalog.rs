//! Import reusable connections without activating them in any workspace.
use super::types::{SavedSetupPackage, invalid};
use crate::{
    desktop_settings::{DesktopSettings, ProviderSetting},
    dto::CommandErrorDto,
    managed_configuration::{CatalogEntrySetting, CatalogRevisionSetting, validate_configuration},
    managed_configuration_commands::{
        advance_unaffected_spaces, append_catalog_revision, bump_global_revision,
    },
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

fn current<T>(entry: &CatalogEntrySetting<T>) -> Option<&T> {
    entry
        .revisions
        .iter()
        .find(|r| r.revision == entry.current_revision)
        .map(|r| &r.value)
}

fn resources_referenced_elsewhere(
    settings: &DesktopSettings,
    package_id: &str,
) -> BTreeSet<String> {
    settings
        .setup_packages
        .iter()
        .filter(|package| package.manifest.id != package_id)
        .filter_map(|package| package.catalog_resources.as_ref())
        .flat_map(|resources| resources.values().cloned())
        .collect()
}

pub(super) fn retain_or_import<T: Clone + PartialEq>(
    entries: &mut Vec<CatalogEntrySetting<T>>,
    previous_id: Option<&String>,
    previous_value: Option<&T>,
    label: &str,
    value: &T,
    changed: &mut BTreeSet<String>,
) -> Result<String, CommandErrorDto> {
    // Only revise an entry still equal to the package's last imported definition.
    // A user's later edits, archived entries, and unrelated definitions stay intact.
    if let Some(entry) = entries.iter_mut().find(|entry| {
        !entry.archived && Some(&entry.id) == previous_id && current(entry) == previous_value
    }) {
        if current(entry) != Some(value) {
            append_catalog_revision(entry, value.clone())?;
            changed.insert(entry.id.clone());
        }
        if entry.label != label {
            label.clone_into(&mut entry.label);
            changed.insert(entry.id.clone());
        }
        return Ok(entry.id.clone());
    }
    if let Some(entry) = entries
        .iter()
        .find(|entry| !entry.archived && current(entry) == Some(value))
    {
        return Ok(entry.id.clone());
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
    changed.insert(id.clone());
    Ok(id)
}

pub(super) fn import_catalog(
    settings: &mut DesktopSettings,
    package: &mut SavedSetupPackage,
    previous: Option<&SavedSetupPackage>,
) -> Result<(), CommandErrorDto> {
    // Exact-value imports may share entries. Only this package's exclusive entries
    // may be revised; other packages must keep their original definitions.
    let shared_resources = resources_referenced_elsewhere(settings, &package.manifest.id);
    let mut staged = settings.clone();
    let previous_revision = staged.global_configuration.revision;
    let mut changed = BTreeSet::new();
    let mut resources = BTreeMap::new();
    for provider in &mut package.providers {
        provider.connection.credential_required =
            provider.credential_slot.is_some() && provider.connection.credential_id.is_none();
        let key = format!("provider:{}", provider.connection.profile);
        let old_value = previous.and_then(|old| {
            old.providers
                .iter()
                .find(|p| p.connection.profile == provider.connection.profile)
        });
        let id = retain_or_import(
            &mut staged.global_configuration.providers,
            previous
                .and_then(|old| old.catalog_resources.as_ref())
                .and_then(|ids| ids.get(&key))
                .filter(|id| !shared_resources.contains(*id)),
            old_value.map(|p| &p.connection),
            &package.manifest.providers[&provider.connection.profile].display_name,
            &provider.connection,
            &mut changed,
        )?;
        resources.insert(key, id);
    }
    for model in &package.models {
        let key = format!("model:{}", model.profile);
        let id = retain_or_import(
            &mut staged.global_configuration.models,
            previous
                .and_then(|old| old.catalog_resources.as_ref())
                .and_then(|ids| ids.get(&key))
                .filter(|id| !shared_resources.contains(*id)),
            previous.and_then(|old| old.models.iter().find(|m| m.profile == model.profile)),
            &model.model,
            model,
            &mut changed,
        )?;
        resources.insert(key, id);
    }
    super::global_catalog::import(
        &mut staged,
        package,
        previous,
        &shared_resources,
        &mut resources,
        &mut changed,
    )?;
    if !changed.is_empty() {
        bump_global_revision(&mut staged.global_configuration)?;
        advance_unaffected_spaces(&mut staged, previous_revision, &changed);
    }
    validate_configuration(&staged.global_configuration, &staged.spaces)?;
    settings.global_configuration = staged.global_configuration;
    settings.spaces = staged.spaces;
    package.catalog_resources = Some(resources);
    Ok(())
}

/// Updating a key must not re-import models or other providers deleted by the user.
pub(super) fn update_credentials(
    settings: &mut DesktopSettings,
    package: &mut SavedSetupPackage,
    previous: &SavedSetupPackage,
) -> Result<(), CommandErrorDto> {
    let Some(mut resources) = previous.catalog_resources.clone() else {
        return import_catalog(settings, package, Some(previous));
    };
    let shared_resources = resources_referenced_elsewhere(settings, &package.manifest.id);
    let mut staged = settings.clone();
    let previous_revision = staged.global_configuration.revision;
    let mut changed = BTreeSet::new();
    for provider in &mut package.providers {
        provider.connection.credential_required =
            provider.credential_slot.is_some() && provider.connection.credential_id.is_none();
        let old = previous
            .providers
            .iter()
            .find(|old| old.connection.profile == provider.connection.profile);
        if old.is_some_and(|old| old.connection == provider.connection) {
            continue;
        }
        let key = format!("provider:{}", provider.connection.profile);
        let id = retain_or_import(
            &mut staged.global_configuration.providers,
            resources
                .get(&key)
                .filter(|id| !shared_resources.contains(*id)),
            old.map(|old| &old.connection),
            &package.manifest.providers[&provider.connection.profile].display_name,
            &provider.connection,
            &mut changed,
        )?;
        resources.insert(key, id);
    }
    if !changed.is_empty() {
        bump_global_revision(&mut staged.global_configuration)?;
        advance_unaffected_spaces(&mut staged, previous_revision, &changed);
    }
    validate_configuration(&staged.global_configuration, &staged.spaces)?;
    settings.global_configuration = staged.global_configuration;
    settings.spaces = staged.spaces;
    package.catalog_resources = Some(resources);
    Ok(())
}

fn same_connection(left: &ProviderSetting, right: &ProviderSetting) -> bool {
    left.profile == right.profile
        && left.kind == right.kind
        && left.base_url == right.base_url
        && left.timeout_ms == right.timeout_ms
}

/// Promote previously saved setups once. A later catalog deletion stays deleted.
pub(crate) fn migrate_catalog(settings: &mut DesktopSettings) -> bool {
    let mut changed = false;
    for index in 0..settings.setup_packages.len() {
        if settings.setup_packages[index].catalog_resources.is_some() {
            continue;
        }
        let mut package = settings.setup_packages[index].clone();
        for provider in &mut package.providers {
            // Preserve an already configured connection to this exact destination.
            // This never obtains a credential from another endpoint or profile.
            if provider.credential_slot.is_some()
                && let Some(active) = settings
                    .providers
                    .iter()
                    .find(|p| same_connection(p, &provider.connection))
                && active.credential_id.is_some()
            {
                provider
                    .connection
                    .credential_id
                    .clone_from(&active.credential_id);
            }
        }
        // Hitting a catalog limit must not make an older Desktop profile unreadable.
        // Explicit re-import still reports the validation error to the operator.
        if import_catalog(settings, &mut package, None).is_ok() {
            settings.setup_packages[index] = package;
            changed = true;
        }
    }
    changed
}

/// Keep native enrollment and the imported inventory entry in sync before startup.
pub(crate) fn sync_configured_credentials(
    settings: &mut DesktopSettings,
) -> Result<(), CommandErrorDto> {
    for index in 0..settings.setup_packages.len() {
        let previous = settings.setup_packages[index].clone();
        let mut package = previous.clone();
        let mut changed = false;
        for provider in &mut package.providers {
            let Some(active) = settings
                .providers
                .iter_mut()
                .find(|p| same_connection(p, &provider.connection))
            else {
                continue;
            };
            if provider.credential_slot.is_some() && active.credential_id.is_some() {
                active.credential_required = false;
                if provider.connection.credential_id != active.credential_id {
                    provider
                        .connection
                        .credential_id
                        .clone_from(&active.credential_id);
                    changed = true;
                }
            }
        }
        if changed {
            update_credentials(settings, &mut package, &previous)?;
            settings.setup_packages[index] = package;
        }
    }
    Ok(())
}

pub(super) fn validate_resources(package: &SavedSetupPackage) -> Result<(), CommandErrorDto> {
    if let Some(resources) = &package.catalog_resources {
        let globals = super::globals::from_yaml(&package.config_yaml)?;
        let keys = package
            .providers
            .iter()
            .map(|p| format!("provider:{}", p.connection.profile))
            .chain(
                package
                    .models
                    .iter()
                    .map(|m| format!("model:{}", m.profile)),
            )
            .chain(globals.resource_keys())
            .collect::<BTreeSet<_>>();
        if resources.keys().cloned().collect::<BTreeSet<_>>() != keys
            || resources.values().any(|id| Uuid::parse_str(id).is_err())
        {
            return Err(invalid("Saved setup catalog references are invalid."));
        }
    }
    Ok(())
}

/// Defaults are a reviewed snapshot. Existing spaces remain pinned until their
/// normal configuration review, including native authority confirmation.
pub(super) fn apply_defaults(
    settings: &mut DesktopSettings,
    package: &SavedSetupPackage,
) -> Result<(), CommandErrorDto> {
    let defaults = super::globals::from_yaml(&package.config_yaml)?
        .defaults
        .ok_or_else(|| invalid("This setup package does not contain global defaults."))?;
    if settings.global_configuration.defaults.current() != Some(&defaults) {
        bump_global_revision(&mut settings.global_configuration)?;
        settings
            .global_configuration
            .defaults
            .revisions
            .last_mut()
            .ok_or_else(|| invalid("Global defaults are unavailable."))?
            .value = defaults;
        validate_configuration(&settings.global_configuration, &settings.spaces)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
