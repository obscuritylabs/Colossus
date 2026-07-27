use crate::{ApiError, ApiErrorReason, ApiResult, CallerContext, IdempotencyKey, scopes};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{EventClassification, ExecutionContext, NewEvent};
use colossus_ports::EventJournal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::sync::{Arc, Mutex, MutexGuard};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

/// Maximum verified content accepted by the initial public artifact service.
pub const MAX_ARTIFACT_BYTES: u64 = 16 * 1_048_576;
/// Maximum content in one transport chunk.
pub const ARTIFACT_CHUNK_BYTES: usize = 256 * 1024;
const UPLOAD_TTL_MINUTES: i64 = 15;
const RESERVED_EVENT: &str = "artifact.upload.reserved.v1";
const COMPLETED_EVENT: &str = "artifact.upload.completed.v1";
const AVAILABLE_EVENT: &str = "artifact.available.v1";

/// Intended caller-visible use of an artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPurpose {
    /// Content supplied to an agent run.
    RunInput,
    /// Released content produced by a run.
    RunOutput,
    /// Workflow definition or input.
    Workflow,
    /// Pack or extension archive.
    Extension,
    /// Exported Colossus archive.
    Archive,
}

/// Release state of an opaque artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    /// Verified bytes are available to the authenticated owner.
    Available,
}

/// Safe metadata for one opaque artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    /// Opaque stable identifier.
    pub artifact_id: String,
    /// Display name that is never interpreted as a server path.
    pub file_name: String,
    /// Normalized declared media type.
    pub media_type: String,
    /// Verified complete byte length.
    pub size_bytes: u64,
    /// Lowercase SHA-256 of the complete bytes.
    pub sha256: String,
    /// Validated intended use.
    pub purpose: ArtifactPurpose,
    /// Current release state.
    pub state: ArtifactState,
    /// UTC RFC3339 creation time.
    pub created_at: String,
}

/// Bounded reservation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateArtifactUploadRequest {
    /// Display name only.
    pub file_name: String,
    /// Declared media type.
    pub media_type: String,
    /// Exact expected byte length.
    pub size_bytes: u64,
    /// Exact expected lowercase SHA-256.
    pub sha256: String,
    /// Validated intended use.
    pub purpose: ArtifactPurpose,
    /// Caller-scoped idempotency key.
    pub idempotency_key: IdempotencyKey,
}

/// Expiring upload reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactUploadReservation {
    /// Opaque caller-bound upload identifier.
    pub upload_id: String,
    /// Maximum accepted bytes per ordered chunk.
    pub chunk_size_bytes: u32,
    /// UTC RFC3339 expiration.
    pub expires_at: String,
}

/// One ordered artifact chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactChunk {
    /// Exact zero-based offset.
    pub offset: u64,
    /// Bounded content bytes.
    pub data: Vec<u8>,
}

/// Released artifact bytes and their verified metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDownload {
    /// Verified safe metadata.
    pub artifact: ArtifactReference,
    /// Complete released bytes.
    pub bytes: Vec<u8>,
}

/// Transport-neutral caller-bound artifact operations.
#[async_trait]
pub trait ArtifactApi: Send + Sync {
    /// Reserve one expiring idempotent upload.
    async fn create_upload(
        &self,
        caller: &CallerContext,
        request: CreateArtifactUploadRequest,
    ) -> ApiResult<ArtifactUploadReservation>;

    /// Verify and atomically release one complete ordered upload.
    async fn upload(
        &self,
        caller: &CallerContext,
        upload_id: &str,
        chunks: Vec<ArtifactChunk>,
    ) -> ApiResult<ArtifactReference>;

    /// Return caller-visible metadata.
    async fn get(&self, caller: &CallerContext, artifact_id: &str) -> ApiResult<ArtifactReference>;

    /// Return released bytes beginning at the requested offset.
    async fn download(
        &self,
        caller: &CallerContext,
        artifact_id: &str,
        offset: u64,
    ) -> ApiResult<ArtifactDownload>;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReservation {
    file_name: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
    purpose: ArtifactPurpose,
    expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArtifact {
    artifact: ArtifactReference,
    content_base64: String,
}

/// Event-sourced artifact service over the encrypted authoritative journal.
pub struct EventSourcedArtifactApi {
    journal: Arc<dyn EventJournal>,
    writer: Mutex<()>,
}

impl EventSourcedArtifactApi {
    /// Bind the encrypted journal used by the runtime.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self {
            journal,
            writer: Mutex::new(()),
        }
    }

