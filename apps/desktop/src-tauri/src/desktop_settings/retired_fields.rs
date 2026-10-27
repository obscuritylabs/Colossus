use super::{DesktopSettings, storage_error};
use crate::{dto::CommandErrorDto, managed_configuration::FieldOverrideSetting};
use serde_json::Value;

/// Retire the removed projection setting from private persisted snapshots only.
/// New configuration and setup imports retain their ordinary strict validation.
pub(super) fn retire_removed_memory_field(
    settings: &mut DesktopSettings,
) -> Result<bool, CommandErrorDto> {
    let mut changed = false;
    for revision in &mut settings.global_configuration.defaults.revisions {
        changed |= remove_override(&mut revision.value.field_overrides);
    }
    for space in &mut settings.spaces {
        changed |= remove_override(&mut space.configuration.field_overrides);
    }
    for package in &mut settings.setup_packages {
        let mut config: Value =
            serde_saphyr::from_str(&package.config_yaml).map_err(|_| storage_error())?;
        let Some(value) = config.pointer_mut("/desktop/defaults/fieldOverrides") else {
            continue;
        };
        // Keep the existing strict entry shape, including every sibling override.
        let mut overrides: Vec<FieldOverrideSetting> =
            serde_json::from_value(value.clone()).map_err(|_| storage_error())?;
        if remove_override(&mut overrides) {
            *value = serde_json::to_value(overrides).map_err(|_| storage_error())?;
            package.config_yaml = serde_saphyr::to_string(&config).map_err(|_| storage_error())?;
            // sha256 identifies the original imported archive, not rewritten local YAML.
            changed = true;
        }
    }
    Ok(changed)
}

fn remove_override(overrides: &mut Vec<FieldOverrideSetting>) -> bool {
    let previous_len = overrides.len();
    overrides.retain(|field| field.field_id != "memory.semantic");
    previous_len != overrides.len()
}
