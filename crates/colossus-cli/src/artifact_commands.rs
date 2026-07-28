use super::*;
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, ArtifactApi, ArtifactChunk, ArtifactDownload,
    ArtifactPurpose, ArtifactReference, CallerContext, CreateArtifactUploadRequest,
    EventSourcedArtifactApi, IdempotencyKey, RequestId, scopes as api_scopes,
};
use sha2::{Digest as _, Sha256};

pub(super) async fn upload_artifact_file(
    runtime: &Runtime,
    path: &Path,
    purpose: ArtifactPurpose,
    idempotency_key: &str,
) -> Result<ArtifactReference, Box<dyn Error>> {
    let bytes = runtime.read_file_bytes(path).await?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| cli_error("artifact file name is invalid"))?
        .to_owned();
    let service = EventSourcedArtifactApi::new(runtime.journal());
    let caller = cli_artifact_caller()?;
    let reservation = service
        .create_upload(
            &caller,
            CreateArtifactUploadRequest {
                file_name,
                media_type: artifact_media_type(path).into(),
                size_bytes: u64::try_from(bytes.len())?,
                sha256: hex::encode(Sha256::digest(&bytes)),
                purpose,
                idempotency_key: IdempotencyKey::new(idempotency_key)?,
            },
        )
        .await?;
    let chunk_size = usize::try_from(reservation.chunk_size_bytes)?;
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
    Ok(service
        .upload(&caller, &reservation.upload_id, chunks)
        .await?)
}

pub(super) async fn get_artifact(
    runtime: &Runtime,
    artifact_id: &str,
) -> Result<ArtifactReference, Box<dyn Error>> {
    Ok(EventSourcedArtifactApi::new(runtime.journal())
        .get(&cli_artifact_caller()?, artifact_id)
        .await?)
}

pub(super) async fn download_artifact_file(
    runtime: &Runtime,
    artifact_id: &str,
    output: &Path,
) -> Result<ArtifactReference, Box<dyn Error>> {
    let ArtifactDownload { artifact, bytes } = EventSourcedArtifactApi::new(runtime.journal())
        .download(&cli_artifact_caller()?, artifact_id, 0)
        .await?;
    runtime.write_file_bytes(output, &bytes).await?;
    Ok(artifact)
}

fn cli_artifact_caller() -> Result<CallerContext, Box<dyn Error>> {
    let principal = ApplicationPrincipal::authenticated(
        "app:colossus-cli",
        "local-cli-interface",
        ApplicationKind::Embedded,
        [
            ApiScope::new(api_scopes::ARTIFACTS_READ)?,
            ApiScope::new(api_scopes::ARTIFACTS_WRITE)?,
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