    fn lock(&self, caller: &CallerContext) -> ApiResult<MutexGuard<'_, ()>> {
        self.writer.lock().map_err(|_| {
            ApiError::internal("artifact storage is unavailable")
                .with_correlation_id(caller.request_id().clone())
        })
    }

    fn upload_id(caller: &CallerContext, key: &IdempotencyKey) -> String {
        let mut hash = Sha256::new();
        hash.update(b"colossus-artifact-upload-v1\0");
        hash.update(caller.principal().application_id().as_bytes());
        hash.update(b"\0");
        hash.update(key.as_str().as_bytes());
        format!("upload-{}", hex::encode(hash.finalize()))
    }

    fn artifact_id(upload_id: &str, sha256: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(b"colossus-artifact-v1\0");
        hash.update(upload_id.as_bytes());
        hash.update(b"\0");
        hash.update(sha256.as_bytes());
        format!("artifact-{}", hex::encode(hash.finalize()))
    }

    fn context(caller: &CallerContext) -> ExecutionContext {
        ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            ..ExecutionContext::default()
        }
    }

    fn event(
        caller: &CallerContext,
        stream_id: String,
        expected_stream_version: u64,
        event_type: &str,
        payload: Value,
    ) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor: caller.actor(),
            context: Self::context(caller),
            payload,
        }
    }

    fn reservation(
        &self,
        caller: &CallerContext,
        upload_id: &str,
    ) -> ApiResult<(StoredReservation, u64, Option<String>)> {
        let events = self
            .journal
            .read_stream(&format!("artifact-upload:{upload_id}"))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let first = events.first().filter(|event| {
            event.event_type == RESERVED_EVENT
                && event.actor.id == caller.principal().application_id()
        });
        let Some(first) = first else {
            return Err(artifact_not_found(caller));
        };
        let reservation = serde_json::from_value::<StoredReservation>(
            self.journal
                .decrypt_payload(first)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?,
        )
        .map_err(|_| invariant(caller))?;
        let completed = events
            .iter()
            .find(|event| event.event_type == COMPLETED_EVENT)
            .map(|event| {
                self.journal
                    .decrypt_payload(event)
                    .map_err(|error| ApiError::from_store(&error, caller.request_id()))
                    .and_then(|payload| {
                        payload
                            .get("artifact_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .ok_or_else(|| invariant(caller))
                    })
            })
            .transpose()?;
        Ok((
            reservation,
            u64::try_from(events.len()).map_err(|_| invariant(caller))?,
            completed,
        ))
    }

    fn stored_artifact(
        &self,
        caller: &CallerContext,
        artifact_id: &str,
    ) -> ApiResult<StoredArtifact> {
        validate_opaque_id(artifact_id, "artifact_id", "artifact-")?;
        let events = self
            .journal
            .read_stream(&format!("artifact:{artifact_id}"))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(event) = events.first().filter(|event| {
            event.event_type == AVAILABLE_EVENT
                && event.actor.id == caller.principal().application_id()
        }) else {
            return Err(artifact_not_found(caller));
        };
        serde_json::from_value(
            self.journal
                .decrypt_payload(event)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?,
        )
        .map_err(|_| invariant(caller))
    }
}

