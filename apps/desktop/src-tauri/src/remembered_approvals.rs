//! Desktop-owned consent for an exact argument vector and working directory.
//! Matches are native-only, workspace-identity-bound, and still consume a fresh
//! one-use approval through the broker and ordinary runtime policy/permit path.

use colossus_sdk::{
    ApprovalInteraction, IdempotencyKey, Interaction, InteractionAnswer, InteractionContent,
    InteractionStatus, RespondInteractionRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tauri::State;

use crate::{
    desktop_settings::{DesktopSettings, SettingsStore},
    dto::CommandErrorDto,
    state::{AppState, TargetConsentContext, TargetHandle},
};

const MAX_ALLOWANCES: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommandAllowance {
    space_id: String,
    fingerprint: String,
}

pub(crate) fn validate_saved(rules: &[CommandAllowance]) -> Result<(), CommandErrorDto> {
    if rules.len() > MAX_ALLOWANCES
        || rules.iter().any(|rule| {
            rule.space_id.is_empty()
                || rule.space_id.len() > 128
                || rule.fingerprint.len() != 64
                || !rule
                    .fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(unavailable());
    }
    Ok(())
}

fn unavailable() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "approval",
        "This command cannot be remembered. Use Allow once.",
    )
}

pub(crate) fn allowance(
    settings: &DesktopSettings,
    target_id: &str,
    approval: &ApprovalInteraction,
) -> Option<CommandAllowance> {
    let command_key = crate::approval_adapter::command_key(approval)?;
    if settings.selected_space_id.as_deref() != Some(target_id) {
        return None;
    }
    let space = settings
        .spaces
        .iter()
        .find(|space| space.id == target_id && !space.archived)?;
    let identity = space.workspace.identity.as_ref()?;
    // No prefix, regex, shell normalization, or agent justification participates.
    // Settings changes and replacement directories cannot inherit earlier consent.
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version": 1, "workspace": identity, "workspacePath": space.workspace.path,
        "access": space.access_profile, "boundary": space.execution_boundary,
        "configuration": space.configuration, "globalRevision": settings.global_configuration.revision,
        "command": command_key,
    })).ok()?;
    Some(CommandAllowance {
        space_id: target_id.into(),
        fingerprint: hex::encode(Sha256::digest(bytes)),
    })
}

pub(crate) fn can_remember(
    target: &TargetHandle,
    target_id: &str,
    approval: &ApprovalInteraction,
) -> bool {
    target.consent == TargetConsentContext::ManagedLocal
        && SettingsStore::open_application().ok().is_some_and(|store| {
            store
                .command_allowances()
                .is_ok_and(|rules| rules.len() < MAX_ALLOWANCES)
                && store
                    .load()
                    .ok()
                    .is_some_and(|settings| allowance(&settings, target_id, approval).is_some())
        })
}

/// Recheck the configuration captured before opening native command review.
pub(crate) fn revalidate_reviewed_allowance(
    settings: &DesktopSettings,
    target_id: &str,
    approval: &ApprovalInteraction,
    reviewed: Option<&CommandAllowance>,
) -> Result<CommandAllowance, CommandErrorDto> {
    let current = allowance(settings, target_id, approval);
    match (reviewed, current) {
        (Some(reviewed), Some(current)) if *reviewed == current => Ok(current),
        _ => Err(CommandErrorDto::invalid(
            "approval",
            "The workspace configuration changed. Review the command again.",
        )),
    }
}

/// The caller holds the native settings guard through validation and persistence.
pub(crate) fn remember(
    store: &SettingsStore,
    rule: CommandAllowance,
) -> Result<(), CommandErrorDto> {
    let mut rules = store.command_allowances()?;
    if !rules.contains(&rule) {
        rules.push(rule);
        store.save_command_allowances(&rules)?;
    }
    Ok(())
}

/// Native watch/hydration may satisfy only a saved exact command. Errors leave
/// the interaction available for manual review; no retry can replay an effect.
pub(crate) async fn apply_saved(
    state: &AppState,
    target: &TargetHandle,
    target_id: &str,
    interaction: &Interaction,
) -> Option<Interaction> {
    if target.consent != TargetConsentContext::ManagedLocal
        || interaction.status != InteractionStatus::Pending
        || !interaction.respondable_by_caller
    {
        return None;
    }
    let InteractionContent::Approval(approval) = &interaction.content else {
        return None;
    };
    // A just-accepted Always allow may still be saving while the next tool's
    // interaction arrives. Briefly serialize with that response before reading.
    let _approval_guard =
        tokio::time::timeout(std::time::Duration::from_secs(2), state.approval_guard())
            .await
            .ok()?;
    // Serialize with clearing remembered consent and changes to native settings.
    let _settings_guard = crate::desktop_commands::connect_guard(state).ok()?;
    let store = SettingsStore::open_application().ok()?;
    let settings = store.load().ok()?;
    let rule = allowance(&settings, target_id, approval)?;
    if !store.command_allowances().ok()?.contains(&rule) {
        return None;
    }
    let request = RespondInteractionRequest {
        run_id: interaction.run_id.clone(),
        interaction_id: interaction.interaction_id.clone(),
        etag: interaction.etag.clone(),
        idempotency_key: IdempotencyKey::new(format!("remembered-{}", interaction.interaction_id))
            .ok()?,
        response: InteractionAnswer::Approval {
            approved: true,
            request_hash: approval.request_hash.clone(),
        },
    };
    crate::approval_adapter::answer_remembered(
        &target.client,
        request,
        &crate::approval_adapter::command_key(approval)?,
    )
    .await
    .ok()
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn remembered_command_count(space_id: &str) -> Result<usize, CommandErrorDto> {
    let rules = SettingsStore::open_application()?.command_allowances()?;
    Ok(rules
        .iter()
        .filter(|rule| rule.space_id == space_id)
        .count())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri injects its state and owned arguments.
pub(crate) fn clear_remembered_commands(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<(), CommandErrorDto> {
    let _guard = crate::desktop_commands::connect_guard(&state)?;
    let store = SettingsStore::open_application()?;
    let mut rules = store.command_allowances()?;
    rules.retain(|rule| rule.space_id != space_id);
    store.save_command_allowances(&rules)
}

#[cfg(test)]
mod tests;
