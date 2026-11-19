//! Native, on-demand export of an authorized canonical conversation snapshot.
//!
//! The renderer supplies a run identity and format, never a path or transcript.
use std::{
    future::Future,
    io::{Read as _, Seek as _, Write as _},
    path::Path,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use colossus_presentation::{
    MAX_TRANSCRIPT_EXPORT_PAGES, TranscriptFormat, TranscriptWriter, transcript_filename,
};
use colossus_sdk::{AgentRunClient, GetRunRequest, ReadSessionTranscriptRequest, TranscriptPage};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, ipc::Channel};
use tauri_plugin_dialog::DialogExt as _;
use tempfile::NamedTempFile;
use tokio::sync::watch;

use crate::{
    commands::{ensure_run_target, target, unary_slot},
    conversation_runs::{ConversationProofs, ConversationSource},
    dto::CommandErrorDto,
    state::{AppState, SelectedTargetLease},
};

const CAPABILITY: &str = "conversations.transcript.v1";
const MAX_COPY_BYTES: u64 = 2 * 1024 * 1024;
const EXPORT_TIMEOUT: Duration = Duration::from_mins(5);

#[derive(Default)]
pub(crate) struct ConversationExports(Mutex<Option<Operation>>);

struct Operation {
    id: String,
    target_id: String,
    cancel: watch::Sender<bool>,
}

impl ConversationExports {
    pub(crate) fn cancel(&self) {
        if let Some(operation) = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            operation.cancel.send_replace(true);
        }
    }

    fn cancel_matching(
        &self,
        operation_id: &str,
        target_id: &str,
    ) -> Result<ExportConversationResult, CommandErrorDto> {
        validate_identity(operation_id, "operationId")?;
        validate_identity(target_id, "targetId")?;
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(operation) = slot.as_ref()
            && operation.id == operation_id
            && operation.target_id == target_id
        {
            operation.cancel.send_replace(true);
        }
        Ok(cancelled_result())
    }

    fn begin(&self, id: &str, target_id: &str) -> Result<Registration<'_>, CommandErrorDto> {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return Err(CommandErrorDto::busy(
                "Another conversation export is in progress.",
            ));
        }
        let (cancel, receiver) = watch::channel(false);
        *slot = Some(Operation {
            id: id.to_owned(),
            target_id: target_id.to_owned(),
            cancel,
        });
        Ok(Registration {
            exports: self,
            receiver,
        })
    }
}