#[async_trait]
impl ArtifactApi for EventSourcedArtifactApi {
    async fn create_upload(
        &self,
        caller: &CallerContext,
        request: CreateArtifactUploadRequest,
    ) -> ApiResult<ArtifactUploadReservation> {
        caller.require_scope(scopes::ARTIFACTS_WRITE)?;
        validate_upload(&request)?;
        let _guard = self.lock(caller)?;
        let upload_id = Self::upload_id(caller, &request.idempotency_key);
        let stream_id = format!("artifact-upload:{upload_id}");
        let events = self
            .journal
            .read_stream(&stream_id)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        if let Some(first) = events.first() {
            if first.actor.id != caller.principal().application_id()
                || first.event_type != RESERVED_EVENT
            {
                return Err(invariant(caller));
            }
            let stored: StoredReservation = serde_json::from_value(
                self.journal
                    .decrypt_payload(first)
                    .map_err(|error| ApiError::from_store(&error, caller.request_id()))?,
            )
            .map_err(|_| invariant(caller))?;
            if stored.file_name != request.file_name
                || stored.media_type != request.media_type
                || stored.size_bytes != request.size_bytes
                || stored.sha256 != request.sha256
                || stored.purpose != request.purpose
            {
                return Err(ApiError::conflict(
                    ApiErrorReason::IdempotencyKeyReused,
                    "the idempotency key was already used for another artifact",
                )
                .with_correlation_id(caller.request_id().clone()));
            }
            return Ok(ArtifactUploadReservation {
                upload_id,
                chunk_size_bytes: ARTIFACT_CHUNK_BYTES as u32,
                expires_at: stored.expires_at,
            });
        }
        let expires_at = (OffsetDateTime::now_utc() + Duration::minutes(UPLOAD_TTL_MINUTES))
            .format(&Rfc3339)
            .map_err(|_| invariant(caller))?;
        let stored = StoredReservation {
            file_name: request.file_name,
            media_type: request.media_type,
            size_bytes: request.size_bytes,
            sha256: request.sha256,
            purpose: request.purpose,
            expires_at: expires_at.clone(),
        };
        self.journal
            .append(Self::event(
                caller,
                stream_id,
                0,
                RESERVED_EVENT,
                serde_json::to_value(stored).map_err(|_| invariant(caller))?,
            ))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        Ok(ArtifactUploadReservation {
            upload_id,
            chunk_size_bytes: ARTIFACT_CHUNK_BYTES as u32,
            expires_at,
        })
    }

    async fn upload(
        &self,
        caller: &CallerContext,
        upload_id: &str,
        chunks: Vec<ArtifactChunk>,
    ) -> ApiResult<ArtifactReference> {
        caller.require_scope(scopes::ARTIFACTS_WRITE)?;
        validate_opaque_id(upload_id, "upload_id", "upload-")?;
        let _guard = self.lock(caller)?;
        let (reservation, upload_version, completed) = self.reservation(caller, upload_id)?;
        if let Some(artifact_id) = completed {
            return Ok(self.stored_artifact(caller, &artifact_id)?.artifact);
        }
        let expires_at = OffsetDateTime::parse(&reservation.expires_at, &Rfc3339)
            .map_err(|_| invariant(caller))?;
        if OffsetDateTime::now_utc() > expires_at {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::ArtifactUnavailable,
                "the artifact upload reservation expired",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        if chunks.is_empty() && reservation.size_bytes != 0 {
            return Err(invalid("chunks", "at least one upload chunk is required"));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(reservation.size_bytes).map_err(|_| invariant(caller))?,
        );
        let mut next_offset = 0_u64;
        for chunk in chunks {
            if chunk.offset != next_offset {
                return Err(invalid(
                    "chunks.offset",
                    "upload chunks must be contiguous and ordered",
                ));
            }
            if chunk.data.len() > ARTIFACT_CHUNK_BYTES {
                return Err(invalid(
                    "chunks.data",
                    "an upload chunk exceeds the advertised size",
                ));
            }
            next_offset = next_offset
                .checked_add(u64::try_from(chunk.data.len()).map_err(|_| invariant(caller))?)
                .ok_or_else(|| invalid("chunks.data", "the upload size overflowed"))?;
            if next_offset > reservation.size_bytes {
                return Err(invalid(
                    "chunks.data",
                    "the upload exceeds its reserved size",
                ));
            }
            bytes.extend_from_slice(&chunk.data);
        }
        if next_offset != reservation.size_bytes {
            return Err(invalid(
                "chunks.data",
                "the upload does not match its reserved size",
            ));
        }
        let actual_sha256 = hex::encode(Sha256::digest(&bytes));
        if actual_sha256 != reservation.sha256 {
            return Err(invalid(
                "chunks.data",
                "the upload does not match its reserved SHA-256",
            ));
        }
        let artifact_id = Self::artifact_id(upload_id, &actual_sha256);
        let created_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| invariant(caller))?;
        let artifact = ArtifactReference {
            artifact_id: artifact_id.clone(),
            file_name: reservation.file_name,
            media_type: reservation.media_type,
            size_bytes: reservation.size_bytes,
            sha256: actual_sha256,
            purpose: reservation.purpose,
            state: ArtifactState::Available,
            created_at,
        };
        let stored = StoredArtifact {
            artifact: artifact.clone(),
            content_base64: BASE64.encode(bytes),
        };
        self.journal
            .append_batch(vec![
                Self::event(
                    caller,
                    format!("artifact:{artifact_id}"),
                    0,
                    AVAILABLE_EVENT,
                    serde_json::to_value(stored).map_err(|_| invariant(caller))?,
                ),
                Self::event(
                    caller,
                    format!("artifact-upload:{upload_id}"),
                    upload_version,
                    COMPLETED_EVENT,
                    json!({"artifact_id": artifact_id}),
                ),
            ])
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        Ok(artifact)
    }

