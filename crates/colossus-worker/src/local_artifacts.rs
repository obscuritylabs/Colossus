use super::*;

#[cfg(test)]
#[path = "local_artifacts_tests.rs"]
mod tests;
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, ArtifactApi, ArtifactChunk, ArtifactDownload,
    ArtifactReference, CallerContext, CreateArtifactUploadRequest, EventSourcedArtifactApi,
    IdempotencyKey, RequestId, scopes,
};
use sha2::{Digest as _, Sha256};
use std::path::Path;

pub(super) async fn upload_artifact_file(
    runtime: &Runtime,
    path: &Path,
    purpose: ArtifactPurpose,
    idempotency_key: &str,
) -> Result<ArtifactReference, WorkerError> {
    let bytes = runtime.read_file_bytes(path).await?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| WorkerError::Protocol("artifact file name is invalid".into()))?
        .to_owned();
    let service = EventSourcedArtifactApi::new(runtime.journal());
    let caller = cli_artifact_caller()?;
    let reservation = service
        .create_upload(
            &caller,
            CreateArtifactUploadRequest {
                file_name,
                media_type: artifact_media_type(path).into(),
                size_bytes: u64::try_from(bytes.len())
                    .map_err(|error| WorkerError::Protocol(error.to_string()))?,
                sha256: hex::encode(Sha256::digest(&bytes)),
                purpose,
                idempotency_key: IdempotencyKey::new(idempotency_key)?,
            },
        )
        .await?;
    let chunk_size = usize::try_from(reservation.chunk_size_bytes)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let chunks = bytes
        .chunks(chunk_size)
        .scan(0_u64, |offset, data| {
            let chunk = ArtifactChunk {
                offset: *offset,
                data: data.to_vec(),
            };
            *offset =
                offset.saturating_add(u64::from(u32::try_from(data.len()).unwrap_or(u32::MAX)));
            Some(chunk)
        })
        .collect();
    service
        .upload(&caller, &reservation.upload_id, chunks)
        .await
        .map_err(Into::into)
}

pub(super) async fn prepare_model_content(
    runtime: &Runtime,
    prompt: &str,
    attachments: &[PathBuf],
) -> Result<ModelContent, WorkerError> {
    if attachments.len() > 16 {
        return Err(WorkerError::Protocol(
            "at most 16 attachments may be supplied".into(),
        ));
    }
    if prompt.len() > 1_048_576 {
        return Err(WorkerError::Protocol(
            "combined prompt and attachments exceed 1 MiB".into(),
        ));
    }
    let mut parts = vec![ModelContentPart::Text {
        text: prompt.into(),
    }];
    let mut image_count = 0_usize;
    let mut image_bytes = 0_u64;
    let mut document_count = 0_usize;
    let mut document_bytes = 0_u64;
    let mut text_bytes = prompt.len();
    for path in attachments {
        let bytes = runtime.read_run_input_file_bytes(path).await?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| WorkerError::Protocol("attachment file name is invalid".into()))?;
        if colossus_runtime::document_media_type_for_file_name(file_name).is_some() {
            let validated = runtime.validate_run_input_document(file_name, None, &bytes)?;
            document_count += 1;
            document_bytes = document_bytes
                .checked_add(validated.size_bytes)
                .ok_or_else(|| WorkerError::Protocol("attachment size overflowed".into()))?;
            if document_count > colossus_contracts::MAX_DOCUMENT_INPUT_COUNT
                || document_bytes > colossus_contracts::MAX_COMBINED_DOCUMENT_INPUT_BYTES
            {
                return Err(WorkerError::Protocol(
                    "documents exceed the 4-file or 32 MiB combined bound".into(),
                ));
            }
            let key = format!(
                "local-document-{}",
                hex::encode(Sha256::digest(
                    format!("{file_name}\0{}", validated.sha256).as_bytes()
                ))
            );
            let artifact =
                upload_artifact_bytes(runtime, file_name, &validated.media_type, &bytes, &key)
                    .await?;
            let file =
                runtime.run_input_file_reference("app:colossus-cli", &artifact.artifact_id)?;
            parts.push(ModelContentPart::File { file });
        } else {
            match runtime.validate_run_input_image(file_name, None, &bytes) {
                Ok(validated) => {
                    image_count += 1;
                    image_bytes =
                        image_bytes
                            .checked_add(validated.size_bytes)
                            .ok_or_else(|| {
                                WorkerError::Protocol("attachment size overflowed".into())
                            })?;
                    if image_count > 16 || image_bytes > 32 * 1_048_576 {
                        return Err(WorkerError::Protocol(
                            "images exceed the 16-image or 32 MiB combined bound".into(),
                        ));
                    }
                    let image =
                        import_validated_image(runtime, file_name, &bytes, &validated).await?;
                    parts.push(ModelContentPart::Image { image });
                }
                Err(error) if image_candidate(path, &bytes) => return Err(error.into()),
                Err(_) => {
                    let text =
                        runtime.prompt_with_text_attachment_bytes("", &[(path.clone(), bytes)])?;
                    text_bytes = text_bytes.saturating_add(text.len());
                    if text_bytes > 1_048_576 {
                        return Err(WorkerError::Protocol(
                            "combined text attachments exceed 1 MiB".into(),
                        ));
                    }
                    parts.push(ModelContentPart::Text { text });
                }
            }
        }
    }
    if image_count == 0 && document_count == 0 {
        Ok(ModelContent::Text(
            parts
                .into_iter()
                .filter_map(|part| {
                    if let ModelContentPart::Text { text } = part {
                        Some(text)
                    } else {
                        None
                    }
                })
                .collect::<String>(),
        ))
    } else {
        Ok(ModelContent::Parts(parts))
    }
}

