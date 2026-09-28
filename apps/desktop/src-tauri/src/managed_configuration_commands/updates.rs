//! Adopt saved global revisions only at an idle native runtime boundary.
use super::{
    ManagedSettingsSnapshotDto, advance_space_revision, connect_guard, persist_and_restart,
    requires_authority_confirmation, resolved_for, settings_store, snapshot,
};
use crate::{
    desktop_settings::{DesktopSettings, SettingsStore},
    dto::CommandErrorDto,
    managed_runtime,
    state::{AppState, ManagedConfigurationDrainGuard},
};
use tauri::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingUpdate {
    Waiting,
    WaitingTerminal,
    Confirmation,
    Failed,
}

pub(crate) async fn pending_status(
    state: &AppState,
    space_id: &str,
    revision: u64,
) -> (String, String) {
    let updates = state.configuration_updates.lock().await;
    let pending = updates
        .get(space_id)
        .filter(|(saved, _)| *saved == revision);
    let (status, message) = match pending.map(|(_, status)| status) {
        Some(PendingUpdate::Waiting) => (
            "update_waiting",
            "Global changes saved. They will apply automatically when active work finishes.",
        ),
        Some(PendingUpdate::WaitingTerminal) => (
            "update_waiting",
            "Global changes saved. Close open terminal sessions to let the update apply automatically.",
        ),
        Some(PendingUpdate::Confirmation) => (
            "update_confirmation",
            "Global changes saved. Applying them requires confirmation of permission or sensitive telemetry changes.",
        ),
        Some(PendingUpdate::Failed) => (
            "update_failed",
            "Global changes are saved but could not be applied. Retry to see the error; the previous configuration is retained.",
        ),
        None => (
            "update_available",
            "Global changes saved. Checking whether they can apply automatically…",
        ),
    };
    (status.to_owned(), message.to_owned())
}

/// The renderer can request reconciliation but cannot choose a background target,
/// cancel work, or approve authority changes through this command.
#[tauri::command]
pub(crate) async fn sync_managed_configuration(
    state: State<'_, AppState>,
) -> Result<Option<ManagedSettingsSnapshotDto>, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let pending = pending_spaces(&settings);
    state
        .configuration_updates
        .lock()
        .await
        .retain(|id, (revision, _)| {
            pending.contains(id) && *revision == settings.global_configuration.revision
        });
    if pending.is_empty() {
        return Ok(None);
    }
    for id in pending {
        if state
            .configuration_updates
            .lock()
            .await
            .get(&id)
            .is_some_and(|(_, status)| {
                matches!(status, PendingUpdate::Confirmation | PendingUpdate::Failed)
            })
        {
            continue;
        }
        let outcome = sync_space(&state, &store, &mut settings, &id).await;
        let mut updates = state.configuration_updates.lock().await;
        match outcome {
            Ok(None) => {
                updates.remove(&id);
            }
            Ok(Some(pending)) => {
                updates.insert(id, (settings.global_configuration.revision, pending));
            }
            Err(_) => {
                // Do not retry a broken configuration every polling tick. Explicit
                // retry surfaces the native error; a new revision gets a new attempt.
                updates.insert(
                    id,
                    (
                        settings.global_configuration.revision,
                        PendingUpdate::Failed,
                    ),
                );
            }
        }
    }
    snapshot(state.inner(), &settings).await.map(Some)
}

fn pending_spaces(settings: &DesktopSettings) -> Vec<String> {
    settings
        .spaces
        .iter()
        .filter(|space| {
            !space.archived
                && space.configuration.accepted_global_revision
                    < settings.global_configuration.revision
        })
        .map(|space| space.id.clone())
        .collect()
}

fn candidate(
    settings: &DesktopSettings,
    space_id: &str,
) -> Result<Option<DesktopSettings>, CommandErrorDto> {
    let before = resolved_for(settings, space_id)?;
    let mut next = settings.clone();
    advance_space_revision(&mut next, space_id)?;
    let after = resolved_for(&next, space_id)?;
    if requires_authority_confirmation(&before, &after) {
        return Ok(None);
    }
    Ok(Some(next))
}

async fn sync_space(
    state: &AppState,
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    space_id: &str,
) -> Result<Option<PendingUpdate>, CommandErrorDto> {
    let Some(mut next) = candidate(settings, space_id)? else {
        return Ok(Some(PendingUpdate::Confirmation));
    };
    if state.connected(space_id).await && state.terminal_session_active() {
        return Ok(Some(PendingUpdate::WaitingTerminal));
    }
    // Close run admission before checking idleness, including in-flight creates.
    // Unlike an explicit Apply action, automatic updates never wait for or cancel a run.
    let Some(drain) = idle_guard(state, space_id, async {
        match state.target(space_id).await {
            Some(target) => managed_runtime::managed_target_has_active_work(&target.client).await,
            None => Ok(false),
        }
    })
    .await?
    else {
        return Ok(Some(PendingUpdate::Waiting));
    };
    persist_and_restart(state, store, &mut next, settings.clone(), space_id).await?;
    *settings = next;
    drop(drain);
    Ok(None)
}

async fn idle_guard(
    state: &AppState,
    space_id: &str,
    active_work: impl std::future::Future<Output = Result<bool, CommandErrorDto>>,
) -> Result<Option<ManagedConfigurationDrainGuard>, CommandErrorDto> {
    let Some(drain) = state.begin_configuration_drain_for(space_id).await else {
        return Ok(None);
    };
    if active_work.await? {
        return Ok(None);
    }
    Ok(Some(drain))
}

#[cfg(test)]
mod tests;
