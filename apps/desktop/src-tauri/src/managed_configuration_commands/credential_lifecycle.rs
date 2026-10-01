//! Stable credential identity and cleanup of references that no workspace retains.
use super::{
    CommandErrorDto, CredentialBackendSetting, DesktopSettings, bump_global_revision,
    catalog_reference_uses_credential, mcp_uses_credential, unknown_credential,
};

pub(super) fn prepare_rotation(
    settings: &mut DesktopSettings,
    id: &str,
) -> Result<(), CommandErrorDto> {
    let metadata = settings
        .global_configuration
        .credentials
        .iter_mut()
        .find(|credential| credential.id == id)
        .ok_or_else(|| unknown_credential("credentialId"))?;
    metadata.backend = CredentialBackendSetting::Desktop;
    if metadata.created_at_ms == 0 {
        metadata.created_at_ms = super::unix_time_millis();
    }
    let previous = settings.global_configuration.revision;
    bump_global_revision(&mut settings.global_configuration)?;
    // Credential identity is stable across all catalog revisions. Existing sidecars
    // retain their in-memory token until normal idle reconciliation restarts them.
    for space in &mut settings.spaces {
        let affected = space
            .providers
            .iter()
            .any(|p| p.credential_id.as_deref() == Some(id))
            || space
                .configuration
                .credential_overrides
                .iter()
                .any(|(source, target)| source == id || target == id)
            || space
                .configuration
                .catalog_revisions
                .iter()
                .any(|(key, reference)| {
                    catalog_reference_uses_credential(
                        &settings.global_configuration,
                        key,
                        reference,
                        id,
                    )
                });
        if !affected && space.configuration.accepted_global_revision == previous {
            space.configuration.accepted_global_revision = settings.global_configuration.revision;
        }
    }
    Ok(())
}

/// Protect current and pinned references before removing unused history. Saved import
/// caches are not runtime authority; clear stale bindings so a later import stays empty.
pub(super) fn prepare_deletion(
    settings: &mut DesktopSettings,
    id: &str,
) -> Result<(), CommandErrorDto> {
    let dependents = super::credential_dependents(settings, id);
    if !dependents.is_empty() {
        return Err(CommandErrorDto::invalid(
            "credentialId",
            &format!(
                "The credential is still referenced by: {}.",
                dependents.join(", ")
            ),
        ));
    }
    for entry in &mut settings.global_configuration.providers {
        entry.revisions.retain(|r| {
            r.revision == entry.current_revision || r.value.credential_id.as_deref() != Some(id)
        });
    }
    for entry in &mut settings.global_configuration.search_providers {
        entry.revisions.retain(|r| {
            r.revision == entry.current_revision || r.value.credential_id.as_deref() != Some(id)
        });
    }
    for entry in &mut settings.global_configuration.mcp_servers {
        entry
            .revisions
            .retain(|r| r.revision == entry.current_revision || !mcp_uses_credential(&r.value, id));
    }
    for package in &mut settings.setup_packages {
        package.credential_bindings.retain(|_, bound| bound != id);
        for provider in &mut package.providers {
            if provider.connection.credential_id.as_deref() == Some(id) {
                provider.connection.credential_id = None;
                provider.connection.credential_required = provider.credential_slot.is_some();
            }
        }
    }
    Ok(())
}
