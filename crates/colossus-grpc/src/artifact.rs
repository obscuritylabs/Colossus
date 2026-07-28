use crate::{status::api_status, system::caller_context};
use colossus_api::{
    ARTIFACT_CHUNK_BYTES, ArtifactApi, ArtifactChunk as CoreArtifactChunk,
    ArtifactPurpose as CoreArtifactPurpose, ArtifactReference as CoreArtifactReference,
    ArtifactState as CoreArtifactState,
    CreateArtifactUploadRequest as CoreCreateArtifactUploadRequest, IdempotencyKey,
    MAX_ARTIFACT_BYTES,
};
use colossus_api_proto::v1alpha1::{
    ArtifactPurpose, ArtifactReference, ArtifactState, CreateArtifactUploadRequest,
    CreateArtifactUploadResponse, DownloadArtifactRequest, DownloadArtifactResponse,
    GetArtifactRequest, GetArtifactResponse, UploadArtifactRequest, UploadArtifactResponse,
    artifact_service_server::ArtifactService,
};
use futures::Stream;
use prost_types::Timestamp;
use std::{pin::Pin, sync::Arc};
use tonic::{Request, Response, Status, Streaming};

const MAX_UPLOAD_CHUNKS: usize =
    (MAX_ARTIFACT_BYTES as usize / ARTIFACT_CHUNK_BYTES).saturating_add(1);

/// Authenticated bounded artifact transport.
#[derive(Clone)]
pub struct ArtifactServiceAdapter {
    api: Arc<dyn ArtifactApi>,
}

impl ArtifactServiceAdapter {
    /// Wrap one transport-neutral artifact service.
    pub fn new(api: Arc<dyn ArtifactApi>) -> Self {
        Self { api }
    }
}

#[tonic::async_trait]
impl ArtifactService for ArtifactServiceAdapter {
    type DownloadArtifactStream =
        Pin<Box<dyn Stream<Item = Result<DownloadArtifactResponse, Status>> + Send + 'static>>;

    async fn create_artifact_upload(
        &self,
        request: Request<CreateArtifactUploadRequest>,
    ) -> Result<Response<CreateArtifactUploadResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let idempotency_key = IdempotencyKey::new(request.idempotency_key).map_err(api_status)?;
        let reservation = self
            .api
            .create_upload(
                &caller,
                CoreCreateArtifactUploadRequest {
                    file_name: request.file_name,
                    media_type: request.media_type,
                    size_bytes: request.size_bytes,
                    sha256: request.sha256,
                    purpose: purpose_from_proto(request.purpose)?,
                    idempotency_key,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(CreateArtifactUploadResponse {
            upload_id: reservation.upload_id,
            chunk_size_bytes: reservation.chunk_size_bytes,
            expires_at: Some(parse_timestamp(&reservation.expires_at)?),
        }))
    }

