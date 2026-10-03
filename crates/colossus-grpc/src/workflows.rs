//! Authenticated bounded resources backed by the shared canonical workflow service.
use crate::{status::api_status, system::caller_context, workflow_wire as wire};
use colossus_api::{WorkflowApi, scopes};
use colossus_api_proto::v1alpha1::{self as proto, automation_service_server::AutomationService};
use futures::Stream;
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tonic::{Request, Response, Status};

/// Transport adapter; credentials and the bound runtime determine authority.
#[derive(Clone)]
pub struct AutomationServiceAdapter {
    api: Option<Arc<dyn WorkflowApi>>,
    watch_slots: Arc<Semaphore>,
}
impl AutomationServiceAdapter {
    /// Compose optional workflow support with the server-wide watch ceiling.
    pub fn new(api: Option<Arc<dyn WorkflowApi>>, watch_slots: Arc<Semaphore>) -> Self {
        Self { api, watch_slots }
    }
    fn api(&self) -> Result<&dyn WorkflowApi, Status> {
        self.api.as_deref().ok_or_else(|| {
            Status::unimplemented("workflow resources are unavailable on this target")
        })
    }
}
fn page(value: Option<proto::PageRequest>) -> Result<(Option<String>, usize), Status> {
    let value = value.unwrap_or_default();
    if value.page_token.len() > 128 || value.page_size > 100 {
        return Err(Status::invalid_argument("invalid workflow page bounds"));
    }
    Ok((
        (!value.page_token.is_empty()).then_some(value.page_token),
        if value.page_size == 0 {
            32
        } else {
            value.page_size as usize
        },
    ))
}
fn bounded<T: prost::Message>(value: T) -> Result<Response<T>, Status> {
    if value.encoded_len() > 2 * 1024 * 1024 {
        return Err(Status::resource_exhausted(
            "workflow response exceeds its bound; request a smaller page",
        ));
    }
    Ok(Response::new(value))
}
fn mutation_output(_: Status) -> Status {
    Status::internal(
        "workflow mutation response could not be confirmed; reconcile the original intent",
    )
}

