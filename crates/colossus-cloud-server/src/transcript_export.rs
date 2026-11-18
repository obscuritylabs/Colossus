//! Complete on-demand exports through the authorized thread's fixed runtime reader.
use crate::{http::Error, server::State};
use axum::{
    Json,
    extract::{Path, State as Extract},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use colossus_cloud::{CloudCaller, CloudError, CloudPermission, CloudThread};
use colossus_cloud_protocol::{ResourceOperation, ResourceReply};
use colossus_presentation::{TranscriptFormat, TranscriptWriter, transcript_filename};
use colossus_sdk::TranscriptPage;
use serde::Deserialize;
use std::{
    io::{self, Write},
    sync::{Arc, OnceLock},
    time::Duration,
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGES: usize = 10_000;
const DEADLINE: Duration = Duration::from_secs(20);
static EXPORTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    format: TranscriptFormat,
}

pub(crate) async fn export(
    Extract(state): Extract<Arc<State>>,
    Path((project, thread_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Response, Error> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Read)?;
    let _permit = EXPORTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| CloudError::ResourceExhausted)?;
    let (bytes, name) = tokio::time::timeout(
        DEADLINE,
        collect(
            state.clone(),
            &headers,
            &caller,
            &project,
            &thread_id,
            input.format,
        ),
    )
    .await
    .map_err(|_| CloudError::ResourceExhausted)??;
    // Membership and exact personal/project identity remain required after buffering.
    let current = state.auth.caller(&headers, &project, true).await?;
    current.require(CloudPermission::Read)?;
    state
        .repo
        .read_thread_metadata(&current, &thread_id)
        .await?;
    let disposition = format!("attachment; filename*=UTF-8''{}", encoded_filename(&name));
    Ok((
        [
            (header::CONTENT_TYPE, input.format.media_type().to_owned()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CONTENT_LENGTH, bytes.len().to_string()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (
                header::HeaderName::from_static("x-colossus-export-complete"),
                "1".to_owned(),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn collect(
    state: Arc<State>,
    headers: &HeaderMap,
    caller: &CloudCaller,
    project: &str,
    thread_id: &str,
    format: TranscriptFormat,
) -> Result<(Vec<u8>, String), Error> {
    let (thread, source_run_id) = source(&state, caller, thread_id).await?;
    let expected_session = thread.session_id.as_deref().ok_or(CloudError::NotFound)?;
    let (generation, reply) = crate::resources::transcript_page(
        state.clone(),
        headers.clone(),
        project.into(),
        thread.node_id.clone(),
        None,
        ResourceOperation::Context,
    )
    .await?;
    if !matches!(reply, ResourceReply::Result { .. }) {
        return Err(CloudError::Conflict.into());
    }
    let mut cursor = None;
    let mut writer: Option<TranscriptWriter<ExportBytes>> = None;
    let mut name = None;
    for _ in 0..MAX_PAGES {
        let current = state.auth.caller(headers, project, true).await?;
        current.require(CloudPermission::Read)?;
        let selected = state.repo.read_thread_metadata(&current, thread_id).await?;
        if selected.node_id != thread.node_id || selected.session_id != thread.session_id {
            return Err(CloudError::Conflict.into());
        }
        let (observed_generation, reply) = crate::resources::transcript_page(
            state.clone(),
            headers.clone(),
            project.into(),
            thread.node_id.clone(),
            Some(generation.clone()),
            ResourceOperation::ReadSessionTranscript {
                source_run_id: source_run_id.clone(),
                page_size: 32,
                page_token: cursor.clone(),
            },
        )
        .await?;
        if observed_generation != generation {
            return Err(CloudError::Conflict.into());
        }
        let page: TranscriptPage = match reply {
            ResourceReply::Result { value } => {
                serde_json::from_value(value).map_err(|_| CloudError::Storage)?
            }
            ResourceReply::Failed { error } => return Err(map_api(error).into()),
        };
        if page.snapshot.session_id != expected_session {
            return Err(CloudError::Conflict.into());
        }
        if writer.is_none() {
            name = Some(transcript_filename(
                page.snapshot.title.as_deref().unwrap_or("Conversation"),
                expected_session,
                format,
            ));
            writer = Some(
                TranscriptWriter::new(ExportBytes::default(), format, &page.snapshot)
                    .map_err(|_| CloudError::Storage)?,
            );
        }
        writer
            .as_mut()
            .ok_or(CloudError::Storage)?
            .write_page(&page)
            .map_err(|_| CloudError::ResourceExhausted)?;
        cursor = page.next_cursor;
        if cursor.is_none() {
            let output = writer
                .ok_or(CloudError::Storage)?
                .finish()
                .map_err(|_| CloudError::Storage)?;
            return Ok((output.0, name.ok_or(CloudError::Storage)?));
        }
    }
    Err(CloudError::ResourceExhausted.into())
}

async fn source(
    state: &State,
    caller: &CloudCaller,
    thread_id: &str,
) -> Result<(CloudThread, String), Error> {
    let mut after = None;
    for _ in 0..100 {
        let detail = state
            .repo
            .thread_detail(caller, thread_id, after.as_deref(), None)
            .await?;
        if let Some(run_id) = detail.tasks.iter().find_map(|task| {
            (task.node_id == detail.thread.node_id && task.thread_id.as_deref() == Some(thread_id))
                .then_some(task.run_id.as_ref())
                .flatten()
                .filter(|id| !id.is_empty())
        }) {
            return Ok((detail.thread, run_id.clone()));
        }
        after = detail.next_task_cursor;
        if after.is_none() {
            return Err(CloudError::NotFound.into());
        }
    }
    Err(CloudError::ResourceExhausted.into())
}

#[derive(Default)]
struct ExportBytes(Vec<u8>);
impl Write for ExportBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other(
                "conversation exceeds browser export size limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encoded_filename(name: &str) -> String {
    let mut result = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            result.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(result, "%{byte:02X}");
        }
    }
    result
}

fn map_api(error: colossus_sdk::ApiError) -> CloudError {
    match error.code {
        colossus_sdk::ApiErrorCode::NotFound => CloudError::NotFound,
        colossus_sdk::ApiErrorCode::PermissionDenied => CloudError::PermissionDenied,
        colossus_sdk::ApiErrorCode::InvalidArgument => CloudError::InvalidArgument,
        colossus_sdk::ApiErrorCode::Conflict | colossus_sdk::ApiErrorCode::FailedPrecondition => {
            CloudError::Conflict
        }
        colossus_sdk::ApiErrorCode::ResourceExhausted => CloudError::ResourceExhausted,
        _ => CloudError::Storage,
    }
}

#[cfg(test)]
mod tests;
