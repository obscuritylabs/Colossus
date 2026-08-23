use colossus_sdk::{
    ApprovalInteraction, ArtifactPurpose, GetRunRequest, IdempotencyKey, InteractionAnswer,
    InteractionContent, UploadArtifactRequest,
};
use sha2::{Digest as _, Sha256};
use std::{fs::File, io::Read as _, path::Path};
use tauri::{AppHandle, State, ipc::Channel};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::{
    desktop_settings::{AsideSetting, SettingsStore},
    dto::{
        ArtifactContentDto, ArtifactReferenceDto, AsideDto, CancelRunInput, CommandErrorDto,
        CreateRunInput, GetRunDto, GetRunInput, InteractionDto, ListAsidesInput, ListRunsDto,
        ListRunsInput, ListSessionActivityDto, ListSessionActivityInput, RespondInteractionInput,
        RunDto, ThreadLifecycleDto, ThreadLifecycleInput, WatchEventDto, WatchRunInput,
    },
    run_list, space_search,
    state::{AppState, SelectedTargetLease, TargetConsentContext, TargetHandle},
};

const MAX_NATIVE_APPROVAL_REASON_BYTES: usize = 4 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 16 * 1_048_576;
const MAX_RENDERED_ARTIFACT_BYTES: usize = 1_048_576;

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn choose_run_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
) -> Result<Option<ArtifactReferenceDto>, CommandErrorDto> {
    let selected = app.dialog().file().blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|_| {
        CommandErrorDto::invalid("attachment", "The selected attachment is unavailable.")
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.len() <= 255)
        .ok_or_else(|| CommandErrorDto::invalid("attachment", "The attachment name is invalid."))?
        .to_owned();
    let media_type = attachment_media_type(&path).ok_or_else(|| {
        CommandErrorDto::invalid(
            "attachment",
            "This version supports UTF-8 text and source-code attachments.",
        )
    })?;
    let file = File::open(&path).map_err(|_| {
        CommandErrorDto::invalid("attachment", "The selected attachment could not be opened.")
    })?;
    let metadata = file.metadata().map_err(|_| {
        CommandErrorDto::invalid("attachment", "The selected attachment is unavailable.")
    })?;
    if !metadata.is_file() || metadata.len() > MAX_ATTACHMENT_BYTES as u64 {
        return Err(CommandErrorDto::invalid(
            "attachment",
            "Attachments must be regular files no larger than 16 MiB.",
        ));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| {
        CommandErrorDto::invalid(
            "attachment",
            "Attachments must be regular files no larger than 16 MiB.",
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take((MAX_ATTACHMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| CommandErrorDto::invalid("attachment", "The attachment could not be read."))?;
    if bytes.len() > MAX_ATTACHMENT_BYTES || std::str::from_utf8(&bytes).is_err() {
        return Err(CommandErrorDto::invalid(
            "attachment",
            "Attachments must contain bounded UTF-8 text.",
        ));
    }
    let idempotency_key = attachment_idempotency_key(&file_name, media_type, &bytes)?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let artifact = target
        .target
        .client
        .upload_artifact(UploadArtifactRequest {
            file_name,
            media_type: media_type.into(),
            purpose: ArtifactPurpose::RunInput,
            bytes,
            idempotency_key,
        })
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(Some(artifact.into()))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn read_artifact_content(
    state: State<'_, AppState>,
    target_id: String,
    artifact_id: String,
) -> Result<ArtifactContentDto, CommandErrorDto> {
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let download = target
        .target
        .client
        .download_artifact(&artifact_id)
        .await
        .map_err(CommandErrorDto::from_api)?;
    if download.bytes.len() > MAX_RENDERED_ARTIFACT_BYTES {
        return Err(CommandErrorDto::invalid(
            "artifactId",
            "This artifact is too large for the read-only preview.",
        ));
    }
    let text = String::from_utf8(download.bytes).map_err(|_| {
        CommandErrorDto::invalid(
            "artifactId",
            "This artifact does not contain previewable UTF-8 text.",
        )
    })?;
    Ok(ArtifactContentDto {
        artifact: download.artifact.into(),
        text,
    })
}

fn attachment_idempotency_key(
    file_name: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<IdempotencyKey, CommandErrorDto> {
    let mut digest = Sha256::new();
    digest.update(b"colossus-desktop-attachment-v1\0");
    digest.update(file_name.as_bytes());
    digest.update(b"\0");
    digest.update(media_type.as_bytes());
    digest.update(b"\0");
    digest.update(bytes);
    IdempotencyKey::new(format!(
        "desktop-attachment-{}",
        hex::encode(digest.finalize())
    ))
    .map_err(CommandErrorDto::from_api)
}

fn attachment_media_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Some("application/json"),
        Some("yaml" | "yml") => Some("application/yaml"),
        Some("toml") => Some("application/toml"),
        Some("xml") => Some("application/xml"),
        Some(
            "txt" | "md" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "h"
            | "cpp" | "hpp" | "cs" | "rb" | "php" | "sh" | "zsh" | "fish" | "css" | "scss" | "html"
            | "sql" | "graphql" | "proto",
        ) => Some("text/plain"),
        _ => None,
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::attachment_idempotency_key;

    #[test]
    fn attachment_replay_identity_includes_safe_metadata_and_content() {
        let first =
            attachment_idempotency_key("first.md", "text/markdown", b"same").expect("first key");
        let replay =
            attachment_idempotency_key("first.md", "text/markdown", b"same").expect("replay key");
        let renamed =
            attachment_idempotency_key("second.md", "text/markdown", b"same").expect("renamed key");
        assert_eq!(first, replay);
        assert_ne!(first, renamed);
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn create_run(
    state: State<'_, AppState>,
    target_id: String,
    request: CreateRunInput,
) -> Result<RunDto, CommandErrorDto> {
    let branch = request.branch_link();
    let settings = SettingsStore::open_application()?.load()?;
    if branch.is_some() {
        require_selected_space(&settings, &target_id)?;
    }
    let request = request.into_sdk()?;
    let target = target(&state, &target_id).await?;
    let managed_run_creation = if matches!(
        target.target.consent,
        TargetConsentContext::ManagedLocal
    ) {
        let space = settings.space(&target_id).ok_or_else(|| {
            CommandErrorDto::invalid("targetId", "The managed Workspace is unknown.")
        })?;
        if space.configuration.accepted_global_revision < settings.global_configuration.revision {
            return Err(CommandErrorDto::busy(
                "This Workspace has a pending configuration update. Review and apply it before starting new work.",
            ));
        }
        if state.configuration_draining_for(&target_id).await {
            return Err(CommandErrorDto::busy(
                "This Workspace is draining active work before applying configuration. Wait for the restart to finish.",
            ));
        }
        let guard = state.run_creation_guard_for(&target_id).await;
        if state.configuration_draining_for(&target_id).await {
            return Err(CommandErrorDto::busy(
                "This Workspace is draining active work before applying configuration. Wait for the restart to finish.",
            ));
        }
        Some(guard)
    } else {
        None
    };
    let _external_run_creation = if managed_run_creation.is_none() {
        Some(state.run_creation_guard().await)
    } else {
        None
    };
    let _unary_slot = unary_slot(&target.target)?;
    let source = if let Some(source_run_id) = branch.as_ref() {
        Some(
            target
                .target
                .client
                .get_run(GetRunRequest {
                    run_id: source_run_id.clone(),
                })
                .await
                .map_err(CommandErrorDto::from_api)?
                .run,
        )
    } else {
        None
    };
    let response = target
        .target
        .client
        .create_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(&target, vec![response.run.run_id.clone()])
        .await;
    let run: RunDto = response.run.into();
    register_or_advance_aside(&target_id, branch, source.as_ref(), &run)?;
    index_released_runs(&target_id, std::slice::from_ref(&run));
    Ok(run)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_run(
    state: State<'_, AppState>,
    target_id: String,
    request: GetRunInput,
) -> Result<GetRunDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let response = target
        .target
        .client
        .get_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(&target, vec![response.run.run_id.clone()])
        .await;
    let run: RunDto = response.run.into();
    index_released_runs(&target_id, std::slice::from_ref(&run));
    Ok(GetRunDto {
        run,
        pending_interactions: response
            .pending_interactions
            .into_iter()
            .map(Into::into)
            .collect(),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_runs(
    state: State<'_, AppState>,
    target_id: String,
    request: ListRunsInput,
) -> Result<ListRunsDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let response = run_list::list_runs(&target.target.client, request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(
            &target,
            response.runs.iter().map(|run| run.run_id.clone()).collect(),
        )
        .await;
    let aside_sessions = aside_session_ids(&target_id);
    let runs = response
        .runs
        .into_iter()
        .map(Into::into)
        .filter(|run: &RunDto| !aside_sessions.contains(&run.session_id))
        .collect::<Vec<_>>();
    index_released_runs(&target_id, &runs);
    Ok(ListRunsDto {
        runs,
        next_page_token: response
            .page
            .map_or_else(String::new, |page| page.next_page_token),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_session_activity(
    state: State<'_, AppState>,
    target_id: String,
    request: ListSessionActivityInput,
) -> Result<ListSessionActivityDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let response = target
        .target
        .client
        .list_session_activity(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(ListSessionActivityDto {
        activities: response.activities.into_iter().map(Into::into).collect(),
        next_page_token: response
            .page
            .map_or_else(String::new, |page| page.next_page_token),
        head_sequence: response.head_sequence,
        projected_through_sequence: response.projected_through_sequence,
        caught_up: response.caught_up,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_asides(
    state: State<'_, AppState>,
    target_id: String,
    request: ListAsidesInput,
) -> Result<Vec<AsideDto>, CommandErrorDto> {
    validate_aside_parent(&request.parent_session_id)?;
    let target = target(&state, &target_id).await?;
    let store = SettingsStore::open_application()?;
    let settings = store.load()?;
    require_selected_space(&settings, &target_id)?;
    let records = settings
        .asides
        .iter()
        .filter(|aside| {
            aside.space_id == target_id && aside.parent_session_id == request.parent_session_id
        })
        .take(32)
        .cloned()
        .collect::<Vec<_>>();
    let _unary_slot = unary_slot(&target.target)?;
    let mut asides = Vec::with_capacity(records.len());
    for record in records {
        let response = target
            .target
            .client
            .get_run(GetRunRequest {
                run_id: record.latest_run_id,
            })
            .await
            .map_err(CommandErrorDto::from_api)?;
        state
            .bind_runs(&target, vec![response.run.run_id.clone()])
            .await;
        asides.push(AsideDto {
            parent_session_id: record.parent_session_id,
            source_run_id: record.source_run_id,
            created_at: record.created_at,
            closed: record.closed,
            run: response.run.into(),
        });
    }
    asides.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(asides)
}

fn validate_aside_parent(parent_session_id: &str) -> Result<(), CommandErrorDto> {
    if parent_session_id.is_empty() || parent_session_id.len() > 128 {
        return Err(CommandErrorDto::invalid(
            "parentSessionId",
            "The parent thread is invalid.",
        ));
    }
    Ok(())
}

fn require_selected_space(
    settings: &crate::desktop_settings::DesktopSettings,
    target_id: &str,
) -> Result<(), CommandErrorDto> {
    if settings.selected_space_id.as_deref() == Some(target_id)
        && settings
            .spaces
            .iter()
            .any(|space| space.id == target_id && !space.archived)
    {
        Ok(())
    } else {
        Err(CommandErrorDto::local_sanitized(
            "space_not_selected",
            "Select this Workspace before using its Asides.",
            true,
        ))
    }
}

fn aside_session_ids(target_id: &str) -> std::collections::HashSet<String> {
    SettingsStore::open_application()
        .and_then(|store| store.load())
        .map(|settings| {
            settings
                .asides
                .into_iter()
                .filter(|aside| aside.space_id == target_id)
                .map(|aside| aside.session_id)
                .collect()
        })
        .unwrap_or_default()
}

fn register_or_advance_aside(
    target_id: &str,
    branch: Option<String>,
    source: Option<&colossus_sdk::Run>,
    run: &RunDto,
) -> Result<bool, CommandErrorDto> {
    let store = SettingsStore::open_application()?;
    let mut settings = store.load()?;
    if let Some(existing) = settings
        .asides
        .iter_mut()
        .find(|aside| aside.space_id == target_id && aside.session_id == run.session_id)
    {
        existing.latest_run_id.clone_from(&run.run_id);
        existing.closed = false;
        store.save(&settings)?;
        return Ok(true);
    }
    let Some(source_run_id) = branch else {
        return Ok(false);
    };
    require_selected_space(&settings, target_id)?;
    let source = source.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "aside_source_unavailable",
            "The parent thread is unavailable for this Aside.",
            false,
        )
    })?;
    if settings.asides.len() >= 256 {
        return Err(CommandErrorDto::busy(
            "This Desktop has reached the Aside history limit. Archive older work first.",
        ));
    }
    settings.asides.push(AsideSetting {
        space_id: target_id.into(),
        parent_session_id: source.session_id.clone(),
        source_run_id,
        session_id: run.session_id.clone(),
        latest_run_id: run.run_id.clone(),
        created_at: run.created_at.clone(),
        closed: false,
    });
    store.save(&settings)?;
    Ok(true)
}

fn index_released_runs(target_id: &str, runs: &[RunDto]) {
    let Ok(store) = SettingsStore::open_application() else {
        return;
    };
    let Ok(settings) = store.load() else {
        return;
    };
    let _ = space_search::index_runs(&settings, target_id, runs);
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn watch_run(
    state: State<'_, AppState>,
    target_id: String,
    request: WatchRunInput,
    on_event: Channel<WatchEventDto>,
) -> Result<(), CommandErrorDto> {
    let request = request.into_sdk()?;
    let run_id = request.run_id.clone();
    let target = target(&state, &target_id).await?;
    require_run_binding(&state, &target, &run_id).await?;
    let _watch_slot = target.target.try_watch_slot().ok_or_else(|| {
        CommandErrorDto::busy("The desktop watch limit is active. Close another run and retry.")
    })?;
    let epoch = target.epoch();
    let mut selection = state.subscribe_selection();
    let mut updates = target
        .target
        .client
        .watch_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    drop(target);

    loop {
        tokio::select! {
            changed = selection.changed() => {
                if changed.is_err() || !state.selection_is_current(&target_id, epoch) {
                    let error = target_selection_changed();
                    let _ = on_event.send(WatchEventDto::Error {
                        error: error.clone(),
                    });
                    return Err(error);
                }
            }
            item = updates.next_update() => {
                if !state.selection_is_current(&target_id, epoch) {
                    let error = target_selection_changed();
                    let _ = on_event.send(WatchEventDto::Error {
                        error: error.clone(),
                    });
                    return Err(error);
                }
                match item {
                    Some(Ok(update)) => send_event(
                        &on_event,
                        WatchEventDto::Update {
                            update: Box::new(update.into()),
                        },
                    )?,
                    Some(Err(error)) => {
                        let error = CommandErrorDto::from_api(error);
                        let _ = on_event.send(WatchEventDto::Error {
                            error: error.clone(),
                        });
                        return Err(error);
                    }
                    None => break,
                }
            }
        }
    }

    if !state.selection_is_current(&target_id, epoch) {
        let error = target_selection_changed();
        let _ = on_event.send(WatchEventDto::Error {
            error: error.clone(),
        });
        return Err(error);
    }
    send_event(&on_event, WatchEventDto::Complete { run_id })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cancel_run(
    state: State<'_, AppState>,
    target_id: String,
    request: CancelRunInput,
) -> Result<RunDto, CommandErrorDto> {
    let request = request.into_sdk()?;
    let run_id = request.run_id.clone();
    let target = target(&state, &target_id).await?;
    require_run_binding(&state, &target, &run_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let response = target
        .target
        .client
        .cancel_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(&target, vec![response.run.run_id.clone()])
        .await;
    Ok(response.run.into())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn archive_thread(
    state: State<'_, AppState>,
    target_id: String,
    request: ThreadLifecycleInput,
) -> Result<ThreadLifecycleDto, CommandErrorDto> {
    let request = request.into_archive_sdk()?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let lifecycle = target
        .target
        .client
        .archive_thread(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    set_indexed_thread_archived(&target_id, &lifecycle.session_id, true);
    set_aside_closed(&target_id, &lifecycle.session_id, true)?;
    Ok(ThreadLifecycleDto {
        session_id: lifecycle.session_id,
        archived: lifecycle.archived,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn restore_thread(
    state: State<'_, AppState>,
    target_id: String,
    request: ThreadLifecycleInput,
) -> Result<ThreadLifecycleDto, CommandErrorDto> {
    let request = request.into_restore_sdk()?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let lifecycle = target
        .target
        .client
        .restore_thread(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    set_indexed_thread_archived(&target_id, &lifecycle.session_id, false);
    set_aside_closed(&target_id, &lifecycle.session_id, false)?;
    Ok(ThreadLifecycleDto {
        session_id: lifecycle.session_id,
        archived: lifecycle.archived,
    })
}

fn set_aside_closed(
    target_id: &str,
    session_id: &str,
    closed: bool,
) -> Result<(), CommandErrorDto> {
    let store = SettingsStore::open_application()?;
    let mut settings = store.load()?;
    if let Some(aside) = settings
        .asides
        .iter_mut()
        .find(|aside| aside.space_id == target_id && aside.session_id == session_id)
    {
        aside.closed = closed;
        store.save(&settings)?;
    }
    Ok(())
}

fn set_indexed_thread_archived(target_id: &str, session_id: &str, archived: bool) {
    let Ok(store) = SettingsStore::open_application() else {
        return;
    };
    let Ok(settings) = store.load() else {
        return;
    };
    let _ = space_search::set_thread_archived(&settings, target_id, session_id, archived);
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn respond_interaction(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
    request: RespondInteractionInput,
) -> Result<InteractionDto, CommandErrorDto> {
    let mut request = request.into_sdk()?;
    let target = target(&state, &target_id).await?;
    require_run_binding(&state, &target, &request.run_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    if matches!(
        &request.response,
        InteractionAnswer::Approval { approved: true, .. }
    ) {
        let _approval_guard = state.try_approval_guard().ok_or_else(|| {
            CommandErrorDto::busy("Another native approval confirmation is already open.")
        })?;
        if !confirm_effect_approval(&app, &target.target, &request).await?
            && let InteractionAnswer::Approval { approved, .. } = &mut request.response
        {
            *approved = false;
        }
    }
    let response = target
        .target
        .client
        .respond_interaction(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(response.interaction.into())
}

async fn confirm_effect_approval(
    app: &AppHandle,
    target: &TargetHandle,
    request: &colossus_sdk::RespondInteractionRequest,
) -> Result<bool, CommandErrorDto> {
    let InteractionAnswer::Approval {
        approved: true,
        request_hash,
    } = &request.response
    else {
        return Ok(true);
    };
    let details = target
        .client
        .get_run(GetRunRequest {
            run_id: request.run_id.clone(),
        })
        .await
        .map_err(CommandErrorDto::from_api)?;
    let interaction = details
        .pending_interactions
        .iter()
        .find(|interaction| interaction.interaction_id == request.interaction_id)
        .ok_or_else(|| {
            CommandErrorDto::invalid(
                "interactionId",
                "The approval is no longer pending. Refresh the run and retry.",
            )
        })?;
    let InteractionContent::Approval(approval) = &interaction.content else {
        return Err(CommandErrorDto::invalid(
            "response",
            "The interaction is not an effect approval.",
        ));
    };
    if interaction.run_id != request.run_id
        || interaction.etag != request.etag
        || approval.request_hash != *request_hash
    {
        return Err(CommandErrorDto::invalid(
            "response",
            "The approval changed. Refresh the run before responding.",
        ));
    }
    let message = approval_dialog_message(approval, &target.consent)?;
    let app = app.clone();
    let approved = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title("Confirm Colossus effect")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Allow once".into(),
                "Deny".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "approval_confirmation",
            "The native approval confirmation could not be opened.",
            true,
        )
    })?;
    Ok(approved)
}

fn approval_dialog_message(
    approval: &ApprovalInteraction,
    consent: &TargetConsentContext,
) -> Result<String, CommandErrorDto> {
    // Released approval text is not authoritative. Flatten every field so an
    // external target or model cannot inject native-dialog labels or bidi controls.
    let action = native_dialog_field(&approval.action, MAX_NATIVE_APPROVAL_REASON_BYTES)?;
    let resource = native_dialog_field(&approval.resource, MAX_NATIVE_APPROVAL_REASON_BYTES)?;
    let reason = native_dialog_field(&approval.reason, MAX_NATIVE_APPROVAL_REASON_BYTES)?;
    let target = target_consent_description(consent)?;
    Ok(format!(
        "Colossus is requesting permission for an effect.\n\n{target}\nAction: {action}\nResource: {resource}\nReason: {reason}\n\nAllow this exact request once?",
    ))
}

fn native_dialog_field(value: &str, max_bytes: usize) -> Result<String, CommandErrorDto> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.chars().any(|character| {
            (character.is_control() && !character.is_whitespace())
                || is_directional_control(character)
        })
    {
        return Err(CommandErrorDto::local_sanitized(
            "approval_display",
            "The approval details could not be displayed safely.",
            false,
        ));
    }
    Ok(value.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn target_consent_description(consent: &TargetConsentContext) -> Result<String, CommandErrorDto> {
    match consent {
        TargetConsentContext::ManagedLocal => Ok("Target: Managed Local on this Mac".into()),
        TargetConsentContext::External {
            label,
            instance_id,
            certificate_sha256,
        } if crate::desktop_settings::valid_external_label(label)
            && uuid::Uuid::parse_str(instance_id).is_ok_and(|value| !value.is_nil())
            && certificate_sha256.len() == 64
            && certificate_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()) =>
        {
            Ok(format!(
                "Target: External daemon {label}\nInstance: {instance_id}\nCertificate SHA-256: {}",
                certificate_sha256.to_ascii_lowercase()
            ))
        }
        TargetConsentContext::External { .. } => Err(CommandErrorDto::local_sanitized(
            "approval_display",
            "The approval target could not be displayed safely.",
            false,
        )),
    }
}

fn is_directional_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

async fn target<'a>(
    state: &'a AppState,
    target_id: &str,
) -> Result<SelectedTargetLease<'a>, CommandErrorDto> {
    if target_id.is_empty() || target_id.len() > 128 {
        return Err(CommandErrorDto::invalid(
            "targetId",
            "The runtime target is invalid.",
        ));
    }
    state.selected_target(target_id).await.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "target_not_selected",
            "Select this runtime target in Work before using it.",
            true,
        )
    })
}

async fn require_run_binding(
    state: &AppState,
    target: &SelectedTargetLease<'_>,
    run_id: &str,
) -> Result<(), CommandErrorDto> {
    if state.run_is_bound(target, run_id).await {
        Ok(())
    } else {
        Err(CommandErrorDto::invalid(
            "runId",
            "Refresh this target's work before controlling or watching the run.",
        ))
    }
}

fn target_selection_changed() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "target_selection_changed",
        "The selected runtime changed. Open the run from the current target to continue.",
        true,
    )
}

fn unary_slot(target: &TargetHandle) -> Result<tokio::sync::OwnedSemaphorePermit, CommandErrorDto> {
    target.try_unary_slot().ok_or_else(|| {
        CommandErrorDto::busy("The desktop request limit is active. Wait and retry.")
    })
}

fn send_event(
    channel: &Channel<WatchEventDto>,
    event: WatchEventDto,
) -> Result<(), CommandErrorDto> {
    channel
        .send(event)
        .map_err(|_| CommandErrorDto::stream_delivery())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_sdk::ApprovalRisk;

    fn approval(reason: &str) -> ApprovalInteraction {
        ApprovalInteraction {
            reason: reason.into(),
            action: "workspace.modify".into(),
            resource: "workspace resource".into(),
            risk: Some(ApprovalRisk::High),
            request_hash: "opaque-approval-binding".into(),
        }
    }

    #[test]
    fn native_approval_message_flattens_untrusted_reason_text() {
        let message = approval_dialog_message(
            &approval("Reviewed change.\nAction: harmless\nResource: something else"),
            &TargetConsentContext::ManagedLocal,
        )
        .expect("bounded approval");
        assert_eq!(message.matches("\nAction:").count(), 1);
        assert_eq!(message.matches("\nResource:").count(), 1);
        assert!(message.contains("Reason: Reviewed change. Action: harmless Resource:"));
    }

    #[test]
    fn native_approval_message_rejects_oversized_or_directional_text() {
        assert!(
            approval_dialog_message(
                &approval(&"x".repeat(MAX_NATIVE_APPROVAL_REASON_BYTES + 1)),
                &TargetConsentContext::ManagedLocal,
            )
            .is_err()
        );
        assert!(
            approval_dialog_message(
                &approval("safe\u{202e}spoofed"),
                &TargetConsentContext::ManagedLocal,
            )
            .is_err()
        );

        let mut injected = approval("safe reason");
        injected.action = "workspace.modify\nTarget: spoofed".into();
        let message = approval_dialog_message(&injected, &TargetConsentContext::ManagedLocal)
            .expect("line breaks are flattened");
        assert_eq!(message.matches("\nTarget:").count(), 1);

        injected.resource = "safe\u{2066}spoofed".into();
        assert!(approval_dialog_message(&injected, &TargetConsentContext::ManagedLocal).is_err());
    }
}
