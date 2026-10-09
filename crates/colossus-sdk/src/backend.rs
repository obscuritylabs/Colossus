use async_trait::async_trait;
#[cfg(feature = "embedded")]
use colossus_api::{AgentRunApi, CallerContext};
#[cfg(feature = "embedded")]
use futures::StreamExt as _;
#[cfg(feature = "embedded")]
use std::fmt;
use std::sync::Arc;

use crate::{
    ApiResult, ArchiveThreadRequest, ArtifactReference, CancelRunRequest, CancelRunResponse,
    CreateRunRequest, CreateRunResponse, DownloadedArtifact, GetRunRequest, GetRunResponse,
    ListRunsRequest, ListRunsResponse, ListSessionActivityRequest, ListSessionActivityResponse,
    RespondInteractionRequest, RespondInteractionResponse, RestoreThreadRequest, RunUpdateStream,
    SdkResult, ServerCapabilities, ThreadLifecycle, UploadArtifactRequest, WatchRunRequest,
};

/// Runtime placement used by this client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackendKind {
    /// Authenticated loopback connection to the persistent installed daemon.
    Daemon,
    /// Authenticated connection to an isolated application-bundled child process.
    Sidecar,
    /// Direct in-process composition over an application-private instance.
    Embedded,
}

/// Caller-bound agent run operations exposed to SDK consumers.
///
/// The caller context is deliberately absent: daemon transports derive it from the
/// authenticated credential, while embedded composition binds one trusted application
/// context before exposing this interface.
#[async_trait]
pub trait AgentRunClient: Send + Sync {
    /// Read metadata-only current policy posture beneath this application's authority.
    async fn get_runtime_policy_posture(&self) -> ApiResult<crate::RuntimePolicyPosture> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "runtime policy metadata is unavailable",
        ))
    }
    /// Explicitly share only this application's sessions in the connected workspace.
    async fn set_workspace_sharing(
        &self,
        _request: crate::SetWorkspaceSharingRequest,
    ) -> ApiResult<crate::WorkspaceSharingState> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "workspace sharing is unavailable",
        ))
    }

    /// Discover runtime-authorized shared sessions with explicit mutation flags.
    async fn list_visible_runs(
        &self,
        request: ListRunsRequest,
    ) -> ApiResult<crate::ListVisibleRunsResponse> {
        let page = self.list_runs(request).await?;
        Ok(crate::ListVisibleRunsResponse {
            runs: page
                .runs
                .into_iter()
                .map(|run| crate::VisibleRun {
                    run,
                    controllable: false,
                    continuable: false,
                })
                .collect(),
            page: page.page,
        })
    }
    /// List caller-owned managed shells.
    async fn list_process_sessions(
        &self,
        _request: crate::ListProcessSessionsRequest,
    ) -> ApiResult<crate::ProcessSessionPage> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "managed shell sessions are unavailable",
        ))
    }
    /// Read or wait for released shell output.
    async fn read_process_session(
        &self,
        _request: crate::ReadProcessSessionRequest,
    ) -> ApiResult<crate::ProcessSessionSnapshot> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "managed shell sessions are unavailable",
        ))
    }
    /// Idempotently request stop of one caller-owned shell.
    async fn stop_process_session(
        &self,
        _request: crate::StopProcessSessionRequest,
    ) -> ApiResult<crate::ProcessSessionSnapshot> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "managed shell sessions are unavailable",
        ))
    }

    /// Create one durable run idempotently.
    async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse>;

    /// Fetch one caller-visible run.
    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse>;

    /// List caller-visible runs with stable pagination.
    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse>;

    /// Return caller-visible canonical session activity.
    async fn list_session_activity(
        &self,
        _request: ListSessionActivityRequest,
    ) -> ApiResult<ListSessionActivityResponse> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "the connected backend does not support session activity",
        ))
    }

    /// Replay and then tail durable run updates.
    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream>;

    /// Whether this caller-bound client has been explicitly closed.
    ///
    /// Transport clients override this so resilient read-only watches do not reconnect
    /// after application shutdown. Custom clients may keep the default when they have
    /// no independent close lifecycle.
    fn is_closed(&self) -> bool {
        false
    }

    /// Wait until this caller-bound client is explicitly closed.
    ///
    /// The default never resolves. Transport clients with a close lifecycle override
    /// it so reconnect backoff can be interrupted immediately.
    async fn wait_closed(&self) {
        std::future::pending::<()>().await;
    }

    /// Request idempotent cooperative cancellation.
    async fn cancel_run(&self, request: CancelRunRequest) -> ApiResult<CancelRunResponse>;

    /// Hide one terminal thread from normal listings.
    async fn archive_thread(&self, _request: ArchiveThreadRequest) -> ApiResult<ThreadLifecycle> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "the connected backend does not support thread archiving",
        ))
    }

    /// Return one archived thread to normal listings.
    async fn restore_thread(&self, _request: RestoreThreadRequest) -> ApiResult<ThreadLifecycle> {
        Err(crate::ApiError::failed_precondition(
            crate::ApiErrorReason::InvalidRunTransition,
            "the connected backend does not support thread restoration",
        ))
    }

    /// Submit a one-use answer to a caller-bound interaction.
    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse>;
}

