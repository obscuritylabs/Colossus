//! Managed bytes in the existing protected artifact journal, never host paths.
use crate::ChatRuntime;
use colossus_api::{
    ARTIFACT_CHUNK_BYTES, ApiResult, ArtifactApi, ArtifactChunk, ArtifactDownload, ArtifactPurpose,
    ArtifactReference, CreateArtifactUploadRequest, IdempotencyKey,
};
use sha2::{Digest, Sha256};

impl ChatRuntime {
    /// Store an authenticated user's input using the ordinary artifact ownership contract.
    /// File names are display metadata; no file is extracted onto a worker filesystem.
    pub async fn upload_file(
        &self,
        file_name: String,
        media_type: String,
        bytes: Vec<u8>,
        key: IdempotencyKey,
    ) -> ApiResult<ArtifactReference> {
        let upload = self
            .files
            .create_upload(
                &self.caller,
                CreateArtifactUploadRequest {
                    file_name,
                    media_type,
                    size_bytes: bytes.len() as u64,
                    sha256: hex::encode(Sha256::digest(&bytes)),
                    purpose: ArtifactPurpose::RunInput,
                    idempotency_key: key,
                },
            )
            .await?;
        let mut chunks = bytes
            .chunks(ARTIFACT_CHUNK_BYTES)
            .enumerate()
            .map(|(index, data)| ArtifactChunk {
                offset: (index * ARTIFACT_CHUNK_BYTES) as u64,
                data: data.to_vec(),
            })
            .collect::<Vec<_>>();
        if chunks.is_empty() {
            chunks.push(ArtifactChunk {
                offset: 0,
                data: vec![],
            });
        }
        self.files
            .upload(&self.caller, &upload.upload_id, chunks)
            .await
    }

    /// Fetch only an artifact owned by this exact project/user application binding.
    pub async fn download_file(&self, artifact_id: &str) -> ApiResult<ArtifactDownload> {
        self.files.download(&self.caller, artifact_id, 0).await
    }
}
