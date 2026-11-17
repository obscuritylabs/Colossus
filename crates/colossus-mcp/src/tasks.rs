//! Same-run custody and policy-gated follow-up of one modern HTTP MCP task.

use super::*;
use crate::executor::{bounded_result, failed};
use crate::remote_tasks::{TaskHandle, TaskRequestError, TaskStart};
use colossus_contracts::{ActorType, EventClassification, NewEvent, PolicyObligations};
use colossus_policy::{EffectGateway, GatewayError, ReleasedEffectResult};
use colossus_ports::{EventJournal, RunControl};
use http::{HeaderName, HeaderValue};
use rmcp::model::TaskStatus;
use std::{collections::HashMap, future::Future, sync::Mutex, time::Duration};
use tokio::time::Instant;

mod recovery;
#[cfg(test)]
mod tests;
pub use recovery::recover_mcp_task_receipts;

const MIN_POLL: Duration = Duration::from_millis(250);
const DEFAULT_POLL: Duration = Duration::from_secs(1);
const MAX_POLLS: usize = 128;
const MAX_LOST_OBSERVATIONS: usize = 2;

struct TaskRun {
    control: RunControl,
    journal: Arc<dyn EventJournal>,
}

tokio::task_local! {
    static TASK_RUN: Option<TaskRun>;
    static TASK_EFFECT: TaskEffect;
}

/// Install task custody only around an ordinary, local, actively controlled run.
/// Nested scopes can disable it. No raw task identifier can survive the run.
pub async fn with_mcp_task_run<F: Future>(
    enabled: bool,
    control: RunControl,
    journal: Arc<dyn EventJournal>,
    future: F,
) -> F::Output {
    TASK_RUN
        .scope(enabled.then_some(TaskRun { control, journal }), future)
        .await
}

pub(super) fn eligible(request: &EffectRequest) -> bool {
    let context = &request.context;
    request.actor.actor_type == ActorType::Model
        && context.session_id.is_some()
        && context.run_id.is_some()
        && context.workflow_id.is_none()
        && context.goal_id.is_none()
        && context.plan_id.is_none()
        && context.draft_plan_id.is_none()
        && context.subagent_id.is_none()
        && TASK_RUN.try_with(Option::is_some).unwrap_or(false)
}

pub(super) fn cancelled_before_dispatch() -> bool {
    TASK_RUN
        .try_with(|run| run.as_ref().is_some_and(|run| run.control.is_cancelled()))
        .unwrap_or(false)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TaskStep {
    Start,
    Get { binding: String },
    Cancel { binding: String },
}

impl TaskStep {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Start => "mcp.call",
            Self::Get { .. } => "mcp.tasks.get",
            Self::Cancel { .. } => "mcp.tasks.cancel",
        }
    }
}

struct TaskEffect {
    request_id: String,
    step: TaskStep,
    slot: TaskSlot,
    deadline: Option<Instant>,
}

type TaskSlot = Arc<Mutex<Option<Arc<TrackedTask>>>>;

struct TrackedTask {
    handle: TaskHandle,
    binding: String,
    nonce: [u8; 32],
    credentials: [u8; 32],
    original: EffectRequest,
    obligations: PolicyObligations,
    request_hash: String,
    max_output: u64,
    call_started: Instant,
    deadline: Instant,
    poll_interval: Option<Duration>,
    receipt_version: Mutex<u64>,
    journal: Arc<dyn EventJournal>,
}

fn scoped_effect(request: &EffectRequest, step: &TaskStep) -> Result<TaskSlot, ExecutionError> {
    if !eligible(request) {
        return Err(failed("MCP Tasks requires an active local Execute run"));
    }
    TASK_EFFECT
        .try_with(|scope| {
            (scope.request_id == request.request_id && scope.step == *step)
                .then(|| Arc::clone(&scope.slot))
        })
        .ok()
        .flatten()
        .ok_or_else(|| failed("MCP Tasks effect lacks matching active custody"))
}

fn authority_fingerprint(headers: &HashMap<HeaderName, HeaderValue>, nonce: &[u8; 32]) -> [u8; 32] {
    let ordered: BTreeMap<_, _> = headers
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_bytes()))
        .collect();
    let mut hash = Sha256::new();
    hash.update(b"colossus-mcp-task-headers-v1");
    hash.update(nonce);
    for (name, value) in ordered {
        hash.update(name.len().to_be_bytes());
        hash.update(name.as_bytes());
        hash.update(value.len().to_be_bytes());
        hash.update(value);
    }
    hash.finalize().into()
}