/// Caller-bound released artifact operations.
#[async_trait]
pub trait ArtifactClient: Send + Sync {
    /// Reserve, upload, verify, and release one complete bounded artifact.
    async fn upload(&self, request: UploadArtifactRequest) -> ApiResult<ArtifactReference>;

    /// Fetch caller-visible artifact metadata.
    async fn get(&self, artifact_id: &str) -> ApiResult<ArtifactReference>;

    /// Download and verify complete released bytes.
    async fn download(&self, artifact_id: &str) -> ApiResult<DownloadedArtifact>;
}

/// Bind a trusted embedded caller context to the transport-neutral public API.
///
/// Daemon clients must not use this adapter: their server creates caller identity from
/// authenticated connection state. It is public so trusted Rust composition crates can
/// build the embedded backend without exposing caller identity to WebView code.
#[cfg(feature = "embedded")]
pub struct ContextBoundAgentRunClient {
    api: Arc<dyn AgentRunApi>,
    caller: CallerContext,
}

#[cfg(feature = "embedded")]
impl ContextBoundAgentRunClient {
    /// Bind one server-created application context to an API implementation.
    pub fn new(api: Arc<dyn AgentRunApi>, caller: CallerContext) -> Self {
        Self { api, caller }
    }
}

#[cfg(feature = "embedded")]
impl fmt::Debug for ContextBoundAgentRunClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextBoundAgentRunClient")
            .finish_non_exhaustive()
    }
}

#[async_trait]
#[cfg(feature = "embedded")]
impl AgentRunClient for ContextBoundAgentRunClient {
    async fn get_runtime_policy_posture(&self) -> ApiResult<crate::RuntimePolicyPosture> {
        self.api.get_runtime_policy_posture(&self.caller).await
    }
    async fn set_workspace_sharing(
        &self,
        request: crate::SetWorkspaceSharingRequest,
    ) -> ApiResult<crate::WorkspaceSharingState> {
        self.api.set_workspace_sharing(&self.caller, request).await
    }
    async fn list_visible_runs(
        &self,
        request: ListRunsRequest,
    ) -> ApiResult<crate::ListVisibleRunsResponse> {
        let page = self
            .api
            .list_visible_runs(
                &self.caller,
                crate::embedded_projection::list_request(request),
            )
            .await?;
        let runs = page
            .runs
            .into_iter()
            .map(|value| {
                Ok(crate::VisibleRun {
                    run: crate::embedded_projection::get_response(value.run, &self.caller)?.run,
                    controllable: value.controllable,
                    continuable: value.continuable,
                })
            })
            .collect::<ApiResult<Vec<_>>>()?;
        Ok(crate::ListVisibleRunsResponse {
            runs,
            page: page
                .next_page_token
                .map(|next_page_token| crate::PageResponse { next_page_token }),
        })
    }
    async fn list_process_sessions(
        &self,
        request: crate::ListProcessSessionsRequest,
    ) -> ApiResult<crate::ProcessSessionPage> {
        self.api.list_process_sessions(&self.caller, request).await
    }
    async fn read_process_session(
        &self,
        request: crate::ReadProcessSessionRequest,
    ) -> ApiResult<crate::ProcessSessionSnapshot> {
        self.api.read_process_session(&self.caller, request).await
    }
    async fn stop_process_session(
        &self,
        request: crate::StopProcessSessionRequest,
    ) -> ApiResult<crate::ProcessSessionSnapshot> {
        self.api.stop_process_session(&self.caller, request).await
    }