struct Registration<'a> {
    exports: &'a ConversationExports,
    receiver: watch::Receiver<bool>,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.exports
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ExportConversationInput {
    Start {
        operation_id: String,
        run_id: String,
        #[serde(default)]
        discovery_proof: Option<String>,
        #[serde(default)]
        format: TranscriptFormat,
        destination: Destination,
    },
    Cancel {
        operation_id: String,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Destination {
    Save,
    Copy,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportConversationResult {
    status: &'static str,
    messages: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportProgress {
    phase: &'static str,
    pages_read: usize,
    messages_read: u64,
}

struct Prepared {
    file: NamedTempFile,
    filename: String,
    messages: u64,
}

struct ExportSelection<'a> {
    state: &'a AppState,
    target_id: &'a str,
    epoch: u64,
    reader: &'a Arc<dyn AgentRunClient>,
    source: ConversationSource,
}

impl ExportSelection<'_> {
    async fn revalidate(&self) -> Result<SelectedTargetLease<'_>, CommandErrorDto> {
        let current = target(self.state, self.target_id).await?;
        if current.epoch() != self.epoch
            || !self.state.selection_is_current(self.target_id, self.epoch)
            || !Arc::ptr_eq(
                self.reader,
                &current
                    .target
                    .conversation_reader(self.source)
                    .map_err(CommandErrorDto::from_api)?,
            )
        {
            return Err(selection_changed());
        }
        Ok(current)
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn export_conversation(
    app: AppHandle,
    state: State<'_, AppState>,
    exports: State<'_, ConversationExports>,
    target_id: String,
    request: ExportConversationInput,
    on_progress: Channel<ExportProgress>,
) -> Result<ExportConversationResult, CommandErrorDto> {
    let ExportConversationInput::Start {
        operation_id,
        run_id,
        discovery_proof,
        format,
        destination,
    } = request
    else {
        let ExportConversationInput::Cancel { operation_id } = request else {
            unreachable!()
        };
        return exports.cancel_matching(&operation_id, &target_id);
    };
    validate_identity(&operation_id, "operationId")?;
    validate_identity(&run_id, "runId")?;
    if discovery_proof
        .as_ref()
        .is_some_and(|proof| proof.len() > 4096)
    {
        return Err(CommandErrorDto::invalid(
            "discoveryProof",
            "Reload conversation history before exporting.",
        ));
    }
    let on_progress = Some(on_progress);
    let mut registration = exports.begin(&operation_id, &target_id)?;
    let mut selection = state.subscribe_selection();
    let selected = target(&state, &target_id).await?;
    let epoch = selected.epoch();
    let _slot = unary_slot(&selected.target)?;
    ensure_run_target(&state, &selected, &run_id).await?;
    let verified = selected
        .target
        .get_conversation_with_proofs(
            GetRunRequest {
                run_id: run_id.clone(),
            },
            ConversationProofs {
                discovery: discovery_proof,
                continuation: None,
            },
        )
        .await
        .map_err(CommandErrorDto::from_api)?;
    let capabilities = if verified.source.uses_connector() {
        selected.target.client.connector_capabilities()
    } else {
        selected.target.client.capabilities()
    };
    if !capabilities.contains(CAPABILITY) {
        return Err(local(
            "export_unsupported",
            "This runtime does not support full conversation export. Update the runtime and reconnect.",
        ));
    }
    let session_id = verified.response.run.session_id;
    let source = verified.source;
    let reader = verified.reader;
    state.bind_runs(&selected, vec![run_id.clone()]).await;
    drop(selected); // Selection changes must cancel a long read, not wait behind it.
    let prepared = tokio::select! {
        biased;
        _ = registration.receiver.changed() => return Ok(cancelled_result()),
        _ = selection.changed() => return Err(selection_changed()),
        result = tokio::time::timeout(EXPORT_TIMEOUT, prepare(
            |page_token| reader.read_session_transcript(ReadSessionTranscriptRequest {
                source_run_id: run_id.clone(), page_size: 32, page_token,
            }), &session_id, format, on_progress.as_ref(),
        )) => result.map_err(|_| local("export_timeout", "The export exceeded five minutes. Nothing was saved."))??,
    };
    complete_export(
        app,
        ExportSelection {
            state: &state,
            target_id: &target_id,
            epoch,
            reader: &reader,
            source,
        },
        &registration,
        prepared,
        format,
        destination,
        on_progress.as_ref(),
    )
    .await
}

async fn complete_export(
    app: AppHandle,
    selected: ExportSelection<'_>,
    registration: &Registration<'_>,
    prepared: Prepared,
    format: TranscriptFormat,
    destination: Destination,
    on_progress: Option<&Channel<ExportProgress>>,
) -> Result<ExportConversationResult, CommandErrorDto> {
    let Prepared {
        mut file,
        filename,
        messages,
    } = prepared;
    if *registration.receiver.borrow() {
        return Ok(cancelled_result());
    }
    match destination {
        Destination::Save => {
            send_progress(on_progress, "choosing_destination", 0, messages)?;
            let picked = tokio::task::spawn_blocking(move || {
                app.dialog()
                    .file()
                    .set_file_name(filename)
                    .add_filter("Conversation transcript", &[format.extension()])
                    .blocking_save_file()
            })
            .await
            .map_err(|_| io_error())?;
            let Some(picked) = picked else {
                return Ok(cancelled_result());
            };
            let path = picked.into_path().map_err(|_| io_error())?;
            if *registration.receiver.borrow() {
                return Ok(cancelled_result());
            }
            send_progress(on_progress, "saving", 0, messages)?;
            // Stage all bytes beside the selected file before taking the publication lease.
            let staged = tokio::task::spawn_blocking(move || {
                stage_destination(&mut file, &path).map(|file| (file, path))
            })
            .await
            .map_err(|_| io_error())??;
            let _current = selected.revalidate().await?;
            // Serialize cancellation with the final no-clobber publication. This small rename
            // holds the native selection lease, so a successful receipt names a saved file.
            let operation = registration
                .exports
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if operation
                .as_ref()
                .is_none_or(|operation| *operation.cancel.borrow())
            {
                return Ok(cancelled_result());
            }
            publish(staged.0, &staged.1)?;
            Ok(ExportConversationResult {
                status: "saved",
                messages,
                text: None,
            })
        }
        Destination::Copy => {
            let text = tokio::task::spawn_blocking(move || copy_text(&mut file))
                .await
                .map_err(|_| io_error())??;
            let _current = selected.revalidate().await?;
            if *registration.receiver.borrow() {
                return Ok(cancelled_result());
            }
            // Desktop has no native clipboard adapter. Release only this bounded formatted
            // result; the renderer checks its exact current route before the clipboard call.
            Ok(ExportConversationResult {
                status: "copy",
                messages,
                text: Some(text),
            })
        }
    }
}

async fn prepare<ReadPage, PageFuture>(
    mut read: ReadPage,
    session_id: &str,
    format: TranscriptFormat,
    progress: Option<&Channel<ExportProgress>>,
) -> Result<Prepared, CommandErrorDto>
where
    ReadPage: FnMut(Option<String>) -> PageFuture,
    PageFuture: Future<Output = Result<TranscriptPage, colossus_sdk::ApiError>>,
{
    let mut cursor = None;
    let mut writer = None;
    let mut filename = String::new();
    let mut messages = 0;
    for page_number in 1..=MAX_TRANSCRIPT_EXPORT_PAGES {
        let page = read(cursor).await.map_err(CommandErrorDto::from_api)?;
        if page.snapshot.session_id != session_id {
            return Err(invalid_transcript());
        }
        messages += u64::try_from(page.messages.len()).map_err(|_| invalid_transcript())?;
        cursor = page.next_cursor.clone();
        if writer.is_none() {
            filename = transcript_filename(
                page.snapshot.title.as_deref().unwrap_or("Conversation"),
                session_id,
                format,
            );
            let snapshot = page.snapshot.clone();
            writer = Some(
                tokio::task::spawn_blocking(move || {
                    let file = NamedTempFile::new().map_err(|_| io_error())?;
                    TranscriptWriter::new(file, format, &snapshot).map_err(|_| invalid_transcript())
                })
                .await
                .map_err(|_| io_error())??,
            );
        }
        let mut active = writer.take().ok_or_else(invalid_transcript)?;
        writer = Some(
            tokio::task::spawn_blocking(move || {
                active.write_page(&page).map_err(|_| invalid_transcript())?;
                Ok::<_, CommandErrorDto>(active)
            })
            .await
            .map_err(|_| io_error())??,
        );
        send_progress(progress, "reading", page_number, messages)?;
        if cursor.is_none() {
            let active = writer.take().ok_or_else(invalid_transcript)?;
            let file = tokio::task::spawn_blocking(move || {
                active.finish().map_err(|_| invalid_transcript())
            })
            .await
            .map_err(|_| io_error())??;
            return Ok(Prepared {
                file,
                filename,
                messages,
            });
        }
    }
    Err(local(
        "export_limit",
        "The export exceeded the 10,000-page limit. Nothing was saved.",
    ))
}

fn stage_destination(
    source: &mut NamedTempFile,
    path: &Path,
) -> Result<NamedTempFile, CommandErrorDto> {
    let parent = path.parent().ok_or_else(io_error)?;
    let mut staged = NamedTempFile::new_in(parent).map_err(|_| io_error())?;
    source.rewind().map_err(|_| io_error())?;
    std::io::copy(source, &mut staged)
        .and_then(|_| staged.flush())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|_| io_error())?;
    Ok(staged)
}

fn publish(staged: NamedTempFile, path: &Path) -> Result<(), CommandErrorDto> {
    staged.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            local(
                "export_exists",
                "That file already exists. Export again and choose a new filename.",
            )
        } else {
            io_error()
        }
    })?;
    Ok(())
}

