use crate::{
    AgentRunClient, ApiError, ApiErrorCode, ApiErrorReason, ApiResult, ArtifactClient,
    ArtifactReference, Backend, BackendKind, CancelRunRequest, CancelRunResponse, CreateRunRequest,
    CreateRunResponse, DownloadedArtifact, GetRunRequest, GetRunResponse, ListRunsRequest,
    ListRunsResponse, PLAN_CONTINUATION_CAPABILITY, RespondInteractionRequest,
    RespondInteractionResponse, RunUpdates, SdkResult, ServerCapabilities, UploadArtifactRequest,
    WatchRunRequest,
};
use std::{fmt, sync::Arc};

/// Cloneable Rust SDK handle suitable for Tauri managed state.
///
/// Clones share one authenticated backend. Closing is defined by that backend and must
/// be idempotent; it never stops a shared daemon.
#[derive(Clone)]
pub struct Colossus {
    backend: Arc<dyn Backend>,
}

impl Colossus {
    /// Construct a client around a transport or embedded backend.
    pub fn from_backend(backend: impl Backend + 'static) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    /// Construct a client around a shared backend produced by a lifecycle adapter.
    pub fn from_shared_backend(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    /// Return the selected lifecycle backend.
    pub fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }

    /// Return a cloneable caller-bound run service.
    pub fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        self.backend.agent_runs()
    }

    /// Return authenticated optional behaviors cached during connection setup.
    pub fn capabilities(&self) -> ServerCapabilities {
        self.backend.capabilities()
    }

    /// Return the caller-bound artifact service when advertised.
    pub fn artifacts(&self) -> Option<Arc<dyn ArtifactClient>> {
        self.backend.artifacts()
    }

    /// Upload one complete bounded artifact.
    pub async fn upload_artifact(
        &self,
        request: UploadArtifactRequest,
    ) -> ApiResult<ArtifactReference> {
        self.backend
            .artifacts()
            .ok_or_else(artifact_service_unavailable)?
            .upload(request)
            .await
    }

    /// Fetch one artifact's metadata.
    pub async fn get_artifact(&self, artifact_id: &str) -> ApiResult<ArtifactReference> {
        self.backend
            .artifacts()
            .ok_or_else(artifact_service_unavailable)?
            .get(artifact_id)
            .await
    }

    /// Download one complete released artifact.
    pub async fn download_artifact(&self, artifact_id: &str) -> ApiResult<DownloadedArtifact> {
        self.backend
            .artifacts()
            .ok_or_else(artifact_service_unavailable)?
            .download(artifact_id)
            .await
    }

    /// Create a durable agent run.
    pub async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        if request.plan_action.is_some()
            && !self
                .backend
                .capabilities()
                .contains(PLAN_CONTINUATION_CAPABILITY)
        {
            return Err(plan_continuation_unavailable());
        }
        self.backend.agent_runs().create_run(request).await
    }

    /// Fetch one run.
    pub async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        self.backend.agent_runs().get_run(request).await
    }

    /// List runs with stable pagination.
    pub async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        self.backend.agent_runs().list_runs(request).await
    }

    /// Replay and tail run updates in order.
    pub async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdates> {
        let client = self.backend.agent_runs();
        let initial_stream = match client.watch_run(request.clone()).await {
            Ok(stream) => Some(stream),
            #[cfg(feature = "daemon")]
            Err(error) if error.code == crate::ApiErrorCode::Unavailable && error.retryable => None,
            Err(error) => return Err(error),
        };

        #[cfg(feature = "daemon")]
        if self.backend.kind() != BackendKind::Embedded {
            return Ok(RunUpdates::resilient(client, request, initial_stream));
        }

        Ok(RunUpdates::checked(
            client,
            initial_stream.expect("non-resilient watch requires an established stream"),
            request,
        ))
    }

    /// Request cooperative cancellation.
    pub async fn cancel_run(&self, request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        self.backend.agent_runs().cancel_run(request).await
    }

    /// Answer one pending prompt or approval exactly once.
    pub async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        self.backend.agent_runs().respond_interaction(request).await
    }

    /// Close this client or isolated backend.
    pub async fn close(&self) -> SdkResult<()> {
        self.backend.close().await
    }
}

fn artifact_service_unavailable() -> ApiError {
    ApiError {
        code: ApiErrorCode::FailedPrecondition,
        reason: ApiErrorReason::ArtifactUnavailable,
        message: "the connected runtime did not advertise artifact operations".into(),
        correlation_id: None,
        retryable: false,
        outcome: colossus_api::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

fn plan_continuation_unavailable() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InvalidRunTransition,
        "the connected runtime did not advertise typed Plan continuation",
    )
}

impl fmt::Debug for Colossus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Colossus")
            .field("backend_kind", &self.backend.kind())
            .finish_non_exhaustive()
    }
}