    async fn get(&self, caller: &CallerContext, artifact_id: &str) -> ApiResult<ArtifactReference> {
        caller.require_scope(scopes::ARTIFACTS_READ)?;
        Ok(self.stored_artifact(caller, artifact_id)?.artifact)
    }

    async fn download(
        &self,
        caller: &CallerContext,
        artifact_id: &str,
        offset: u64,
    ) -> ApiResult<ArtifactDownload> {
        caller.require_scope(scopes::ARTIFACTS_READ)?;
        let stored = self.stored_artifact(caller, artifact_id)?;
        if offset > stored.artifact.size_bytes {
            return Err(invalid(
                "offset",
                "download offset exceeds the artifact size",
            ));
        }
        let bytes = BASE64
            .decode(stored.content_base64)
            .map_err(|_| invariant(caller))?;
        if u64::try_from(bytes.len()).ok() != Some(stored.artifact.size_bytes)
            || hex::encode(Sha256::digest(&bytes)) != stored.artifact.sha256
        {
            return Err(invariant(caller));
        }
        let offset = usize::try_from(offset).map_err(|_| invariant(caller))?;
        Ok(ArtifactDownload {
            artifact: stored.artifact,
            bytes: bytes[offset..].to_vec(),
        })
    }
}

fn validate_upload(request: &CreateArtifactUploadRequest) -> ApiResult<()> {
    if request.file_name.is_empty()
        || request.file_name.len() > 255
        || request.file_name.trim() != request.file_name
        || request.file_name.chars().any(char::is_control)
    {
        return Err(invalid(
            "file_name",
            "file_name must be a bounded safe display name",
        ));
    }
    if request.media_type.is_empty()
        || request.media_type.len() > 128
        || !request.media_type.is_ascii()
        || !request.media_type.contains('/')
        || request
            .media_type
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(invalid(
            "media_type",
            "media_type must be a bounded MIME type",
        ));
    }
    if request.size_bytes > MAX_ARTIFACT_BYTES {
        return Err(ApiError::bounded_resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "the artifact exceeds the configured size bound",
        ));
    }
    if request.sha256.len() != 64
        || !request
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("sha256", "sha256 must be lowercase hexadecimal"));
    }
    Ok(())
}

pub(crate) fn validate_artifact_id(value: &str, field: &str) -> ApiResult<()> {
    validate_opaque_id(value, field, "artifact-")
}

fn validate_opaque_id(value: &str, field: &str, prefix: &str) -> ApiResult<()> {
    let Some(digest) = value.strip_prefix(prefix) else {
        return Err(invalid(field, "the opaque identifier is invalid"));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(field, "the opaque identifier is invalid"));
    }
    Ok(())
}

fn invalid(field: &str, description: &str) -> ApiError {
    ApiError::invalid(ApiErrorReason::InvalidArgument, field, description)
}

fn artifact_not_found(caller: &CallerContext) -> ApiError {
    ApiError::not_found(
        ApiErrorReason::ArtifactNotFound,
        "the requested artifact was not found",
    )
    .with_correlation_id(caller.request_id().clone())
}