    async fn upload_artifact(
        &self,
        request: Request<Streaming<UploadArtifactRequest>>,
    ) -> Result<Response<UploadArtifactResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let mut stream = request.into_inner();
        let mut upload_id: Option<String> = None;
        let mut chunks = Vec::new();
        let mut total = 0_u64;
        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|_| Status::invalid_argument("the artifact upload stream is invalid"))?
        {
            if chunks.len() >= MAX_UPLOAD_CHUNKS || chunk.data.len() > ARTIFACT_CHUNK_BYTES {
                return Err(Status::resource_exhausted(
                    "the artifact upload exceeds configured bounds",
                ));
            }
            match &upload_id {
                Some(expected) if expected != &chunk.upload_id => {
                    return Err(Status::invalid_argument(
                        "every artifact chunk must use the same upload ID",
                    ));
                }
                None => upload_id = Some(chunk.upload_id),
                Some(_) => {}
            }
            total = total
                .checked_add(
                    u64::try_from(chunk.data.len()).map_err(|_| {
                        Status::resource_exhausted("the artifact upload is too large")
                    })?,
                )
                .ok_or_else(|| Status::resource_exhausted("the artifact upload is too large"))?;
            if total > MAX_ARTIFACT_BYTES {
                return Err(Status::resource_exhausted(
                    "the artifact upload exceeds configured bounds",
                ));
            }
            chunks.push(CoreArtifactChunk {
                offset: chunk.offset,
                data: chunk.data,
            });
        }
        let upload_id = upload_id
            .ok_or_else(|| Status::invalid_argument("the artifact upload stream is empty"))?;
        let artifact = self
            .api
            .upload(&caller, &upload_id, chunks)
            .await
            .map_err(api_status)?;
        Ok(Response::new(UploadArtifactResponse {
            artifact: Some(reference_to_proto(artifact)?),
        }))
    }

    async fn get_artifact(
        &self,
        request: Request<GetArtifactRequest>,
    ) -> Result<Response<GetArtifactResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let artifact = self
            .api
            .get(&caller, &request.into_inner().artifact_id)
            .await
            .map_err(api_status)?;
        Ok(Response::new(GetArtifactResponse {
            artifact: Some(reference_to_proto(artifact)?),
        }))
    }

    async fn download_artifact(
        &self,
        request: Request<DownloadArtifactRequest>,
    ) -> Result<Response<Self::DownloadArtifactStream>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let start = request.offset;
        let download = self
            .api
            .download(&caller, &request.artifact_id, start)
            .await
            .map_err(api_status)?;
        let chunks = download
            .bytes
            .chunks(ARTIFACT_CHUNK_BYTES)
            .enumerate()
            .map(|(index, data)| {
                let relative = u64::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_mul(ARTIFACT_CHUNK_BYTES as u64))
                    .ok_or_else(|| Status::internal("artifact chunk offset overflowed"))?;
                Ok(DownloadArtifactResponse {
                    offset: start
                        .checked_add(relative)
                        .ok_or_else(|| Status::internal("artifact chunk offset overflowed"))?,
                    data: data.to_vec(),
                })
            })
            .collect::<Vec<_>>();
        Ok(Response::new(Box::pin(tokio_stream::iter(chunks))))
    }
}

fn purpose_from_proto(value: i32) -> Result<CoreArtifactPurpose, Status> {
    match ArtifactPurpose::try_from(value).ok() {
        Some(ArtifactPurpose::RunInput) => Ok(CoreArtifactPurpose::RunInput),
        Some(ArtifactPurpose::RunOutput) => Ok(CoreArtifactPurpose::RunOutput),
        Some(ArtifactPurpose::Workflow) => Ok(CoreArtifactPurpose::Workflow),
        Some(ArtifactPurpose::Extension) => Ok(CoreArtifactPurpose::Extension),
        Some(ArtifactPurpose::Archive) => Ok(CoreArtifactPurpose::Archive),
        Some(ArtifactPurpose::Unspecified) | None => {
            Err(Status::invalid_argument("artifact purpose is required"))
        }
    }
}

fn reference_to_proto(value: CoreArtifactReference) -> Result<ArtifactReference, Status> {
    let purpose = match value.purpose {
        CoreArtifactPurpose::RunInput => ArtifactPurpose::RunInput,
        CoreArtifactPurpose::RunOutput => ArtifactPurpose::RunOutput,
        CoreArtifactPurpose::Workflow => ArtifactPurpose::Workflow,
        CoreArtifactPurpose::Extension => ArtifactPurpose::Extension,
        CoreArtifactPurpose::Archive => ArtifactPurpose::Archive,
    };
    let state = match value.state {
        CoreArtifactState::Available => ArtifactState::Available,
    };
    Ok(ArtifactReference {
        artifact_id: value.artifact_id,
        file_name: value.file_name,
        media_type: value.media_type,
        size_bytes: value.size_bytes,
        sha256: value.sha256,
        purpose: purpose as i32,
        state: state as i32,
        created_at: Some(parse_timestamp(&value.created_at)?),
    })
}

fn parse_timestamp(value: &str) -> Result<Timestamp, Status> {
    value
        .parse::<Timestamp>()
        .map_err(|_| Status::internal("artifact timestamp is invalid"))
}
