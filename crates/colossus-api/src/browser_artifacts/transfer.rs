use super::*;
use colossus_ports::{
    BrowserDownloadPublication, BrowserReleasedArtifact, BrowserUploadArtifact,
    BrowserUploadArtifactRequest, BrowserUploadDescriptor, MAX_BROWSER_TRANSFER_BYTES,
};
use zeroize::Zeroizing;

fn caller(
    journal: &dyn EventJournal,
    binding: &colossus_contracts::BrowserSessionBinding,
    operation: &str,
    scopes: &[&str],
) -> Result<CallerContext, BrowserArtifactError> {
    let owner = super::ownership::resolve(journal, binding)?;
    let principal = ApplicationPrincipal::authenticated(
        &owner,
        "native-browser-artifact-custody",
        ApplicationKind::Embedded,
        scopes
            .iter()
            .map(|scope| ApiScope::new(*scope))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| BrowserArtifactError::Invalid)?,
        Vec::<String>::new(),
        Vec::<String>::new(),
    )
    .map_err(|_| BrowserArtifactError::Invalid)?;
    Ok(CallerContext::authenticated(
        principal,
        RequestId::new(operation).map_err(|_| BrowserArtifactError::Invalid)?,
    ))
}

pub(super) async fn resolve_upload(
    journal: Arc<dyn EventJournal>,
    request: BrowserUploadArtifactRequest,
) -> Result<BrowserUploadArtifact, BrowserArtifactError> {
    if request.run_id.is_empty() || request.run_id.len() > 128 {
        return Err(BrowserArtifactError::Invalid);
    }
    let caller = caller(
        journal.as_ref(),
        &request.binding,
        "browser-upload-input",
        &[scopes::ARTIFACTS_READ],
    )?;
    let service = EventSourcedArtifactApi::new(journal);
    let metadata = service
        .get(&caller, &request.artifact_id)
        .await
        .map_err(|_| BrowserArtifactError::Invalid)?;
    if !matches!(
        metadata.purpose,
        ArtifactPurpose::RunInput | ArtifactPurpose::RunOutput
    ) || metadata.size_bytes == 0
        || metadata.size_bytes > u64::from(MAX_BROWSER_TRANSFER_BYTES)
    {
        return Err(BrowserArtifactError::Invalid);
    }
    let downloaded = service
        .download(&caller, &request.artifact_id, 0)
        .await
        .map_err(|_| BrowserArtifactError::Invalid)?;
    let bytes = Zeroizing::new(downloaded.bytes);
    if downloaded.artifact != metadata
        || bytes.len() as u64 != metadata.size_bytes
        || hex::encode(Sha256::digest(&*bytes)) != metadata.sha256
    {
        return Err(BrowserArtifactError::Invalid);
    }
    // Explicit native certificate/key entry must not be routed through a browser upload.
    let name = metadata.file_name.to_ascii_lowercase();
    if [".pfx", ".p12", ".key", ".pem", ".jks", ".keystore"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || [b"PRIVATE KEY".as_slice(), b"OPENSSH PRIVATE KEY".as_slice()]
            .iter()
            .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
    {
        return Err(BrowserArtifactError::Invalid);
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    let file_name = extension.map_or_else(
        || "upload.bin".into(),
        |extension| format!("upload.{extension}"),
    );
    Ok(BrowserUploadArtifact {
        descriptor: BrowserUploadDescriptor {
            artifact_id: metadata.artifact_id,
            size_bytes: metadata.size_bytes as u32,
            sha256: metadata.sha256,
            file_name,
        },
        bytes,
    })
}

pub(super) async fn publish_download(
    journal: Arc<dyn EventJournal>,
    request: BrowserDownloadPublication,
) -> Result<BrowserReleasedArtifact, BrowserArtifactError> {
    let descriptor = &request.descriptor;
    if request.run_id.is_empty()
        || request.run_id.len() > 128
        || request.call_id.is_empty()
        || request.call_id.len() > 128
        || request.bytes.len() > MAX_BROWSER_TRANSFER_BYTES as usize
        || request.bytes.len() != descriptor.size_bytes as usize
        || hex::encode(Sha256::digest(&*request.bytes)) != descriptor.sha256
        || descriptor.control_generation == 0
        || descriptor.transfer_id.len() != 32
        || !descriptor
            .transfer_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(BrowserArtifactError::Invalid);
    }
    let mut identity = Sha256::new();
    identity.update(b"colossus-released-browser-download-v1\0");
    identity
        .update(serde_json::to_vec(&request.binding).map_err(|_| BrowserArtifactError::Invalid)?);
    identity.update(serde_json::to_vec(descriptor).map_err(|_| BrowserArtifactError::Invalid)?);
    for value in [&request.run_id, &request.call_id] {
        identity.update([0]);
        identity.update(value.as_bytes());
    }
    let identity = hex::encode(identity.finalize());
    let caller = caller(
        journal.as_ref(),
        &request.binding,
        &format!("browser-download-{identity}"),
        &[scopes::ARTIFACTS_WRITE],
    )?;
    let service = EventSourcedArtifactApi::new(journal);
    let reservation = service
        .create_upload(
            &caller,
            CreateArtifactUploadRequest {
                file_name: "browser-download.bin".into(),
                media_type: "application/octet-stream".into(),
                size_bytes: u64::from(descriptor.size_bytes),
                sha256: descriptor.sha256.clone(),
                purpose: ArtifactPurpose::RunOutput,
                idempotency_key: IdempotencyKey::new(format!("browser-download-{identity}"))
                    .map_err(|_| BrowserArtifactError::Invalid)?,
            },
        )
        .await
        .map_err(|_| BrowserArtifactError::OutcomeUnknown)?;
    let mut offset = 0;
    let chunks = request
        .bytes
        .chunks(reservation.chunk_size_bytes as usize)
        .map(|data| {
            let chunk = ArtifactChunk {
                offset,
                data: data.to_vec(),
            };
            offset += data.len() as u64;
            chunk
        })
        .collect();
    let artifact = service
        .upload(&caller, &reservation.upload_id, chunks)
        .await
        .map_err(|_| BrowserArtifactError::OutcomeUnknown)?;
    Ok(BrowserReleasedArtifact {
        artifact_id: artifact.artifact_id,
        file_name: artifact.file_name,
        media_type: artifact.media_type,
        size_bytes: artifact.size_bytes,
        sha256: artifact.sha256,
    })
}