fn invariant(caller: &CallerContext) -> ApiError {
    ApiError::internal("the durable artifact record is invalid")
        .with_correlation_id(caller.request_id().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiScope, ApplicationKind, ApplicationPrincipal, RequestId};
    use colossus_testkit::InMemoryEventJournal;

    fn caller(application: &str, scopes: &[&str]) -> CallerContext {
        CallerContext::authenticated(
            ApplicationPrincipal::authenticated(
                application,
                "credential-artifacts",
                ApplicationKind::Sidecar,
                scopes
                    .iter()
                    .map(|scope| ApiScope::new(*scope).expect("scope")),
                ["primary".into()],
                Vec::<String>::new(),
            )
            .expect("principal"),
            RequestId::new("request-artifacts").expect("request"),
        )
    }

    #[tokio::test]
    async fn upload_is_verified_owner_bound_and_downloadable() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = EventSourcedArtifactApi::new(journal);
        let owner = caller(
            "app:artifact-owner",
            &[scopes::ARTIFACTS_WRITE, scopes::ARTIFACTS_READ],
        );
        let bytes = b"hello artifact".to_vec();
        let request = CreateArtifactUploadRequest {
            file_name: "hello.txt".into(),
            media_type: "text/plain".into(),
            size_bytes: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&bytes)),
            purpose: ArtifactPurpose::RunInput,
            idempotency_key: IdempotencyKey::new("artifact-example").expect("key"),
        };
        let reserved = service
            .create_upload(&owner, request.clone())
            .await
            .expect("reserve");
        assert_eq!(
            service
                .create_upload(&owner, request)
                .await
                .expect("replay"),
            reserved
        );
        let artifact = service
            .upload(
                &owner,
                &reserved.upload_id,
                vec![ArtifactChunk {
                    offset: 0,
                    data: bytes.clone(),
                }],
            )
            .await
            .expect("upload");
        let download = service
            .download(&owner, &artifact.artifact_id, 6)
            .await
            .expect("download");
        assert_eq!(download.bytes, b"artifact");

        let other = caller("app:artifact-other", &[scopes::ARTIFACTS_READ]);
        assert!(matches!(
            service.get(&other, &artifact.artifact_id).await,
            Err(ApiError {
                reason: ApiErrorReason::ArtifactNotFound,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn upload_rejects_wrong_digest_without_releasing_content() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = EventSourcedArtifactApi::new(journal);
        let owner = caller(
            "app:artifact-owner",
            &[scopes::ARTIFACTS_WRITE, scopes::ARTIFACTS_READ],
        );
        let reserved = service
            .create_upload(
                &owner,
                CreateArtifactUploadRequest {
                    file_name: "bad.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: 3,
                    sha256: "0".repeat(64),
                    purpose: ArtifactPurpose::RunInput,
                    idempotency_key: IdempotencyKey::new("artifact-bad").expect("key"),
                },
            )
            .await
            .expect("reserve");
        assert!(
            service
                .upload(
                    &owner,
                    &reserved.upload_id,
                    vec![ArtifactChunk {
                        offset: 0,
                        data: b"bad".to_vec(),
                    }],
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn empty_artifacts_are_released_and_path_like_identifiers_are_rejected() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let service = EventSourcedArtifactApi::new(journal);
        let owner = caller(
            "app:artifact-owner",
            &[scopes::ARTIFACTS_WRITE, scopes::ARTIFACTS_READ],
        );
        let reserved = service
            .create_upload(
                &owner,
                CreateArtifactUploadRequest {
                    file_name: "empty.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: 0,
                    sha256: hex::encode(Sha256::digest([])),
                    purpose: ArtifactPurpose::RunInput,
                    idempotency_key: IdempotencyKey::new("artifact-empty").expect("key"),
                },
            )
            .await
            .expect("reserve");
        let artifact = service
            .upload(&owner, &reserved.upload_id, Vec::new())
            .await
            .expect("empty upload");
        assert!(
            service
                .download(&owner, &artifact.artifact_id, 0)
                .await
                .expect("download")
                .bytes
                .is_empty()
        );
        assert_eq!(
            service
                .get(&owner, "../private/empty.txt")
                .await
                .expect_err("path-like artifact ID")
                .reason,
            ApiErrorReason::InvalidArgument
        );
    }
}