impl TrackedTask {
    fn record(&self, state: Status) -> Result<(), ExecutionError> {
        let mut version = self.receipt_version.lock().map_err(|_| unknown())?;
        let event = self
            .journal
            .append(NewEvent {
                event_version: 1,
                stream_id: format!("{}{}", recovery::PREFIX, self.original.request_id),
                expected_stream_version: *version,
                classification: EventClassification::Effect,
                event_type: recovery::EVENT.into(),
                actor: self.original.actor.clone(),
                context: self.original.context.clone(),
                payload: serde_json::to_value(recovery::Receipt {
                    binding: self.binding.clone(),
                    call_request_id: self.original.request_id.clone(),
                    call_request_hash: self.request_hash.clone(),
                    state,
                })
                .map_err(|_| unknown())?,
            })
            .map_err(|_| unknown())?;
        *version = event.stream_version;
        Ok(())
    }
}

/// Require matching active custody even for an immediately completed opt-in call.
pub(super) fn verify_step(request: &EffectRequest, step: &TaskStep) -> Result<(), ExecutionError> {
    scoped_effect(request, step).map(|_| ())
}

pub(super) fn adopt(
    request: &EffectRequest,
    permit: &ExecutionPermit,
    headers: &HashMap<HeaderName, HeaderValue>,
    started: TaskStart,
    max_output: u64,
    call_started: Instant,
    deadline: Instant,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let slot = scoped_effect(request, &TaskStep::Start)?;
    let journal = TASK_RUN
        .try_with(|run| run.as_ref().map(|run| Arc::clone(&run.journal)))
        .ok()
        .flatten()
        .ok_or_else(unknown)?;
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| unknown())?;
    let deadline = started
        .ttl
        .and_then(|ttl| call_started.checked_add(ttl))
        .map_or(deadline, |end| end.min(deadline));
    let initial = match started.status {
        TaskStatus::Working => Status::Working,
        TaskStatus::InputRequired => Status::InputRequired,
        TaskStatus::Completed => Status::Completed,
        TaskStatus::Failed => Status::Failed,
        TaskStatus::Cancelled => Status::Cancelled,
        _ => Status::Unknown,
    };
    let task = Arc::new(TrackedTask {
        binding: hex::encode(started.handle.binding(&nonce)),
        credentials: authority_fingerprint(headers, &nonce),
        handle: started.handle,
        nonce,
        original: request.clone(),
        obligations: permit.obligations().clone(),
        request_hash: permit.request_hash().into(),
        max_output,
        call_started,
        deadline,
        poll_interval: started.poll_interval,
        receipt_version: Mutex::new(0),
        journal,
    });
    let mut slot = slot.lock().map_err(|_| unknown())?;
    if slot.is_some() {
        return Err(unknown());
    }
    task.record(Status::Working)?;
    *slot = Some(task);
    initial.into_effect(max_output)
}

pub(super) struct TaskAccess {
    task: Arc<TrackedTask>,
    deadline: Instant,
}

impl TaskAccess {
    pub(super) fn handle(&self) -> &TaskHandle {
        &self.task.handle
    }

    pub(super) fn output_bound(&self, bound: u64) -> u64 {
        bound.min(self.task.max_output)
    }

    pub(super) fn timeout_bound(&self, bound: u64) -> Result<u64, ExecutionError> {
        let remaining = self
            .deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        let remaining = u64::try_from(remaining).unwrap_or(u64::MAX).min(bound);
        if remaining == 0 {
            Err(unknown())
        } else {
            Ok(remaining)
        }
    }

    pub(super) fn ensure_live(&self) -> Result<(), ExecutionError> {
        if Instant::now() < self.deadline {
            Ok(())
        } else {
            Err(unknown())
        }
    }
}

pub(super) fn access(
    request: &EffectRequest,
    permit: &ExecutionPermit,
    step: &TaskStep,
    headers: &HashMap<HeaderName, HeaderValue>,
) -> Result<TaskAccess, ExecutionError> {
    let slot = scoped_effect(request, step)?;
    let task = slot
        .lock()
        .map_err(|_| unknown())?
        .clone()
        .ok_or_else(unknown)?;
    let (TaskStep::Get { binding } | TaskStep::Cancel { binding }) = step else {
        return Err(unknown());
    };
    let current = permit.obligations();
    // Never replace the original authority. Reissued permits can deny the
    // operation or shorten its budget; drift in resource authority fails closed.
    if *binding != task.binding
        || request.actor != task.original.actor
        || request.context != task.original.context
        || request.resource != task.original.resource
        || current.resource_authority != task.obligations.resource_authority
        || current.network_destinations != task.obligations.network_destinations
        || current.allowed_environment != task.obligations.allowed_environment
        || current.required_redactions != task.obligations.required_redactions
        || authority_fingerprint(headers, &task.nonce) != task.credentials
    {
        return Err(unknown());
    }
    let deadline = TASK_EFFECT
        .try_with(|scope| scope.deadline)
        .ok()
        .flatten()
        .ok_or_else(unknown)?
        .min(task.deadline);
    let access = TaskAccess { task, deadline };
    access.ensure_live()?;
    Ok(access)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Working,
    InputRequired,
    Completed,
    Failed,
    CancelRequested,
    Cancelled,
    ConnectionLost,
    Unavailable,
    Unknown,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    status: Status,
    poll_ms: Option<u64>,
    ttl_ms: Option<u64>,
}