    async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        let response = self
            .api
            .create_run(
                &self.caller,
                crate::embedded_projection::create_request(request),
            )
            .await?;
        crate::embedded_projection::create_response(response)
    }

    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        let run = self
            .api
            .get_run(
                &self.caller,
                colossus_api::GetRunRequest {
                    run_id: request.run_id,
                },
            )
            .await?;
        crate::embedded_projection::get_response(run, &self.caller)
    }

    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        let response = self
            .api
            .list_runs(
                &self.caller,
                crate::embedded_projection::list_request(request),
            )
            .await?;
        crate::embedded_projection::list_response(response)
    }

    async fn list_session_activity(
        &self,
        request: ListSessionActivityRequest,
    ) -> ApiResult<ListSessionActivityResponse> {
        let response = self
            .api
            .list_session_activity(
                &self.caller,
                crate::embedded_projection::activity_request(request),
            )
            .await?;
        Ok(crate::embedded_projection::activity_response(response))
    }

    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        let stream = self
            .api
            .watch_run(
                &self.caller,
                colossus_api::WatchRunRequest {
                    run_id: request.run_id,
                    after_sequence: request.after_sequence,
                },
            )
            .await?;
        let api = Arc::clone(&self.api);
        let caller = self.caller.clone();
        let stream = stream.then(move |item| {
            let api = Arc::clone(&api);
            let caller = caller.clone();
            async move {
                let update = item?;
                let interaction_etag =
                    if matches!(&update.kind, colossus_api::RunUpdateKind::Interaction {
                        interaction
                    } if interaction.status == colossus_api::InteractionStatus::Pending)
                    {
                        let current = api
                            .get_run(
                                &caller,
                                colossus_api::GetRunRequest {
                                    run_id: update.run_id.clone(),
                                },
                            )
                            .await?;
                        current
                            .pending_interaction
                            .as_ref()
                            .filter(|pending| {
                                matches!(&update.kind, colossus_api::RunUpdateKind::Interaction {
                                    interaction
                                } if pending.id == interaction.id)
                            })
                            .map(|_| current.etag)
                    } else {
                        None
                    };
                crate::embedded_projection::run_update(
                    update,
                    interaction_etag.as_deref(),
                    &caller,
                )
            }
        });
        Ok(Box::pin(stream))
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        let run = self
            .api
            .cancel_run(
                &self.caller,
                colossus_api::CancelRunRequest {
                    run_id: request.run_id,
                    idempotency_key: request.idempotency_key,
                },
            )
            .await?;
        crate::embedded_projection::cancel_response(run)
    }

    async fn archive_thread(&self, request: ArchiveThreadRequest) -> ApiResult<ThreadLifecycle> {
        let lifecycle = self
            .api
            .archive_thread(
                &self.caller,
                colossus_api::ArchiveThreadRequest {
                    run_id: request.run_id,
                    idempotency_key: request.idempotency_key,
                },
            )
            .await?;
        Ok(crate::embedded_projection::thread_lifecycle(lifecycle))
    }

    async fn restore_thread(&self, request: RestoreThreadRequest) -> ApiResult<ThreadLifecycle> {
        let lifecycle = self
            .api
            .restore_thread(
                &self.caller,
                colossus_api::RestoreThreadRequest {
                    run_id: request.run_id,
                    idempotency_key: request.idempotency_key,
                },
            )
            .await?;
        Ok(crate::embedded_projection::thread_lifecycle(lifecycle))
    }

    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        let run_id = request.run_id.clone();
        let request = crate::embedded_projection::interaction_request(request)?;
        let interaction = self.api.respond_interaction(&self.caller, request).await?;
        crate::embedded_projection::interaction_response(interaction, &run_id, &self.caller)
    }
}

/// Backend owned by a `Colossus` client.
///
/// `close` closes only the client channel for a shared daemon. Sidecar and embedded
/// implementations additionally supervise clean child/runtime shutdown.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Placement and lifecycle semantics of this backend.
    fn kind(&self) -> BackendKind;

    /// Caller-bound run service.
    fn agent_runs(&self) -> Arc<dyn AgentRunClient>;

    /// Authenticated runtime instance, when this backend has a native identity.
    fn instance_id(&self) -> Option<crate::InstanceId> {
        None
    }

    /// Independent cloud run client, present only after native cloud grant provisioning.
    fn connector_runs(&self) -> Option<Arc<dyn AgentRunClient>> {
        None
    }

    /// Independent cloud workflow resources; never falls back to primary authority.
    fn connector_workflows(&self) -> Option<Arc<dyn crate::WorkflowClient>> {
        None
    }
    /// Independent cloud plugin reads; never falls back to primary authority.
    fn connector_plugins(&self) -> Option<Arc<dyn crate::PluginClient>> {
        None
    }
    /// Capabilities of the independent cloud application connection.
    fn connector_capabilities(&self) -> ServerCapabilities {
        ServerCapabilities::default()
    }

    /// Cached authenticated server capabilities.
    ///
    /// Custom and preview-era embedded backends default to no optional behaviors.
    fn capabilities(&self) -> ServerCapabilities {
        ServerCapabilities::default()
    }

    /// Caller-bound artifact service when advertised.
    fn artifacts(&self) -> Option<Arc<dyn ArtifactClient>> {
        None
    }

    /// Caller-bound plugin reads, available only with explicit capability support.
    fn plugins(&self) -> Option<Arc<dyn crate::PluginClient>> {
        None
    }

    /// Caller-bound workflow resources when explicitly advertised.
    fn workflows(&self) -> Option<Arc<dyn crate::WorkflowClient>> {
        None
    }

    /// Close this client or isolated runtime idempotently.
    async fn close(&self) -> SdkResult<()>;
}
