use super::{
    CommandErrorDto, DeleteGlobalMcpServerInput, DesktopSettings, advance_unaffected_spaces,
    bump_global_revision, ensure_global_revision,
};
use std::collections::BTreeSet;

pub(super) fn apply_mcp_deletion(
    settings: &mut DesktopSettings,
    request: &DeleteGlobalMcpServerInput,
) -> Result<(), CommandErrorDto> {
    ensure_global_revision(settings, request.expected_revision)?;
    if !settings
        .global_configuration
        .mcp_servers
        .iter()
        .any(|entry| entry.id == request.resource_id)
    {
        return Err(CommandErrorDto::invalid(
            "resourceId",
            "The MCP server is unknown.",
        ));
    }
    let dependents = settings
        .spaces
        .iter()
        .filter(|space| {
            space
                .configuration
                .catalog_revisions
                .iter()
                .any(|(key, reference)| {
                    key.starts_with("mcp:") && reference.resource_id == request.resource_id
                })
        })
        .map(|space| {
            format!(
                "{}{}",
                space.display_name,
                if space.archived { " (archived)" } else { "" },
            )
        })
        .collect::<Vec<_>>();
    if !dependents.is_empty() {
        return Err(CommandErrorDto::invalid(
            "resourceId",
            &format!(
                "Disable this MCP server and apply changes in these Workspaces before deleting it: {}. Restore archived Workspaces first.",
                dependents.join(", "),
            ),
        ));
    }

    let previous_revision = settings.global_configuration.revision;
    // Advance first so revision overflow cannot leave a partially removed entry.
    bump_global_revision(&mut settings.global_configuration)?;
    settings
        .global_configuration
        .mcp_servers
        .retain(|entry| entry.id != request.resource_id);
    advance_unaffected_spaces(settings, previous_revision, &BTreeSet::new());
    Ok(())
}

#[cfg(test)]
mod tests;