#[tonic::async_trait]
impl AutomationService for AutomationServiceAdapter {
    async fn get_workflow(
        &self,
        request: Request<proto::GetWorkflowRequest>,
    ) -> Result<Response<proto::GetWorkflowResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = self
            .api()?
            .get_workflow(caller, request.get_ref().workflow_id.clone())
            .await
            .map_err(api_status)?;
        bounded(proto::GetWorkflowResponse {
            workflow: Some(wire::definition(value)?),
        })
    }
    async fn list_workflows(
        &self,
        request: Request<proto::ListWorkflowsRequest>,
    ) -> Result<Response<proto::ListWorkflowsResponse>, Status> {
        let caller = caller_context(&request)?;
        let (after, limit) = page(request.get_ref().page.clone())?;
        let value = self
            .api()?
            .list_workflows(caller, after, limit)
            .await
            .map_err(api_status)?;
        bounded(proto::ListWorkflowsResponse {
            workflows: value
                .items
                .into_iter()
                .map(wire::definition)
                .collect::<Result<_, _>>()?,
            page: Some(proto::PageResponse {
                next_page_token: value.next_cursor.unwrap_or_default(),
            }),
        })
    }
    async fn validate_workflow_definition(
        &self,
        request: Request<proto::ValidateWorkflowDefinitionRequest>,
    ) -> Result<Response<proto::ValidateWorkflowDefinitionResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = self
            .api()?
            .validate_definition(caller, request.get_ref().yaml.clone())
            .await
            .map_err(api_status)?;
        bounded(proto::ValidateWorkflowDefinitionResponse {
            workflow: Some(wire::definition(value)?),
        })
    }
    async fn register_workflow_definition(
        &self,
        request: Request<proto::RegisterWorkflowDefinitionRequest>,
    ) -> Result<Response<proto::RegisterWorkflowDefinitionResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = request.get_ref();
        let value = self
            .api()?
            .register_definition(
                caller,
                value.yaml.clone(),
                value.expected_definition_hash.clone(),
                value.idempotency_key.clone(),
            )
            .await
            .map_err(api_status)?;
        bounded(proto::RegisterWorkflowDefinitionResponse {
            workflow: Some(wire::definition(value).map_err(mutation_output)?),
        })
    }
    async fn list_workflow_schedules(
        &self,
        request: Request<proto::ListWorkflowSchedulesRequest>,
    ) -> Result<Response<proto::ListWorkflowSchedulesResponse>, Status> {
        let caller = caller_context(&request)?;
        let (after, limit) = page(request.get_ref().page.clone())?;
        let value = self
            .api()?
            .list_schedules(caller, after, limit)
            .await
            .map_err(api_status)?;
        bounded(proto::ListWorkflowSchedulesResponse {
            schedules: value
                .items
                .into_iter()
                .map(wire::schedule)
                .collect::<Result<_, _>>()?,
            page: Some(proto::PageResponse {
                next_page_token: value.next_cursor.unwrap_or_default(),
            }),
        })
    }
    async fn get_workflow_schedule(
        &self,
        request: Request<proto::GetWorkflowScheduleRequest>,
    ) -> Result<Response<proto::GetWorkflowScheduleResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = self
            .api()?
            .get_schedule(caller, request.get_ref().schedule_id.clone())
            .await
            .map_err(api_status)?;
        bounded(proto::GetWorkflowScheduleResponse {
            schedule: Some(wire::schedule(value)?),
        })
    }
    async fn create_workflow_schedule(
        &self,
        request: Request<proto::CreateWorkflowScheduleRequest>,
    ) -> Result<Response<proto::CreateWorkflowScheduleResponse>, Status> {
        let caller = caller_context(&request)?;
        caller
            .require_scope(scopes::SCHEDULES_CREATE)
            .map_err(api_status)?;
        let value = request.get_ref();
        let value = self
            .api()?
            .create_schedule(
                caller,
                colossus_api::CreateWorkflowScheduleRequest {
                    schedule_id: value.schedule_id.clone(),
                    workflow_id: value.workflow_id.clone(),
                    expected_hash: value.expected_definition_hash.clone(),
                    inputs: wire::json(value.input.clone())?,
                    cadence_seconds: value.cadence_seconds,
                    starts_at: wire::instant(value.starts_at)?,
                    misfire_policy: wire::misfire(value.misfire_policy)?,
                    enabled: value.enabled,
                    idempotency_key: value.idempotency_key.clone(),
                },
            )
            .await
            .map_err(api_status)?;
        bounded(proto::CreateWorkflowScheduleResponse {
            schedule: Some(wire::schedule(value).map_err(mutation_output)?),
        })
    }
    async fn set_workflow_schedule_enabled(
        &self,
        request: Request<proto::SetWorkflowScheduleEnabledRequest>,
    ) -> Result<Response<proto::SetWorkflowScheduleEnabledResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = request.get_ref();
        let value = self
            .api()?
            .set_schedule_enabled(
                caller,
                colossus_api::SetWorkflowScheduleEnabledRequest {
                    schedule_id: value.schedule_id.clone(),
                    enabled: value.enabled,
                    etag: value.etag.clone(),
                },
            )
            .await
            .map_err(api_status)?;
        bounded(proto::SetWorkflowScheduleEnabledResponse {
            schedule: Some(wire::schedule(value).map_err(mutation_output)?),
        })
    }
    async fn get_workflow_work_state(
        &self,
        request: Request<proto::GetWorkflowWorkStateRequest>,
    ) -> Result<Response<proto::GetWorkflowWorkStateResponse>, Status> {
        let caller = caller_context(&request)?;
        bounded(proto::GetWorkflowWorkStateResponse {
            active: self
                .api()?
                .has_active_work(caller)
                .await
                .map_err(api_status)?,
        })
    }
    async fn start_workflow_run(
        &self,
        request: Request<proto::StartWorkflowRunRequest>,
    ) -> Result<Response<proto::StartWorkflowRunResponse>, Status> {
        let caller = caller_context(&request)?;
        caller
            .require_scope(scopes::WORKFLOW_RUNS_START)
            .map_err(api_status)?;
        let value = request.get_ref();
        let value = self
            .api()?
            .start_run(
                caller,
                colossus_api::StartWorkflowRunRequest {
                    workflow_id: value.workflow_id.clone(),
                    expected_hash: value.expected_definition_hash.clone(),
                    inputs: wire::json(value.input.clone())?,
                    idempotency_key: value.idempotency_key.clone(),
                },
            )
            .await
            .map_err(api_status)?;
        bounded(proto::StartWorkflowRunResponse {
            workflow_run: Some(wire::run(value).map_err(mutation_output)?),
        })
    }
    async fn get_workflow_run(
        &self,
        request: Request<proto::GetWorkflowRunRequest>,
    ) -> Result<Response<proto::GetWorkflowRunResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = self
            .api()?
            .get_run(caller, request.get_ref().workflow_run_id.clone())
            .await
            .map_err(api_status)?;
        bounded(proto::GetWorkflowRunResponse {
            workflow_run: Some(wire::run(value)?),
        })
    }
    type WatchWorkflowRunStream = Pin<
        Box<dyn Stream<Item = Result<proto::WatchWorkflowRunResponse, Status>> + Send + 'static>,
    >;
    async fn watch_workflow_run(
        &self,
        request: Request<proto::WatchWorkflowRunRequest>,
    ) -> Result<Response<Self::WatchWorkflowRunStream>, Status> {
        let caller = caller_context(&request)?.clone();
        caller
            .require_scope(scopes::WORKFLOW_RUNS_READ)
            .map_err(api_status)?;
        self.api()?;
        let api = self
            .api
            .clone()
            .ok_or_else(|| Status::unimplemented("workflow resources are unavailable"))?;
        let id = request.get_ref().workflow_run_id.clone();
        let cursor = request.get_ref().after_sequence;
        let initial = api.get_run(&caller, id.clone()).await.map_err(api_status)?;
        if cursor > initial.last_sequence {
            return Err(Status::invalid_argument(
                "workflow cursor is ahead of the canonical run",
            ));
        }
        let permit = self
            .watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("watch capacity reached"))?;
        let state = (
            api,
            caller,
            id,
            cursor,
            Some(initial),
            permit,
            tokio::time::Instant::now(),
        );
        // Coalesce canonical snapshots. Retaining the permit in the stream means
        // dropping a watch immediately returns capacity without a detached task.
        let stream = futures::stream::unfold(Some(state), |state| async move {
            let (api, caller, id, mut cursor, mut initial, permit, started) = state?;
            loop {
                if started.elapsed() >= Duration::from_secs(15 * 60) {
                    return None;
                }
                let value = match initial.take() {
                    Some(value) => Ok(value),
                    None => api.get_run(&caller, id.clone()).await.map_err(api_status),
                };
                let value = match value {
                    Ok(value) => value,
                    Err(error) => return Some((Err(error), None)),
                };
                let terminal = !matches!(
                    value.status,
                    colossus_api::WorkflowStatus::Queued
                        | colossus_api::WorkflowStatus::Running
                        | colossus_api::WorkflowStatus::Waiting
                );
                if value.last_sequence > cursor {
                    cursor = value.last_sequence;
                    let result = wire::run(value).map(|run| proto::WatchWorkflowRunResponse {
                        sequence: cursor,
                        created_at: run.updated_at,
                        workflow_run: Some(run),
                    });
                    let state = (!terminal && result.is_ok())
                        .then_some((api, caller, id, cursor, None, permit, started));
                    return Some((result, state));
                }
                if terminal {
                    return None;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        // Optional state lets terminal/error snapshots end the stream on the next poll.
        Ok(Response::new(
            Box::pin(stream) as Self::WatchWorkflowRunStream
        ))
    }
}
