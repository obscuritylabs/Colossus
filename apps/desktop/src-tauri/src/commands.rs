use colossus_sdk::{
    ApprovalInteraction, ArtifactPurpose, GetRunRequest, IdempotencyKey, InteractionAnswer,
    InteractionContent, UploadArtifactRequest,
};
use sha2::{Digest as _, Sha256};
use std::{fs::File, io::Read as _, path::Path};
use tauri::{AppHandle, State, ipc::Channel};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::{
    dto::{
        ArtifactContentDto, ArtifactReferenceDto, CancelRunInput, CommandErrorDto, CreateRunInput,
        GetRunDto, GetRunInput, InteractionDto, ListRunsDto, ListRunsInput,
        RespondInteractionInput, RunDto, WatchEventDto, WatchRunInput,
    },
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
    let _run_creation = state.run_creation_guard().await;
    let request = request.into_sdk()?;
    let target = target(&state, &target_id).await?;
    let _unary_slot = unary_slot(&target.target)?;
    let response = target
        .target
        .client
        .create_run(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(&target, vec![response.run.run_id.clone()])
        .await;
    Ok(response.run.into())
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
    Ok(GetRunDto {
        run: response.run.into(),
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
    let response = target
        .target
        .client
        .list_runs(request)
        .await
        .map_err(CommandErrorDto::from_api)?;
    state
        .bind_runs(
            &target,
            response.runs.iter().map(|run| run.run_id.clone()).collect(),
        )
        .await;
    Ok(ListRunsDto {
        runs: response.runs.into_iter().map(Into::into).collect(),
        next_page_token: response
            .page
            .map_or_else(String::new, |page| page.next_page_token),
    })
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
