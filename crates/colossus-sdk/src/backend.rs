use async_trait::async_trait;
#[cfg(feature = "embedded")]
use colossus_api::{AgentRunApi, CallerContext};
#[cfg(feature = "embedded")]
use futures::StreamExt as _;
#[cfg(feature = "embedded")]
use std::fmt;
use std::sync::Arc;

use crate::{
    ApiResult, CancelRunRequest, CancelRunResponse, CreateRunRequest, CreateRunResponse,
    GetRunRequest, GetRunResponse, ListRunsRequest, ListRunsResponse, RespondInteractionRequest,
    RespondInteractionResponse, RunUpdateStream, SdkResult, WatchRunRequest,
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
    /// Create one durable run idempotently.
    async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse>;

    /// Fetch one caller-visible run.
    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse>;

    /// List caller-visible runs with stable pagination.
    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse>;

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

    /// Submit a one-use answer to a caller-bound interaction.
    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse>;
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

    /// Close this client or isolated runtime idempotently.
    async fn close(&self) -> SdkResult<()>;
}
