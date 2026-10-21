//! Tenant-owned file data. No request can name or grant a worker filesystem path.
use super::management::Management;
use super::*;
use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub(super) const MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum FileOperation {
    Upload {
        file_name: String,
        media_type: String,
        content_base64: String,
        idempotency_key: String,
    },
    Download {
        artifact_id: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    file_name: String,
    media_type: String,
    content_base64: String,
    idempotency_key: String,
}

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/projects/{project}/hosted/files", post(upload))
        .route(
            "/api/projects/{project}/hosted/files/{artifact}",
            get(download),
        )
}

async fn upload(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Upload>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    let operation = FileOperation::Upload {
        file_name: input.file_name,
        media_type: input.media_type,
        content_base64: input.content_base64,
        idempotency_key: input.idempotency_key,
    };
    validate(&operation)?;
    let value = service(&state)?
        .file_operation(&state, &caller, operation)
        .await?;
    // Recheck membership before releasing file metadata after a potentially remote operation.
    state
        .auth
        .caller(&headers, &project, true)
        .await?
        .require(CloudPermission::Execute)?;
    Ok(Json(value))
}

async fn download(
    Extract(state): Extract<Arc<State>>,
    Path((project, artifact)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    colossus_cloud::validate_identifier(&artifact)?;
    let value = service(&state)?
        .file_operation(
            &state,
            &caller,
            FileOperation::Download {
                artifact_id: artifact,
            },
        )
        .await?;
    state
        .auth
        .caller(&headers, &project, false)
        .await?
        .require(CloudPermission::Read)?;
    let encoded = value["content_base64"]
        .as_str()
        .ok_or(CloudError::Storage)?;
    let bytes = STANDARD.decode(encoded).map_err(|_| CloudError::Storage)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(CloudError::ResourceExhausted.into());
    }
    // Attachment disposition and a fixed media type prevent active uploaded content
    // from acquiring this application's browser origin.
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, "attachment; filename=download"),
        ],
        bytes,
    )
        .into_response())
}

fn validate(operation: &FileOperation) -> std::result::Result<(), CloudError> {
    match operation {
        FileOperation::Upload {
            content_base64,
            media_type,
            ..
        } => {
            if content_base64.len() > MAX_FILE_BYTES.div_ceil(3) * 4
                || !matches!(
                    media_type.as_str(),
                    "text/plain" | "text/markdown" | "text/csv" | "application/json"
                )
            {
                return Err(CloudError::InvalidArgument);
            }
            let bytes = STANDARD
                .decode(content_base64)
                .map_err(|_| CloudError::InvalidArgument)?;
            if bytes.len() > MAX_FILE_BYTES || std::str::from_utf8(&bytes).is_err() {
                return Err(CloudError::InvalidArgument);
            }
        }
        FileOperation::Download { artifact_id } => {
            colossus_cloud::validate_identifier(artifact_id)?
        }
    }
    Ok(())
}

impl HostedExecution {
    async fn file_operation(
        &self,
        state: &Arc<State>,
        caller: &CloudCaller,
        operation: FileOperation,
    ) -> std::result::Result<serde_json::Value, CloudError> {
        validate(&operation)?;
        caller.require(CloudPermission::Read)?;
        // Hydration of owned file custody is a host operation. It does not admit an
        // agent run or expose the broader internal run client to a read-only caller.
        let loader = CloudCaller::new(
            caller.subject().into(),
            caller.project_id().into(),
            [CloudPermission::Read, CloudPermission::Execute]
                .into_iter()
                .collect(),
        )?;
        self.ensure(state, &loader).await?;
        let _lifecycle = self.lifecycle.lock().await;
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        let live = self.live.lock().await.get(&identity).cloned();
        if let Some(live) = live {
            if *live.shutdown.borrow() {
                return Err(CloudError::Conflict);
            }
            if live.project_id != caller.project_id() || live.user_id != caller.subject() {
                return Err(CloudError::PermissionDenied);
            }
            if let (Some(shared), Some(lease)) = (&self.shared, &live.lease) {
                shared.sql.verify_lease(lease)?;
            }
            let value = Self::handle_file_operation(&live, operation).await?;
            if let (Some(shared), Some(lease)) = (&self.shared, &live.lease) {
                shared.sql.verify_lease(lease)?;
            }
            return Ok(value);
        }
        let lease = self
            .shared
            .as_ref()
            .ok_or(CloudError::Storage)?
            .sql
            .read_lease(&identity)?;
        if lease.project_id != caller.project_id() || lease.user_id != caller.subject() {
            return Err(CloudError::PermissionDenied);
        }
        self.management_request(&lease, Management::File { file: operation })
            .await
    }

    pub(super) async fn handle_file_operation(
        live: &LiveRuntime,
        operation: FileOperation,
    ) -> std::result::Result<serde_json::Value, CloudError> {
        validate(&operation)?;
        let _operation = live.pin();
        match operation {
            FileOperation::Upload {
                file_name,
                media_type,
                content_base64,
                idempotency_key,
            } => {
                let bytes = STANDARD
                    .decode(content_base64)
                    .map_err(|_| CloudError::InvalidArgument)?;
                let key = colossus_api::IdempotencyKey::new(idempotency_key)
                    .map_err(|_| CloudError::InvalidArgument)?;
                let artifact = live
                    .runtime
                    .upload_file(file_name, media_type, bytes, key)
                    .await
                    .map_err(map_api_error)?;
                Ok(serde_json::json!({"artifact": artifact}))
            }
            FileOperation::Download { artifact_id } => {
                let file = live
                    .runtime
                    .download_file(&artifact_id)
                    .await
                    .map_err(map_api_error)?;
                if file.bytes.len() > MAX_FILE_BYTES {
                    return Err(CloudError::ResourceExhausted);
                }
                Ok(
                    serde_json::json!({"artifact": file.artifact, "content_base64": STANDARD.encode(file.bytes)}),
                )
            }
        }
    }
}

fn map_api_error(error: colossus_api::ApiError) -> CloudError {
    match error.code {
        colossus_api::ApiErrorCode::NotFound => CloudError::NotFound,
        colossus_api::ApiErrorCode::PermissionDenied => CloudError::PermissionDenied,
        colossus_api::ApiErrorCode::InvalidArgument => CloudError::InvalidArgument,
        colossus_api::ApiErrorCode::Conflict => CloudError::Conflict,
        colossus_api::ApiErrorCode::ResourceExhausted => CloudError::ResourceExhausted,
        _ => CloudError::Storage,
    }
}