fn copy_text(file: &mut NamedTempFile) -> Result<String, CommandErrorDto> {
    if file.as_file().metadata().map_err(|_| io_error())?.len() > MAX_COPY_BYTES {
        return Err(local(
            "export_copy_limit",
            "This transcript is larger than the 2 MiB copy limit. Choose Save instead.",
        ));
    }
    file.rewind().map_err(|_| io_error())?;
    let mut text = String::new();
    file.take(MAX_COPY_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| io_error())?;
    if text.len() as u64 > MAX_COPY_BYTES {
        return Err(invalid_transcript());
    }
    Ok(text)
}

fn validate_identity(value: &str, field: &str) -> Result<(), CommandErrorDto> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(CommandErrorDto::invalid(
            field,
            "The export identity is invalid.",
        ));
    }
    Ok(())
}

fn send_progress(
    channel: Option<&Channel<ExportProgress>>,
    phase: &'static str,
    pages_read: usize,
    messages_read: u64,
) -> Result<(), CommandErrorDto> {
    if let Some(channel) = channel {
        channel
            .send(ExportProgress {
                phase,
                pages_read,
                messages_read,
            })
            .map_err(|_| {
                local(
                    "export_cancelled",
                    "The export window closed. Nothing was saved.",
                )
            })?;
    }
    Ok(())
}
fn local(code: &str, message: &str) -> CommandErrorDto {
    CommandErrorDto::local_sanitized(code, message, true)
}
fn io_error() -> CommandErrorDto {
    local(
        "export_io",
        "The transcript could not be saved. Choose a writable location and retry.",
    )
}
fn invalid_transcript() -> CommandErrorDto {
    local(
        "export_incomplete",
        "The transcript is incomplete, changed, or exceeded the 256 MiB export limit. Nothing was saved.",
    )
}
fn selection_changed() -> CommandErrorDto {
    local(
        "export_selection_changed",
        "The runtime changed during export. Nothing was saved. Reopen the conversation and retry.",
    )
}
fn cancelled_result() -> ExportConversationResult {
    ExportConversationResult {
        status: "cancelled",
        messages: 0,
        text: None,
    }
}

#[cfg(test)]
#[path = "conversation_export/tests.rs"]
mod tests;