pub(super) async fn import_image_reference(
    runtime: &Runtime,
    path: &Path,
) -> Result<ModelImageReference, WorkerError> {
    let bytes = runtime.read_run_input_file_bytes(path).await?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| WorkerError::Protocol("attachment file name is invalid".into()))?;
    let validated = runtime.validate_run_input_image(file_name, None, &bytes)?;
    import_validated_image(runtime, file_name, &bytes, &validated).await
}

async fn import_validated_image(
    runtime: &Runtime,
    file_name: &str,
    bytes: &[u8],
    validated: &colossus_runtime::ValidatedImage,
) -> Result<ModelImageReference, WorkerError> {
    let artifact = upload_artifact_bytes(
        runtime,
        file_name,
        &validated.media_type,
        bytes,
        &format!("worker-image-{}", validated.sha256),
    )
    .await?;
    runtime
        .run_input_image_reference("app:colossus-cli", &artifact.artifact_id)
        .map_err(Into::into)
}

async fn upload_artifact_bytes(
    runtime: &Runtime,
    file_name: &str,
    media_type: &str,
    bytes: &[u8],
    idempotency_key: &str,
) -> Result<ArtifactReference, WorkerError> {
    let service = EventSourcedArtifactApi::new(runtime.journal());
    let caller = cli_artifact_caller()?;
    let reservation = service
        .create_upload(
            &caller,
            CreateArtifactUploadRequest {
                file_name: file_name.into(),
                media_type: media_type.into(),
                size_bytes: u64::try_from(bytes.len())
                    .map_err(|error| WorkerError::Protocol(error.to_string()))?,
                sha256: hex::encode(Sha256::digest(bytes)),
                purpose: ArtifactPurpose::RunInput,
                idempotency_key: IdempotencyKey::new(idempotency_key)?,
            },
        )
        .await?;
    let chunk_size = usize::try_from(reservation.chunk_size_bytes)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let chunks = bytes
        .chunks(chunk_size)
        .scan(0_u64, |offset, data| {
            let chunk = ArtifactChunk {
                offset: *offset,
                data: data.to_vec(),
            };
            *offset = offset.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
            Some(chunk)
        })
        .collect();
    service
        .upload(&caller, &reservation.upload_id, chunks)
        .await
        .map_err(Into::into)
}

fn image_candidate(path: &Path, bytes: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    matches!(
        extension.as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "svg")
    ) || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
        || bytes.starts_with(b"GIF8")
        || std::str::from_utf8(bytes)
            .ok()
            .is_some_and(|text| text.trim_start().starts_with("<svg"))
}

pub(super) async fn get_artifact(
    runtime: &Runtime,
    artifact_id: &str,
) -> Result<ArtifactReference, WorkerError> {
    EventSourcedArtifactApi::new(runtime.journal())
        .get(&cli_artifact_caller()?, artifact_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn download_artifact_file(
    runtime: &Runtime,
    artifact_id: &str,
    output: &Path,
) -> Result<ArtifactReference, WorkerError> {
    let ArtifactDownload { artifact, bytes } = EventSourcedArtifactApi::new(runtime.journal())
        .download(&cli_artifact_caller()?, artifact_id, 0)
        .await?;
    runtime.write_file_bytes(output, &bytes).await?;
    Ok(artifact)
}

fn cli_artifact_caller() -> Result<CallerContext, WorkerError> {
    let principal = ApplicationPrincipal::authenticated(
        "app:colossus-cli",
        "local-cli-interface",
        ApplicationKind::Embedded,
        [
            ApiScope::new(scopes::ARTIFACTS_READ)?,
            ApiScope::new(scopes::ARTIFACTS_WRITE)?,
        ],
        Vec::<String>::new(),
        Vec::<String>::new(),
    )?;
    Ok(CallerContext::authenticated(
        principal,
        RequestId::new(format!("cli-artifact-{}", Uuid::now_v7()))?,
    ))
}

fn artifact_media_type(path: &Path) -> &'static str {
    if let Some(media) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(colossus_runtime::document_media_type_for_file_name)
    {
        return media;
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => "application/json",
        Some("md") | Some("markdown") => "text/markdown",
        Some("yaml") | Some("yml") => "application/yaml",
        Some("toml") => "application/toml",
        Some("csv") => "text/csv",
        _ => "application/octet-stream",
    }
}