impl Status {
    pub(super) fn into_effect(self, bound: u64) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.observation(None, None, bound)
    }

    pub(super) fn observation(
        self,
        poll: Option<Duration>,
        ttl: Option<Duration>,
        bound: u64,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        bounded_result(
            &Observation {
                status: self,
                poll_ms: poll
                    .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
                ttl_ms: ttl.map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
            },
            bound,
        )
    }

    pub(super) fn from_request_error(error: TaskRequestError) -> Self {
        match error {
            TaskRequestError::Transport => Self::ConnectionLost,
            TaskRequestError::Unavailable => Self::Unavailable,
            _ => Self::Unknown,
        }
    }
}

fn unknown() -> ExecutionError {
    ExecutionError::OutcomeUnknown(
        "MCP task outcome is unknown; the original tool call was not repeated".into(),
    )
}

fn gateway_unknown() -> GatewayError {
    GatewayError::OutcomeUnknown(unknown().to_string())
}

/// Errors from the opted-in call path preserve the effect gateway's uncertainty.
#[derive(Debug, Error)]
pub enum McpTaskCallError {
    /// The opted-in request could not be constructed.
    #[error(transparent)]
    Config(#[from] McpError),
    /// The original call or one of its observations could not be released.
    #[error(transparent)]
    Gateway(#[from] GatewayError),
}

impl McpExecutor {
    /// Invoke an opted-in server inside the active run, with fresh authorization for
    /// every continuation. Returns None only before dispatch when Tasks is disabled.
    pub async fn invoke_task_call(
        &self,
        gateway: &EffectGateway,
        effect: &dyn EffectExecutor,
        actor: &Actor,
        context: &ExecutionContext,
        operation: &McpOperation,
    ) -> Result<Option<McpCallOutput>, McpTaskCallError> {
        if !self.supports_task_calls(operation.server()) {
            return Ok(None);
        }
        let start = self.request_step(
            actor.clone(),
            context.clone(),
            operation.clone(),
            Some(TaskStep::Start),
        )?;
        if !eligible(&start) {
            return Ok(None);
        }
        let control = TASK_RUN
            .with(|run| run.as_ref().map(|run| run.control.clone()))
            .ok_or_else(gateway_unknown)?;
        if control.is_cancelled() {
            return Err(GatewayError::Execution(
                "MCP task call was cancelled before dispatch".into(),
            )
            .into());
        }
        let slot = Arc::new(Mutex::new(None));
        let result = execute_step(gateway, effect, start, TaskStep::Start, &slot, None).await;
        let task = slot.lock().map_err(|_| gateway_unknown())?.clone();
        match (result, task) {
            (Ok(result), None) => serde_json::from_slice(&result.bytes)
                .map(Some)
                .map_err(|_| gateway_unknown().into()),
            (Err(error), None) => Err(error.into()),
            (Err(_), Some(task)) => {
                task.record(Status::Unknown)
                    .map_err(|_| gateway_unknown())?;
                Err(gateway_unknown().into())
            }
            (Ok(result), Some(task)) => {
                let Ok(acknowledged) = serde_json::from_slice::<Observation>(&result.bytes) else {
                    task.record(Status::Unknown)
                        .map_err(|_| gateway_unknown())?;
                    return Err(gateway_unknown().into());
                };
                let (state, output) = self
                    .follow_task(
                        gateway,
                        effect,
                        operation,
                        &control,
                        (&task, acknowledged.status),
                        &slot,
                    )
                    .await;
                task.record(state).map_err(|_| gateway_unknown())?;
                output.map(Some).map_err(Into::into)
            }
        }
    }

    async fn follow_task(
        &self,
        gateway: &EffectGateway,
        effect: &dyn EffectExecutor,
        operation: &McpOperation,
        control: &RunControl,
        released: (&TrackedTask, Status),
        slot: &TaskSlot,
    ) -> (Status, Result<McpCallOutput, GatewayError>) {
        let (task, initial) = released;
        let mut interval = match initial {
            Status::InputRequired => return (initial, Err(input_required())),
            Status::Cancelled => return (initial, Err(cancelled())),
            Status::Failed => return (initial, Err(remote_failed())),
            Status::Completed => Duration::ZERO,
            Status::Working => task.poll_interval.unwrap_or(DEFAULT_POLL).max(MIN_POLL),
            _ => return (Status::Unknown, Err(gateway_unknown())),
        };
        let mut lost = 0;
        let mut cancel_requested = false;
        let mut deadline = task.deadline;
        for _ in 0..MAX_POLLS {
            wait_poll(deadline, interval, control, cancel_requested).await;
            if Instant::now() >= deadline {
                break;
            }
            if control.is_cancelled() && !cancel_requested {
                cancel_requested = true;
                let step = TaskStep::Cancel {
                    binding: task.binding.clone(),
                };
                let result = self
                    .observe(gateway, effect, operation, (task, deadline), step, slot)
                    .await;
                if result.is_ok_and(|result| {
                    serde_json::from_slice::<Observation>(&result.bytes)
                        .is_ok_and(|value| value.status == Status::CancelRequested)
                }) && task.record(Status::CancelRequested).is_err()
                {
                    return (Status::Unknown, Err(gateway_unknown()));
                }
            }
            let step = TaskStep::Get {
                binding: task.binding.clone(),
            };
            let Ok(result) = self
                .observe(gateway, effect, operation, (task, deadline), step, slot)
                .await
            else {
                break;
            };
            if let Ok(output) = serde_json::from_slice::<McpCallOutput>(&result.bytes) {
                return (Status::Completed, Ok(output));
            }
            let Ok(value) = serde_json::from_slice::<Observation>(&result.bytes) else {
                break;
            };
            match value.status {
                Status::Working => {
                    lost = 0;
                    if let Some(end) = value
                        .ttl_ms
                        .and_then(|ttl| task.call_started.checked_add(Duration::from_millis(ttl)))
                    {
                        deadline = deadline.min(end);
                    }
                    interval = value
                        .poll_ms
                        .map(Duration::from_millis)
                        .unwrap_or(interval)
                        .max(MIN_POLL);
                }
                Status::ConnectionLost if lost < MAX_LOST_OBSERVATIONS => lost += 1,
                Status::Cancelled => return (Status::Cancelled, Err(cancelled())),
                Status::Failed => return (Status::Failed, Err(remote_failed())),
                Status::InputRequired => return (Status::InputRequired, Err(input_required())),
                Status::Unavailable => return (Status::Unavailable, Err(gateway_unknown())),
                _ => break,
            }
        }
        let error = if cancel_requested {
            GatewayError::OutcomeUnknown(
                "MCP task cancellation was not confirmed by the server".into(),
            )
        } else {
            gateway_unknown()
        };
        (Status::Unknown, Err(error))
    }

    async fn observe(
        &self,
        gateway: &EffectGateway,
        effect: &dyn EffectExecutor,
        operation: &McpOperation,
        active: (&TrackedTask, Instant),
        step: TaskStep,
        slot: &TaskSlot,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        let (task, deadline) = active;
        let request = self
            .request_step(
                task.original.actor.clone(),
                task.original.context.clone(),
                operation.clone(),
                Some(step.clone()),
            )
            .map_err(|_| gateway_unknown())?;
        tokio::time::timeout_at(
            deadline,
            execute_step(gateway, effect, request, step, slot, Some(deadline)),
        )
        .await
        .map_err(|_| gateway_unknown())?
    }
}

async fn execute_step(
    gateway: &EffectGateway,
    effect: &dyn EffectExecutor,
    request: EffectRequest,
    step: TaskStep,
    slot: &TaskSlot,
    deadline: Option<Instant>,
) -> Result<ReleasedEffectResult, GatewayError> {
    let scope = TaskEffect {
        request_id: request.request_id.clone(),
        step,
        slot: Arc::clone(slot),
        deadline,
    };
    TASK_EFFECT
        .scope(scope, gateway.execute(request, effect))
        .await
}

async fn wait_poll(deadline: Instant, interval: Duration, control: &RunControl, cancel_sent: bool) {
    let next = Instant::now()
        .checked_add(interval)
        .unwrap_or(deadline)
        .min(deadline);
    while Instant::now() < next && (cancel_sent || !control.is_cancelled()) {
        tokio::time::sleep_until(next.min(Instant::now() + MIN_POLL)).await;
    }
}

fn input_required() -> GatewayError {
    GatewayError::OutcomeUnknown(
        "MCP task requires unsupported input; no input was sent and the task was not repeated"
            .into(),
    )
}

fn cancelled() -> GatewayError {
    GatewayError::Execution("Remote MCP task confirmed cancelled".into())
}

fn remote_failed() -> GatewayError {
    GatewayError::Execution("Remote MCP task failed".into())
}
