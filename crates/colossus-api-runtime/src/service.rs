use crate::{
    admission::{
        AdmissionLimitReached, ListAdmission, ReserveRun, RunAdmissionConfig, RunAdmissionState,
        WatchAdmission,
    },
    feed::RunFeeds,
    interactions::PublicInteractionRouter,
    writer::RunWriter,
};
use async_trait::async_trait;
use colossus_api::{
    AgentRunApi, ApiError, ApiErrorReason, ApiResult, ApplicationPrincipal, ArchiveThreadRequest,
    ArtifactApi, ArtifactPurpose, ArtifactState, CallerContext, CancelRunRequest, ContentPart,
    CreateRunRequest, CreateRunResponse, EventSourcedArtifactApi, EventSourcedRunRepository,
    GetRunRequest, Interaction, InteractionStatus, ListRunsRequest, ListRunsResponse, NewRun,
    OutcomeCertainty, PlanExecutionStrategy as PublicPlanExecutionStrategy, PlanRunAction,
    PlanStatus as PublicPlanStatus, RequestId, ResearchDepth as PublicResearchDepth,
    ResearchSourceKind as PublicResearchSourceKind, RespondInteractionRequest,
    RestoreThreadRequest, Run, RunBranchContextMode, RunCancellation, RunExecutionRequest,
    RunFailure, RunMode, RunRepository, RunResult, RunStatus, RunUpdateKind, RunUpdateStream,
    ThreadLifecycle, TokenUsage, ToolActivity, ToolActivityState, WatchRunRequest, scopes,
};
use colossus_contracts::{
    ActorType, AgentRunCancellation, AgentRunMode, AgentRunOutcome, AgentRunResult,
    ControlledAgentTerminal, GoalRunOutcome, GoalRunResult, PlanDraftTarget, PlanExecutionOutcome,
    PlanExecutionStrategy, PlanRecord, PlanStatus, ProviderEvent, ResearchDepth,
    ResearchSourceKind, RunEvent, RunEventEnvelope, RunPhase, ToolCall, ToolResult,
};
use colossus_ports::{ModelProviderError, RunControl, RunEventObserver, StoreError};
use colossus_runtime::{Runtime, RuntimeError};
use futures::FutureExt as _;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

const WATCH_PAGE_SIZE: usize = 16;
const WATCH_CHANNEL_SIZE: usize = 8;
const MAX_PENDING_RECOVERIES: usize = 256;
const MAX_RENDERED_INPUT_BYTES: usize = 1_048_576;
const MAX_TOOL_ACTIVITY_PREVIEW_BYTES: usize = 65_536;
const TOOL_ACTIVITY_PREVIEW_TRUNCATION: &str = "\n… preview truncated";

#[derive(Clone)]
struct ActiveRun {
    control: RunControl,
    writer: Arc<RunWriter>,
    generation: u64,
}

struct ExecutionRegistry {
    active: BTreeMap<String, ActiveRun>,
    admission: RunAdmissionState,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum ExecutionTestFault {
    Panic,
    FailTerminalAppend,
}

struct PendingExecution {
    caller: CallerContext,
    run: Run,
    request: CreateRunRequest,
    create_session: bool,
    control: RunControl,
    writer: Arc<RunWriter>,
    generation: u64,
}

struct ExecutionStart {
    caller: CallerContext,
    run: Run,
    request: CreateRunRequest,
    create_session: bool,
}

enum CreateDecision {
    Fresh {
        run: Run,
        pending: Box<PendingExecution>,
    },
    Replay(Run),
}

enum StartDecision {
    Started(Box<PendingExecution>),
    AlreadyStarted,
    Limited,
}

#[derive(Clone, Copy)]
enum ExecutionAdmission {
    Fresh,
    Existing,
}

#[derive(Clone)]
struct PlanSelection {
    plan: PlanRecord,
}

struct BranchSource {
    session_id: String,
    source_message_count: u64,
    context_mode: RunBranchContextMode,
}

/// Durable public agent-run facade backed by the real Colossus runtime.
#[derive(Clone)]
pub struct RuntimeAgentRunApi {
    runtime: Arc<Runtime>,
    repository: Arc<dyn RunRepository>,
    artifacts: Arc<dyn ArtifactApi>,
    interactions: Arc<PublicInteractionRouter>,
    feeds: Arc<RunFeeds>,
    execution: Arc<Mutex<ExecutionRegistry>>,
    watches: Arc<WatchAdmission>,
    lists: Arc<ListAdmission>,
    active_changed: Arc<tokio::sync::Notify>,
    recovery: Arc<Mutex<()>>,
    pending_recoveries: Arc<Mutex<BTreeMap<String, CallerContext>>>,
    default_role: String,
    instructions: String,
    #[cfg(test)]
    next_execution_fault: Arc<Mutex<Option<ExecutionTestFault>>>,
}

impl RuntimeAgentRunApi {
    /// Bind an already composed runtime and its public interaction router.
    pub fn new(
        runtime: Arc<Runtime>,
        interactions: Arc<PublicInteractionRouter>,
        default_role: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Self {
        let repository: Arc<dyn RunRepository> =
            Arc::new(EventSourcedRunRepository::new(runtime.journal()));
        Self::with_repository_and_admission(
            runtime,
            repository,
            interactions,
            default_role,
            instructions,
            RunAdmissionConfig::default(),
        )
    }

    /// Bind a runtime with explicit validated public admission controls.
    pub fn with_admission(
        runtime: Arc<Runtime>,
        interactions: Arc<PublicInteractionRouter>,
        default_role: impl Into<String>,
        instructions: impl Into<String>,
        admission: RunAdmissionConfig,
    ) -> Self {
        let repository: Arc<dyn RunRepository> =
            Arc::new(EventSourcedRunRepository::new(runtime.journal()));
        Self::with_repository_and_admission(
            runtime,
            repository,
            interactions,
            default_role,
            instructions,
            admission,
        )
    }

    /// Construct with an explicit repository, primarily for conformance adapters.
    pub fn with_repository(
        runtime: Arc<Runtime>,
        repository: Arc<dyn RunRepository>,
        interactions: Arc<PublicInteractionRouter>,
        default_role: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Self {
        Self::with_repository_and_admission(
            runtime,
            repository,
            interactions,
            default_role,
            instructions,
            RunAdmissionConfig::default(),
        )
    }

    /// Construct with an explicit repository and admission configuration.
    pub fn with_repository_and_admission(
        runtime: Arc<Runtime>,
        repository: Arc<dyn RunRepository>,
        interactions: Arc<PublicInteractionRouter>,
        default_role: impl Into<String>,
        instructions: impl Into<String>,
        admission: RunAdmissionConfig,
    ) -> Self {
        let watches = WatchAdmission::new(&admission);
        let lists = ListAdmission::new(&admission);
        Self {
            artifacts: Arc::new(EventSourcedArtifactApi::new(runtime.journal())),
            runtime,
            repository,
            interactions,
            feeds: Arc::new(RunFeeds::default()),
            execution: Arc::new(Mutex::new(ExecutionRegistry {
                active: BTreeMap::new(),
                admission: RunAdmissionState::new(admission, Instant::now()),
            })),
            watches,
            lists,
            active_changed: Arc::new(tokio::sync::Notify::new()),
            recovery: Arc::new(Mutex::new(())),
            pending_recoveries: Arc::new(Mutex::new(BTreeMap::new())),
            default_role: default_role.into(),
            instructions: instructions.into(),
            #[cfg(test)]
            next_execution_fault: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_next_execution_fault(&self, fault: ExecutionTestFault) {
        *lock(&self.next_execution_fault) = Some(fault);
    }

    #[cfg(test)]
    pub(crate) fn active_execution_count(&self) -> usize {
        lock(&self.execution).active.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_recovery_count(&self) -> usize {
        lock(&self.pending_recoveries).len()
    }

    #[cfg(test)]
    pub(crate) fn attach_active_writer_for_test(
        &self,
        run_id: impl Into<String>,
        writer: Arc<RunWriter>,
    ) {
        lock(&self.execution).active.insert(
            run_id.into(),
            ActiveRun {
                control: RunControl::default(),
                writer,
                generation: u64::MAX,
            },
        );
    }

    fn session_id(
        &self,
        caller: &CallerContext,
        requested: Option<&str>,
    ) -> ApiResult<(String, bool)> {
        let Some(session_id) = requested else {
            return Ok((Uuid::now_v7().to_string(), true));
        };
        let events = self
            .runtime
            .journal()
            .read_stream_from(&format!("session:{session_id}"), 0, 1)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let owned = events.first().is_some_and(|event| {
            event.event_type == "session.created.v1"
                && event.actor.actor_type == ActorType::Application
                && event.actor.id == caller.principal().application_id()
        });
        if !owned {
            return Err(ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested session was not found",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        Ok((session_id.into(), false))
    }

    fn branch_source_session(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
    ) -> ApiResult<Option<BranchSource>> {
        let Some(branch) = request.branch.as_ref() else {
            return Ok(None);
        };
        let source = self
            .repository
            .get_run(caller, &branch.source_run_id)?
            .ok_or_else(|| {
                ApiError::not_found(
                    ApiErrorReason::RunNotFound,
                    "the requested branch source was not found",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        let session = self
            .runtime
            .get_session(&source.session_id)
            .map_err(|error| match error {
                RuntimeError::Store(store) => ApiError::from_store(&store, caller.request_id()),
                _ => ApiError::failed_precondition(
                    ApiErrorReason::InvalidRunTransition,
                    "the branch source context is unavailable",
                )
                .with_correlation_id(caller.request_id().clone()),
            })?
            .ok_or_else(|| {
                ApiError::not_found(
                    ApiErrorReason::RunNotFound,
                    "the requested branch source was not found",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        let (source_message_count, context_mode) = match branch.context_mode {
            RunBranchContextMode::SourceRunConversation => {
                let source_messages = self.runtime.session_messages(&source.session_id).map_err(
                    |error| match error {
                        RuntimeError::Store(store) => {
                            ApiError::from_store(&store, caller.request_id())
                        }
                        _ => ApiError::failed_precondition(
                            ApiErrorReason::InvalidRunTransition,
                            "the branch source context is unavailable",
                        )
                        .with_correlation_id(caller.request_id().clone()),
                    },
                )?;
                let boundary = source_messages
                    .iter()
                    .filter(|message| message.run_id == source.id)
                    .map(|message| message.sequence)
                    .max()
                    .ok_or_else(|| {
                        ApiError::failed_precondition(
                            ApiErrorReason::InvalidRunTransition,
                            "the branch source run has no canonical conversation boundary",
                        )
                        .with_correlation_id(caller.request_id().clone())
                    })?;
                (boundary, RunBranchContextMode::Conversation)
            }
            mode => (branch.source_message_count, mode),
        };
        if source_message_count > session.message_count || source_message_count > 512 {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "branch.source_message_count",
                "branch boundary exceeds canonical source history",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        Ok(Some(BranchSource {
            session_id: source.session_id,
            source_message_count,
            context_mode,
        }))
    }

    async fn rendered_input(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
    ) -> ApiResult<String> {
        let mut rendered = String::new();
        for part in &request.input {
            let segment = match part {
                ContentPart::Text { text } => text.clone(),
                ContentPart::Artifact { artifact_id } => {
                    let download = self.artifacts.download(caller, artifact_id, 0).await?;
                    if download.artifact.state != ArtifactState::Available
                        || download.artifact.purpose != ArtifactPurpose::RunInput
                        || !supported_text_attachment(&download.artifact.media_type)
                    {
                        return Err(ApiError::failed_precondition(
                            ApiErrorReason::ArtifactUnavailable,
                            "the artifact is not an available text run input",
                        )
                        .with_correlation_id(caller.request_id().clone()));
                    }
                    let text = String::from_utf8(download.bytes).map_err(|_| {
                        ApiError::invalid(
                            ApiErrorReason::InvalidArgument,
                            "input.artifact",
                            "text run-input artifacts must contain UTF-8",
                        )
                        .with_correlation_id(caller.request_id().clone())
                    })?;
                    format!(
                        "Attached file {} ({}):\n{}",
                        download.artifact.file_name, download.artifact.media_type, text
                    )
                }
            };
            if !rendered.is_empty() {
                rendered.push_str("\n\n");
            }
            if rendered.len().saturating_add(segment.len()) > MAX_RENDERED_INPUT_BYTES {
                return Err(ApiError::bounded_resource_exhausted(
                    ApiErrorReason::CapacityExceeded,
                    "combined text and attachment input exceeds the run-input bound",
                )
                .with_correlation_id(caller.request_id().clone()));
            }
            rendered.push_str(&segment);
        }
        Ok(rendered)
    }

    fn reserve_execution_locked(
        &self,
        registry: &mut ExecutionRegistry,
        start: ExecutionStart,
        now: Instant,
        admission: ExecutionAdmission,
    ) -> Result<StartDecision, AdmissionLimitReached> {
        let ExecutionStart {
            caller,
            run,
            request,
            create_session,
        } = start;
        if registry.active.contains_key(&run.id) {
            return Ok(StartDecision::AlreadyStarted);
        }
        let application_id = caller.principal().application_id().to_owned();
        let reserved = match admission {
            ExecutionAdmission::Fresh => {
                registry
                    .admission
                    .reserve_checked(&application_id, &run.id, now)
            }
            ExecutionAdmission::Existing => {
                registry
                    .admission
                    .reserve_existing(&application_id, &run.id, now)
            }
        }?;
        let reservation = match reserved {
            ReserveRun::Reserved(reservation) => reservation,
            ReserveRun::AlreadyReserved => return Ok(StartDecision::AlreadyStarted),
        };
        let writer = Arc::new(RunWriter::new(
            Arc::clone(&self.repository),
            Arc::clone(&self.feeds),
            caller.clone(),
            &run,
        ));
        let control = RunControl::default();
        match registry.active.entry(run.id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(ActiveRun {
                    control: control.clone(),
                    writer: Arc::clone(&writer),
                    generation: reservation.generation(),
                });
            }
            Entry::Occupied(_) => {
                let _ =
                    registry
                        .admission
                        .release(&run.id, reservation.generation(), Instant::now());
                return Ok(StartDecision::AlreadyStarted);
            }
        }
        Ok(StartDecision::Started(Box::new(PendingExecution {
            caller,
            run,
            request,
            create_session,
            control,
            writer,
            generation: reservation.generation(),
        })))
    }

    fn ensure_execution_started(
        &self,
        caller: CallerContext,
        run: Run,
        request: CreateRunRequest,
        create_session: bool,
    ) -> StartDecision {
        let mut registry = lock(&self.execution);
        match self.reserve_execution_locked(
            &mut registry,
            ExecutionStart {
                caller,
                run,
                request,
                create_session,
            },
            Instant::now(),
            ExecutionAdmission::Existing,
        ) {
            Ok(decision) => decision,
            Err(AdmissionLimitReached) => StartDecision::Limited,
        }
    }

    fn create_admitted(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
        new_run: &NewRun,
        create_session: bool,
    ) -> ApiResult<CreateDecision> {
        if let Some(replay) = self.repository.resolve_create_run(caller, request)? {
            return Ok(CreateDecision::Replay(replay));
        }

        let now = Instant::now();
        let application_id = caller.principal().application_id();
        let mut registry = lock(&self.execution);
        if let Some(replay) = self.repository.resolve_create_run(caller, request)? {
            return Ok(CreateDecision::Replay(replay));
        }
        registry
            .admission
            .check_new(application_id, now)
            .map_err(|AdmissionLimitReached| capacity_error(caller))?;

        let created = self.repository.create_run(caller, request, new_run)?;
        if created.replayed {
            return Ok(CreateDecision::Replay(created.value));
        }
        let run = created.value;
        let pending = match self
            .reserve_execution_locked(
                &mut registry,
                ExecutionStart {
                    caller: caller.clone(),
                    run: run.clone(),
                    request: request.clone(),
                    create_session,
                },
                now,
                ExecutionAdmission::Fresh,
            )
            .map_err(|AdmissionLimitReached| capacity_error(caller))?
        {
            StartDecision::Started(pending) => pending,
            StartDecision::AlreadyStarted | StartDecision::Limited => {
                return Err(recovery_invariant(caller));
            }
        };
        Ok(CreateDecision::Fresh { run, pending })
    }

    fn plan_selection(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
    ) -> ApiResult<Option<PlanSelection>> {
        let Some(action) = request.plan_action.as_ref() else {
            return Ok(None);
        };
        caller.require_scope(scopes::RUNS_READ)?;
        let source = self
            .repository
            .get_run(caller, action.source_run_id())?
            .ok_or_else(|| {
                ApiError::not_found(
                    ApiErrorReason::RunNotFound,
                    "the Plan source run was not found",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        let (plan_id, released_revision, released_status) =
            match (source.result.as_ref(), source.cancellation.as_ref()) {
                (Some(result), _) => (
                    result.plan_id.as_deref(),
                    result.plan_revision,
                    result.plan_status,
                ),
                (_, Some(cancellation)) => (
                    cancellation.plan_id.as_deref(),
                    cancellation.plan_revision,
                    cancellation.plan_status,
                ),
                _ => (None, None, None),
            };
        let (Some(plan_id), Some(released_revision), Some(released_status)) =
            (plan_id, released_revision, released_status)
        else {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::InvalidRunTransition,
                "the source run does not expose an actionable Plan revision",
            )
            .with_correlation_id(caller.request_id().clone()));
        };
        if request.session_id.as_deref() != Some(source.session_id.as_str()) {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "session_id",
                "Plan continuation must remain in the source run session",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        if action.expected_revision() != released_revision {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "plan_action.expected_revision",
                "the requested revision does not match the source run",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        if released_status != PublicPlanStatus::Draft {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::InvalidRunTransition,
                "only a released draft Plan can be revised or executed",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let plan = self
            .runtime
            .get_plan(plan_id)
            .map_err(|_| {
                ApiError::failed_precondition(
                    ApiErrorReason::InvalidRunTransition,
                    "the selected Plan is unavailable",
                )
                .with_correlation_id(caller.request_id().clone())
            })?
            .ok_or_else(|| {
                ApiError::failed_precondition(
                    ApiErrorReason::InvalidRunTransition,
                    "the selected Plan is unavailable",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        if plan.session_id != source.session_id || plan.id != plan_id {
            return Err(recovery_invariant(caller));
        }
        if plan.revision != released_revision {
            return Err(ApiError::conflict(
                ApiErrorReason::ConcurrentModification,
                "the Plan changed; review its latest revision before continuing",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        if plan.status != PlanStatus::Draft {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::InvalidRunTransition,
                "the Plan is no longer a draft",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        Ok(Some(PlanSelection { plan }))
    }

    fn spawn_execution(&self, pending: PendingExecution) {
        self.active_changed.notify_waiters();
        let PendingExecution {
            caller,
            run,
            request,
            create_session,
            control,
            writer,
            generation,
        } = pending;
        let service = self.clone();
        #[cfg(test)]
        let execution_fault = lock(&service.next_execution_fault).take();
        #[cfg(not(test))]
        let panic_after_start = false;
        #[cfg(test)]
        let panic_after_start = matches!(execution_fault, Some(ExecutionTestFault::Panic));
        #[cfg(not(test))]
        let fail_terminal_append = false;
        #[cfg(test)]
        let fail_terminal_append = matches!(
            execution_fault,
            Some(ExecutionTestFault::FailTerminalAppend)
        );
        tokio::spawn(async move {
            let run_id = run.id.clone();
            let task = service.execute(
                caller,
                run,
                request,
                create_session,
                control,
                Arc::clone(&writer),
                panic_after_start,
                fail_terminal_append,
            );
            let mut terminal_persisted = match AssertUnwindSafe(task).catch_unwind().await {
                Ok(persisted) => persisted,
                Err(_) => writer.append(unknown_outcome_failure()).is_ok(),
            };
            if !terminal_persisted {
                terminal_persisted = match writer.synchronize_durable_state() {
                    Ok(current) if current.status.is_terminal() => true,
                    Ok(_) => writer.append(unknown_outcome_failure()).is_ok(),
                    Err(_) => false,
                };
            }
            service.interactions.cancel_run(&run_id);
            let mut registry = lock(&service.execution);
            let matching = registry
                .active
                .get(&run_id)
                .is_some_and(|active| active.generation == generation);
            if matching {
                registry.active.remove(&run_id);
                let _ = registry
                    .admission
                    .release(&run_id, generation, Instant::now());
            }
            drop(registry);
            if matching {
                service.active_changed.notify_waiters();
            }
            if !terminal_persisted {
                // Wake existing watchers so transport clients reconcile or reconnect instead
                // of waiting forever on a non-terminal cursor after storage failed.
                service.feeds.close(&run_id);
            }
            service.drain_pending_recoveries();
        });
    }

    fn enqueue_pending_recovery(
        &self,
        caller: CallerContext,
        run_id: &str,
    ) -> Result<(), AdmissionLimitReached> {
        let mut pending = lock(&self.pending_recoveries);
        if pending.contains_key(run_id) {
            return Ok(());
        }
        if pending.len() >= MAX_PENDING_RECOVERIES {
            return Err(AdmissionLimitReached);
        }
        pending.insert(run_id.into(), caller);
        Ok(())
    }

    fn remove_pending_recovery(&self, run_id: &str) {
        lock(&self.pending_recoveries).remove(run_id);
    }

    fn drain_pending_recoveries(&self) {
        let _recovery = lock(&self.recovery);
        let pending = lock(&self.pending_recoveries)
            .iter()
            .map(|(run_id, caller)| (run_id.clone(), caller.clone()))
            .collect::<Vec<_>>();
        for (run_id, caller) in pending {
            let current = match self.repository.recoverable_run(&caller, &run_id) {
                Ok(Some(current)) => current,
                Ok(None) => {
                    self.remove_pending_recovery(&run_id);
                    continue;
                }
                Err(_) => continue,
            };
            let (run, execution) = current;
            if run.status.is_terminal() {
                self.remove_pending_recovery(&run_id);
                continue;
            }
            if run.status == RunStatus::Cancelling && run.started_at.is_none() {
                let writer = RunWriter::new(
                    Arc::clone(&self.repository),
                    Arc::clone(&self.feeds),
                    caller.clone(),
                    &run,
                );
                if writer.append(cancelled_before_start()).is_ok() {
                    self.remove_pending_recovery(&run_id);
                }
                continue;
            }
            if run.status != RunStatus::Queued {
                self.remove_pending_recovery(&run_id);
                continue;
            }
            let create_session = execution.request.session_id.is_none();
            match self.ensure_execution_started(caller, run, execution.request, create_session) {
                StartDecision::Started(pending) => {
                    self.remove_pending_recovery(&run_id);
                    self.spawn_execution(*pending);
                }
                StartDecision::AlreadyStarted => {
                    self.remove_pending_recovery(&run_id);
                }
                StartDecision::Limited => {}
            }
        }
    }

    fn recover_orphan(&self, caller: &CallerContext, run: Run) -> ApiResult<Run> {
        if run.status.is_terminal() || lock(&self.execution).active.contains_key(&run.id) {
            return Ok(run);
        }
        let _recovery = lock(&self.recovery);
        if lock(&self.execution).active.contains_key(&run.id) {
            return self
                .repository
                .recoverable_run(caller, &run.id)?
                .map(|(current, _)| current)
                .ok_or_else(|| missing_run(caller));
        }
        let Some((current, execution)) = self.repository.recoverable_run(caller, &run.id)? else {
            return Err(missing_run(caller));
        };
        if current.status.is_terminal() {
            return Ok(current);
        }
        let recovered = recovered_caller(&execution, &current.id, caller)?;
        match current.status {
            RunStatus::Queued => {
                let create_session = execution.request.session_id.is_none();
                let pending_caller = recovered.clone();
                match self.ensure_execution_started(
                    recovered,
                    current.clone(),
                    execution.request,
                    create_session,
                ) {
                    StartDecision::Started(pending) => self.spawn_execution(*pending),
                    StartDecision::AlreadyStarted => {}
                    StartDecision::Limited => {
                        self.enqueue_pending_recovery(pending_caller, &current.id)
                            .map_err(|AdmissionLimitReached| capacity_error(caller))?;
                    }
                }
                Ok(current)
            }
            RunStatus::Waiting => {
                let writer = RunWriter::new(
                    Arc::clone(&self.repository),
                    Arc::clone(&self.feeds),
                    recovered.clone(),
                    &current,
                );
                if let Some(mut interaction) = current.pending_interaction.clone() {
                    interaction.status = InteractionStatus::Cancelled;
                    writer.append(RunUpdateKind::Interaction { interaction })?;
                }
                writer.append(interrupted_failure())?;
                self.current_recovered(&recovered, &current.id, caller)
            }
            RunStatus::Cancelling if current.started_at.is_none() => {
                let writer = RunWriter::new(
                    Arc::clone(&self.repository),
                    Arc::clone(&self.feeds),
                    recovered.clone(),
                    &current,
                );
                writer.append(cancelled_before_start())?;
                self.current_recovered(&recovered, &current.id, caller)
            }
            RunStatus::Running | RunStatus::Cancelling => {
                let writer = RunWriter::new(
                    Arc::clone(&self.repository),
                    Arc::clone(&self.feeds),
                    recovered.clone(),
                    &current,
                );
                writer.append(unknown_outcome_failure())?;
                self.current_recovered(&recovered, &current.id, caller)
            }
            RunStatus::Completed
            | RunStatus::Failed
            | RunStatus::Cancelled
            | RunStatus::Interrupted
            | RunStatus::OutcomeUnknown => Ok(current),
        }
    }

    fn current_recovered(
        &self,
        recovered: &CallerContext,
        run_id: &str,
        external: &CallerContext,
    ) -> ApiResult<Run> {
        self.repository
            .recoverable_run(recovered, run_id)?
            .map(|(run, _)| run)
            .ok_or_else(|| missing_run(external))
    }

    fn cancel_orphan(&self, caller: &CallerContext, request: &CancelRunRequest) -> ApiResult<Run> {
        let _recovery = lock(&self.recovery);
        let active_run = lock(&self.execution).active.get(&request.run_id).cloned();
        if let Some(active_run) = active_run {
            let run = active_run
                .writer
                .request_cancellation(caller, &request.idempotency_key)?;
            active_run.control.cancel();
            self.interactions.cancel_run(&request.run_id);
            return Ok(run);
        }
        let Some((before, execution)) = self.repository.recoverable_run(caller, &request.run_id)?
        else {
            return Err(missing_run(caller));
        };
        let result = self.repository.request_cancellation(
            caller,
            &request.run_id,
            &request.idempotency_key,
        )?;
        self.feeds.publish(
            &request.run_id,
            result.value.last_sequence,
            result.value.status.is_terminal(),
        );
        if result.value.status.is_terminal() {
            return Ok(result.value);
        }
        let recovered = recovered_caller(&execution, &request.run_id, caller)?;
        let writer = RunWriter::new(
            Arc::clone(&self.repository),
            Arc::clone(&self.feeds),
            recovered.clone(),
            &result.value,
        );
        match before.status {
            RunStatus::Queued => {
                writer.append(cancelled_before_start())?;
            }
            RunStatus::Waiting => {
                writer.append(interrupted_failure())?;
            }
            RunStatus::Running | RunStatus::Cancelling => {
                writer.append(unknown_outcome_failure())?;
            }
            RunStatus::Completed
            | RunStatus::Failed
            | RunStatus::Cancelled
            | RunStatus::Interrupted
            | RunStatus::OutcomeUnknown => {}
        }
        self.current_recovered(&recovered, &request.run_id, caller)
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        caller: CallerContext,
        run: Run,
        request: CreateRunRequest,
        create_session: bool,
        control: RunControl,
        writer: Arc<RunWriter>,
        panic_after_start: bool,
        fail_terminal_append: bool,
    ) -> bool {
        let was_cancelled_before_start = control.is_cancelled()
            || match writer.current_execution_run() {
                Ok(Some(current)) => {
                    current.status == RunStatus::Cancelling && current.started_at.is_none()
                }
                Ok(None) | Err(_) => return false,
            };
        if was_cancelled_before_start {
            return writer.append(cancelled_before_start()).is_ok();
        }
        let create_session = if request.branch.is_some() {
            let source = match self.branch_source_session(&caller, &request) {
                Ok(Some(value)) => value,
                Ok(None) => return false,
                Err(error) => {
                    let _ = writer.append(RunUpdateKind::Failure {
                        status: RunStatus::Failed,
                        failure: RunFailure {
                            code: "branch.context_unavailable".into(),
                            message: error.message,
                            outcome: OutcomeCertainty::Known,
                            recoverable: false,
                            http_status: None,
                            retry_after_ms: None,
                        },
                    });
                    return true;
                }
            };
            if self
                .runtime
                .ensure_session_branch(
                    &source.session_id,
                    source.source_message_count,
                    source.context_mode,
                    &run.session_id,
                    &run.id,
                    caller.actor(),
                )
                .is_err()
            {
                let _ = writer.append(RunUpdateKind::Failure {
                    status: RunStatus::Failed,
                    failure: RunFailure {
                        code: "branch.context_unavailable".into(),
                        message: "the Aside context could not be prepared".into(),
                        outcome: OutcomeCertainty::Known,
                        recoverable: false,
                        http_status: None,
                        retry_after_ms: None,
                    },
                });
                return true;
            }
            false
        } else {
            create_session
        };
        if writer
            .append(RunUpdateKind::State {
                status: RunStatus::Running,
            })
            .is_err()
        {
            return match writer.current_execution_run() {
                Ok(Some(current))
                    if current.status == RunStatus::Cancelling && current.started_at.is_none() =>
                {
                    writer.append(cancelled_before_start()).is_ok()
                }
                _ => false,
            };
        }
        if panic_after_start {
            panic!("injected public execution panic after durable start");
        }
        let prompt = match self.rendered_input(&caller, &request).await {
            Ok(prompt) => prompt,
            Err(error) => {
                let _ = writer.append(RunUpdateKind::Failure {
                    status: RunStatus::Failed,
                    failure: RunFailure {
                        code: "artifact.input_unavailable".into(),
                        message: error.message,
                        outcome: OutcomeCertainty::Known,
                        recoverable: false,
                        http_status: None,
                        retry_after_ms: None,
                    },
                });
                return true;
            }
        };
        let max_turns = if request.max_turns == 0 {
            None
        } else {
            u16::try_from(request.max_turns).ok()
        };
        let allowed_tools = caller
            .principal()
            .allowed_tools()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if request.mode == RunMode::Research {
            if create_session
                && self
                    .runtime
                    .create_application_session(&run.session_id, Some(&run.title), caller.actor())
                    .is_err()
            {
                let _ = writer.append(invariant_failure());
                return true;
            }
            if self
                .runtime
                .append_application_message(
                    &run.session_id,
                    &run.id,
                    colossus_contracts::ModelMessage {
                        role: colossus_contracts::ModelMessageRole::User,
                        content: prompt.clone(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    caller.actor(),
                )
                .is_err()
            {
                let _ = writer.append(invariant_failure());
                return true;
            }
            let depth = match request
                .research_depth
                .unwrap_or(PublicResearchDepth::Standard)
            {
                PublicResearchDepth::Quick => ResearchDepth::Quick,
                PublicResearchDepth::Standard => ResearchDepth::Standard,
                PublicResearchDepth::Deep => ResearchDepth::Deep,
            };
            let source_kinds = request
                .research_sources
                .iter()
                .map(|kind| match kind {
                    PublicResearchSourceKind::Repo => ResearchSourceKind::Repo,
                    PublicResearchSourceKind::Web => ResearchSourceKind::Web,
                    PublicResearchSourceKind::Mcp => ResearchSourceKind::Mcp,
                })
                .collect();
            let execution = self.runtime.run_research_as(
                &run.session_id,
                &prompt,
                depth,
                source_kinds,
                caller.actor(),
            );
            let kind = match self
                .interactions
                .scope(Arc::clone(&writer), execution)
                .await
            {
                Ok(_research) if control.is_cancelled() => RunUpdateKind::Cancellation {
                    cancellation: RunCancellation {
                        turn: 0,
                        message:
                            "Research was cancelled after the active evidence operation completed."
                                .into(),
                        plan_id: None,
                        plan_revision: None,
                        plan_status: None,
                        goal_id: None,
                    },
                },
                Ok(research) => {
                    for progress in &research.progress {
                        let phase = format!("{:?}", progress.phase).to_ascii_lowercase();
                        let status = format!("{:?}", progress.status).to_ascii_lowercase();
                        let _ = writer.append(RunUpdateKind::Notice {
                            notice: colossus_api::RunNotice {
                                reason: format!("research.{phase}.{status}"),
                                message: progress.message.clone(),
                            },
                        });
                    }
                    research_result(research)
                }
                Err(error) => runtime_failure(&error),
            };
            return writer.append(kind).is_ok();
        }
        let selection = match self.plan_selection(&caller, &request) {
            Ok(selection) => selection,
            Err(error) => {
                let _ = writer.append(plan_selection_failure(error));
                return true;
            }
        };
        let accepts_related_run_events = matches!(
            request.plan_action.as_ref(),
            Some(PlanRunAction::Execute {
                strategy: PublicPlanExecutionStrategy::Goal { .. },
                ..
            })
        );
        let mut observer = PublicRunObserver {
            writer: Arc::clone(&writer),
            accepts_related_run_events,
        };
        let kind = match request.plan_action.as_ref() {
            Some(PlanRunAction::Execute { strategy, .. }) => {
                let Some(selection) = selection else {
                    let _ = writer.append(invariant_failure());
                    return true;
                };
                let strategy = match strategy {
                    PublicPlanExecutionStrategy::Direct => PlanExecutionStrategy::Direct,
                    PublicPlanExecutionStrategy::Goal { max_iterations } => {
                        PlanExecutionStrategy::Goal {
                            max_iterations: *max_iterations,
                        }
                    }
                };
                let execution = self
                    .runtime
                    .approve_and_execute_public_plan_stream_controlled(
                        &run.role,
                        &run.session_id,
                        &selection.plan.id,
                        selection.plan.revision,
                        strategy,
                        max_turns,
                        &run.id,
                        request.end_user_id.as_deref(),
                        caller.remote_trace_context(),
                        &mut observer,
                        &control,
                    );
                match self
                    .interactions
                    .scope(Arc::clone(&writer), execution)
                    .await
                {
                    Ok(outcome) => public_plan_execution(outcome),
                    Err(error) => runtime_failure(&error),
                }
            }
            action => {
                let mode = match (action, selection) {
                    (Some(PlanRunAction::Revise { .. }), Some(PlanSelection { plan })) => {
                        AgentRunMode::Plan(PlanDraftTarget::Update {
                            plan_id: plan.id,
                            revision: plan.revision,
                        })
                    }
                    (None, None) if request.mode == RunMode::Plan => {
                        AgentRunMode::Plan(PlanDraftTarget::Create)
                    }
                    (None, None) => AgentRunMode::Execute,
                    _ => {
                        let _ = writer.append(invariant_failure());
                        return true;
                    }
                };
                let execution = self
                    .runtime
                    .run_public_model_with_mode_and_skills_stream_controlled(
                        &run.role,
                        &self.instructions,
                        &prompt,
                        max_turns,
                        &run.id,
                        &run.session_id,
                        create_session,
                        &request.skill_ids,
                        &allowed_tools,
                        mode,
                        request.end_user_id.as_deref(),
                        caller.remote_trace_context(),
                        caller.actor(),
                        &mut observer,
                        &control,
                    );
                match self
                    .interactions
                    .scope(Arc::clone(&writer), execution)
                    .await
                {
                    Ok(AgentRunOutcome::Completed { result }) => public_result(result),
                    Ok(AgentRunOutcome::Cancelled { result }) => public_cancellation(result),
                    Err(error) => runtime_failure(&error),
                }
            }
        };
        if fail_terminal_append {
            return false;
        }
        writer.append(kind).is_ok()
    }

    /// Cooperatively stop active public runs and unblock their interactions.
    pub fn request_shutdown(&self) {
        let active = lock(&self.execution)
            .active
            .iter()
            .map(|(run_id, run)| (run_id.clone(), run.clone()))
            .collect::<Vec<_>>();
        for (run_id, run) in active {
            run.control.cancel();
            self.interactions.cancel_run(&run_id);
        }
    }

    /// Cancel active runs and wait boundedly for their terminal durable updates.
    pub async fn shutdown_and_wait(&self, timeout: std::time::Duration) -> bool {
        self.request_shutdown();
        tokio::time::timeout(timeout, async {
            loop {
                let notified = self.active_changed.notified();
                if lock(&self.execution).active.is_empty() {
                    return;
                }
                notified.await;
            }
        })
        .await
        .is_ok()
    }
}

#[async_trait]
impl AgentRunApi for RuntimeAgentRunApi {
    async fn create_run(
        &self,
        caller: &CallerContext,
        request: CreateRunRequest,
    ) -> ApiResult<CreateRunResponse> {
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        request
            .validate()
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        if let Some(replay) = self.repository.resolve_create_run(caller, &request)? {
            return Ok(CreateRunResponse {
                run: self.recover_orphan(caller, replay)?,
            });
        }
        self.plan_selection(caller, &request)?;
        self.branch_source_session(caller, &request)?;
        self.rendered_input(caller, &request).await?;
        let role = request
            .role
            .clone()
            .unwrap_or_else(|| self.default_role.clone());
        caller.require_role(&role)?;
        let (session_id, create_session) =
            self.session_id(caller, request.session_id.as_deref())?;
        let new_run = NewRun::from_request(Uuid::now_v7().to_string(), session_id, role, &request)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let run = match self.create_admitted(caller, &request, &new_run, create_session)? {
            CreateDecision::Fresh { run, pending } => {
                self.spawn_execution(*pending);
                run
            }
            CreateDecision::Replay(run) => self.recover_orphan(caller, run)?,
        };
        Ok(CreateRunResponse { run })
    }

    async fn get_run(&self, caller: &CallerContext, request: GetRunRequest) -> ApiResult<Run> {
        let run = self
            .repository
            .get_run(caller, &request.run_id)?
            .ok_or_else(|| {
                ApiError::not_found(
                    ApiErrorReason::RunNotFound,
                    "the requested run was not found",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        self.recover_orphan(caller, run)
    }

    async fn list_runs(
        &self,
        caller: &CallerContext,
        request: ListRunsRequest,
    ) -> ApiResult<ListRunsResponse> {
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|AdmissionLimitReached| capacity_error(caller))?;
        self.repository.list_runs(caller, &request)
    }

    async fn watch_run(
        &self,
        caller: &CallerContext,
        request: WatchRunRequest,
    ) -> ApiResult<RunUpdateStream> {
        let run = self
            .repository
            .get_run(caller, &request.run_id)?
            .ok_or_else(|| {
                ApiError::not_found(
                    ApiErrorReason::RunNotFound,
                    "the requested run was not found",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        let run = self.recover_orphan(caller, run)?;
        let watch_permit = self
            .watches
            .acquire(caller.principal().application_id())
            .map_err(|AdmissionLimitReached| capacity_error(caller))?;
        let mut notifications = self.feeds.subscribe(&run.id, run.last_sequence);
        let repository = Arc::clone(&self.repository);
        let feeds = Arc::clone(&self.feeds);
        let caller = caller.clone();
        let run_id = run.id;
        let mut cursor = request.after_sequence;
        let (sender, receiver) = mpsc::channel(WATCH_CHANNEL_SIZE);
        tokio::spawn(async move {
            let _watch_permit = watch_permit;
            loop {
                match repository.updates_after(&caller, &run_id, cursor, WATCH_PAGE_SIZE) {
                    Ok(updates) => {
                        let had_updates = !updates.is_empty();
                        let terminal_update = updates
                            .iter()
                            .any(|update| update_is_terminal(&update.kind));
                        for update in updates {
                            cursor = update.sequence;
                            if sender.send(Ok(update)).await.is_err() {
                                return;
                            }
                        }
                        if terminal_update {
                            feeds.close(&run_id);
                            return;
                        }
                        if had_updates {
                            continue;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                }
                match repository.get_run(&caller, &run_id) {
                    Ok(Some(current)) if current.status.is_terminal() => {
                        feeds.close(&run_id);
                        return;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => return,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                }
                tokio::select! {
                    () = sender.closed() => return,
                    changed = notifications.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }

    async fn cancel_run(
        &self,
        caller: &CallerContext,
        request: CancelRunRequest,
    ) -> ApiResult<Run> {
        let active_run = lock(&self.execution).active.get(&request.run_id).cloned();
        if let Some(active_run) = active_run {
            let run = active_run
                .writer
                .request_cancellation(caller, &request.idempotency_key)?;
            active_run.control.cancel();
            self.interactions.cancel_run(&request.run_id);
            return Ok(run);
        }
        self.cancel_orphan(caller, &request)
    }

    async fn archive_thread(
        &self,
        caller: &CallerContext,
        request: ArchiveThreadRequest,
    ) -> ApiResult<ThreadLifecycle> {
        self.repository
            .archive_thread(caller, &request.run_id, &request.idempotency_key)
    }

    async fn restore_thread(
        &self,
        caller: &CallerContext,
        request: RestoreThreadRequest,
    ) -> ApiResult<ThreadLifecycle> {
        self.repository
            .restore_thread(caller, &request.run_id, &request.idempotency_key)
    }

    async fn respond_interaction(
        &self,
        caller: &CallerContext,
        request: RespondInteractionRequest,
    ) -> ApiResult<Interaction> {
        if let Some(replay) = self.repository.resolve_interaction_response(
            caller,
            &request.run_id,
            &request.interaction_id,
            &request.etag,
            &request.idempotency_key,
            &request.response,
        )? {
            let active_run = lock(&self.execution).active.get(&request.run_id).cloned();
            if let Some(active_run) = active_run {
                active_run.writer.synchronize_durable_state()?;
            }
            self.interactions.deliver(&request.run_id, &replay);
            return Ok(replay);
        }
        if !lock(&self.execution).active.contains_key(&request.run_id)
            && let Some((run, _)) = self.repository.recoverable_run(caller, &request.run_id)?
        {
            let _ = self.recover_orphan(caller, run)?;
        }
        let active_run = lock(&self.execution)
            .active
            .get(&request.run_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::failed_precondition(
                    ApiErrorReason::InteractionUnavailable,
                    "the interaction is not available",
                )
                .with_correlation_id(caller.request_id().clone())
            })?;
        let interaction = active_run.writer.respond_interaction(
            caller,
            &request.interaction_id,
            &request.etag,
            &request.idempotency_key,
            request.response,
        )?;
        // The response operation succeeds once the response is durably consumed.
        // A closed runtime waiter cannot retroactively make that known write fail.
        self.interactions.deliver(&request.run_id, &interaction);
        Ok(interaction)
    }
}

struct PublicRunObserver {
    writer: Arc<RunWriter>,
    accepts_related_run_events: bool,
}

#[async_trait]
impl RunEventObserver for PublicRunObserver {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        if !self.accepts_related_run_events && envelope.run_id != self.writer.run_id() {
            return Err(ModelProviderError::Failed(
                "public run event identity did not match".into(),
            ));
        }
        self.writer
            .append(public_event(envelope.event))
            .map(|_| ())
            .map_err(|_| ModelProviderError::Failed("public run feed is unavailable".into()))
    }
}

fn public_event(event: RunEvent) -> RunUpdateKind {
    match event {
        RunEvent::Provider { event } => match event {
            ProviderEvent::ModelDelta { text } => RunUpdateKind::OutputDelta { text },
            ProviderEvent::ReasoningSummary { summary } => {
                RunUpdateKind::ReasoningSummary { summary }
            }
            ProviderEvent::ToolCallRequested { call_id, name, .. } => RunUpdateKind::ToolActivity {
                activity: ToolActivity {
                    call_id,
                    tool_name: name,
                    state: ToolActivityState::Requested,
                    summary: "validated tool call requested".into(),
                    input: None,
                    preview: None,
                },
            },
            ProviderEvent::FinalOutput { .. } => RunUpdateKind::Notice {
                notice: colossus_api::RunNotice {
                    reason: "model.final_output".into(),
                    message: "the final visible output is available in the run result".into(),
                },
            },
            ProviderEvent::Usage { usage } => RunUpdateKind::Usage {
                usage: TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage.total_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                },
            },
        },
        RunEvent::Phase {
            phase,
            turn,
            action,
            ..
        } => {
            let phase = match phase {
                RunPhase::Preparing => "preparing",
                RunPhase::WaitingForModel => "waiting_for_model",
                RunPhase::Responding => "responding",
                RunPhase::Cancelling => "cancelling",
                RunPhase::Cancelled => "cancelled",
                RunPhase::Completed => "completed",
            };
            let mut message = format!("run phase changed to {phase}");
            if let Some(turn) = turn {
                message.push_str(&format!(" at turn {turn}"));
            }
            if let Some(action) = action {
                message.push_str(": ");
                message.push_str(&action);
            }
            RunUpdateKind::Notice {
                notice: colossus_api::RunNotice {
                    reason: format!("run.phase.{phase}"),
                    message,
                },
            }
        }
        RunEvent::ToolStarted { turn, call, .. } => {
            let input = released_tool_input(&call);
            RunUpdateKind::ToolActivity {
                activity: ToolActivity {
                    call_id: call.call_id,
                    tool_name: call.name,
                    state: ToolActivityState::Started,
                    summary: format!("tool execution started at turn {turn}"),
                    input,
                    preview: None,
                },
            }
        }
        RunEvent::ToolCompleted { turn, result, .. } => {
            let preview = released_tool_preview(&result);
            RunUpdateKind::ToolActivity {
                activity: ToolActivity {
                    call_id: result.call_id,
                    tool_name: result.name,
                    state: if result.exit_code == 0 {
                        ToolActivityState::Completed
                    } else {
                        ToolActivityState::Failed
                    },
                    summary: if result.exit_code == 0 {
                        format!("tool execution completed at turn {turn}")
                    } else {
                        format!("tool execution failed at turn {turn}")
                    },
                    input: None,
                    preview,
                },
            }
        }
        RunEvent::ToolCancelled { turn, call, .. } => RunUpdateKind::ToolActivity {
            activity: ToolActivity {
                call_id: call.call_id,
                tool_name: call.name,
                state: ToolActivityState::Cancelled,
                summary: format!("tool execution was cancelled before start at turn {turn}"),
                input: None,
                preview: None,
            },
        },
        RunEvent::PlanWritten { plan } => RunUpdateKind::Notice {
            notice: colossus_api::RunNotice {
                reason: "plan.written".into(),
                message: format!(
                    "plan {} was persisted at revision {}",
                    plan.id, plan.revision
                ),
            },
        },
        RunEvent::Error {
            code, recoverable, ..
        } => {
            let message = released_run_error_notice(&code, recoverable).into();
            RunUpdateKind::Notice {
                notice: colossus_api::RunNotice {
                    reason: code,
                    message,
                },
            }
        }
    }
}

fn bounded_tool_activity_text(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    if value.len() <= MAX_TOOL_ACTIVITY_PREVIEW_BYTES {
        return Some(value.to_owned());
    }
    let mut end = MAX_TOOL_ACTIVITY_PREVIEW_BYTES - TOOL_ACTIVITY_PREVIEW_TRUNCATION.len();
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut preview = String::with_capacity(MAX_TOOL_ACTIVITY_PREVIEW_BYTES);
    preview.push_str(&value[..end]);
    preview.push_str(TOOL_ACTIVITY_PREVIEW_TRUNCATION);
    Some(preview)
}

fn released_tool_input(call: &ToolCall) -> Option<String> {
    bounded_tool_activity_text(&serde_json::to_string(&call.arguments).ok()?)
}

fn released_tool_preview(result: &ToolResult) -> Option<String> {
    (result.exit_code == 0)
        .then(|| bounded_tool_activity_text(&result.output))
        .flatten()
}

fn released_run_error_notice(code: &str, recoverable: bool) -> &'static str {
    if recoverable {
        "the run encountered a recoverable error and will continue"
    } else if code == "tool.denied" {
        "the requested tool was denied before execution; review policy, tool access, and approval settings"
    } else {
        "the run encountered an error and could not continue"
    }
}

fn recovered_caller(
    execution: &RunExecutionRequest,
    run_id: &str,
    external: &CallerContext,
) -> ApiResult<CallerContext> {
    let credential_id = format!("recovery-{run_id}");
    let principal = ApplicationPrincipal::authenticated(
        execution.application_id.clone(),
        credential_id,
        execution.application_kind,
        execution.scopes.clone(),
        execution.allowed_roles.clone(),
        execution.allowed_tools.clone(),
    )
    .map_err(|_| recovery_invariant(external))?;
    let request_id =
        RequestId::new(format!("recovery-{run_id}")).map_err(|_| recovery_invariant(external))?;
    Ok(CallerContext::authenticated(principal, request_id)
        .with_remote_trace_context(execution.trace_context.clone()))
}

fn supported_text_attachment(media_type: &str) -> bool {
    media_type.starts_with("text/")
        || matches!(
            media_type,
            "application/json"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/xml"
        )
}

fn interrupted_failure() -> RunUpdateKind {
    RunUpdateKind::Failure {
        status: RunStatus::Interrupted,
        failure: RunFailure {
            code: "runtime.interrupted".into(),
            message: "the runtime stopped while the run was waiting for input".into(),
            outcome: OutcomeCertainty::Known,
            recoverable: true,
            http_status: None,
            retry_after_ms: None,
        },
    }
}

fn cancelled_before_start() -> RunUpdateKind {
    RunUpdateKind::Cancellation {
        cancellation: RunCancellation {
            turn: 0,
            message: "the run was cancelled before execution began".into(),
            plan_id: None,
            plan_revision: None,
            plan_status: None,
            goal_id: None,
        },
    }
}

fn public_result(result: AgentRunResult) -> RunUpdateKind {
    let (plan_id, plan_revision, plan_status) =
        result.plan.as_ref().map_or((None, None, None), |plan| {
            (
                Some(plan.id.clone()),
                Some(plan.revision),
                Some(public_plan_status(plan.status)),
            )
        });
    RunUpdateKind::Result {
        result: RunResult {
            output: result.output,
            plan_id,
            plan_revision,
            plan_status,
            goal_id: None,
            profile: result.profile,
            model_profile: result.model_profile,
            provider_profile: result.provider_profile,
            model: result.model,
            elapsed_seconds: result.elapsed_seconds,
        },
    }
}

fn research_result(result: colossus_contracts::ResearchRun) -> RunUpdateKind {
    let elapsed_seconds = result
        .completed_at
        .as_deref()
        .and_then(|completed_at| {
            let started = OffsetDateTime::parse(&result.created_at, &Rfc3339).ok()?;
            let completed = OffsetDateTime::parse(completed_at, &Rfc3339).ok()?;
            Some((completed - started).as_seconds_f64().max(0.0))
        })
        .unwrap_or(0.0);
    RunUpdateKind::Result {
        result: RunResult {
            output: result.report,
            plan_id: None,
            plan_revision: None,
            plan_status: None,
            goal_id: None,
            profile: "research".into(),
            model_profile: "research".into(),
            provider_profile: "research".into(),
            model: "research".into(),
            elapsed_seconds,
        },
    }
}

fn public_cancellation(result: AgentRunCancellation) -> RunUpdateKind {
    let (plan_id, plan_revision, plan_status) =
        result.plan.as_ref().map_or((None, None, None), |plan| {
            (
                Some(plan.id.clone()),
                Some(plan.revision),
                Some(public_plan_status(plan.status)),
            )
        });
    RunUpdateKind::Cancellation {
        cancellation: RunCancellation {
            turn: result.turn.into(),
            message: "the run was cancelled at a safe boundary".into(),
            plan_id,
            plan_revision,
            plan_status,
            goal_id: None,
        },
    }
}

fn public_plan_execution(outcome: PlanExecutionOutcome) -> RunUpdateKind {
    match outcome {
        PlanExecutionOutcome::CancelledBeforeStart { plan } => RunUpdateKind::Cancellation {
            cancellation: RunCancellation {
                turn: 0,
                message: "Plan execution was cancelled before the Plan was consumed".into(),
                plan_id: Some(plan.id),
                plan_revision: Some(plan.revision),
                plan_status: Some(public_plan_status(plan.status)),
                goal_id: None,
            },
        },
        PlanExecutionOutcome::Direct { plan, terminal } => match terminal {
            ControlledAgentTerminal::Completed { mut result } => {
                result.plan = Some(plan);
                public_result(result)
            }
            ControlledAgentTerminal::Cancelled { mut result } => {
                result.plan = Some(plan);
                public_cancellation(result)
            }
            ControlledAgentTerminal::Failed {
                message,
                outcome_unknown,
                ..
            } => plan_execution_failure(message, false, outcome_unknown),
        },
        PlanExecutionOutcome::Goal { plan, terminal } => match terminal {
            GoalRunOutcome::Completed { result } => public_goal_result(plan, result),
            GoalRunOutcome::Cancelled {
                result,
                cancellation,
            } => RunUpdateKind::Cancellation {
                cancellation: RunCancellation {
                    turn: cancellation
                        .as_ref()
                        .map_or(0, |value| u32::from(value.turn)),
                    message: "Goal execution was cancelled at a safe boundary".into(),
                    plan_id: Some(plan.id),
                    plan_revision: Some(plan.revision),
                    plan_status: Some(public_plan_status(plan.status)),
                    goal_id: Some(result.goal.id),
                },
            },
            GoalRunOutcome::Failed {
                message,
                outcome_unknown,
                result: _,
                run_id: _,
            } => plan_execution_failure(message, true, outcome_unknown),
        },
    }
}

fn public_goal_result(plan: PlanRecord, result: GoalRunResult) -> RunUpdateKind {
    let output = if !result.goal.summary.trim().is_empty() {
        result.goal.summary.clone()
    } else if let Some(iteration) = result.iterations.last() {
        iteration.output.clone()
    } else if result.iteration_budget_exhausted {
        "Goal iteration budget ended before additional output was produced.".into()
    } else {
        "The Goal reached its current terminal state.".into()
    };
    RunUpdateKind::Result {
        result: RunResult {
            output,
            plan_id: Some(plan.id),
            plan_revision: Some(plan.revision),
            plan_status: Some(public_plan_status(plan.status)),
            goal_id: Some(result.goal.id),
            profile: "goal".into(),
            model_profile: "goal".into(),
            provider_profile: "goal".into(),
            model: "goal".into(),
            elapsed_seconds: result.elapsed_seconds,
        },
    }
}

fn public_plan_status(status: PlanStatus) -> PublicPlanStatus {
    match status {
        PlanStatus::Draft => PublicPlanStatus::Draft,
        PlanStatus::Approved => PublicPlanStatus::Approved,
        PlanStatus::Executed => PublicPlanStatus::Executed,
        PlanStatus::Discarded => PublicPlanStatus::Discarded,
    }
}

fn plan_execution_failure(
    message: String,
    recoverable_if_known: bool,
    outcome_unknown: bool,
) -> RunUpdateKind {
    RunUpdateKind::Failure {
        status: if outcome_unknown {
            RunStatus::OutcomeUnknown
        } else {
            RunStatus::Failed
        },
        failure: RunFailure {
            code: if outcome_unknown {
                "plan.execution_outcome_unknown"
            } else {
                "plan.execution_failed"
            }
            .into(),
            message,
            outcome: if outcome_unknown {
                OutcomeCertainty::Unknown
            } else {
                OutcomeCertainty::Known
            },
            recoverable: !outcome_unknown && recoverable_if_known,
            http_status: None,
            retry_after_ms: None,
        },
    }
}

fn plan_selection_failure(error: ApiError) -> RunUpdateKind {
    let code = if error.reason == ApiErrorReason::ConcurrentModification {
        "plan.concurrent_modification"
    } else {
        "plan.selection_unavailable"
    };
    RunUpdateKind::Failure {
        status: RunStatus::Failed,
        failure: RunFailure {
            code: code.into(),
            message: error.message,
            outcome: error.outcome,
            recoverable: false,
            http_status: None,
            retry_after_ms: None,
        },
    }
}

fn invariant_failure() -> RunUpdateKind {
    RunUpdateKind::Failure {
        status: RunStatus::Failed,
        failure: RunFailure {
            code: "runtime.invariant_failed".into(),
            message: "the Plan run could not be started safely".into(),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: None,
            retry_after_ms: None,
        },
    }
}

fn unknown_outcome_failure() -> RunUpdateKind {
    RunUpdateKind::Failure {
        status: RunStatus::OutcomeUnknown,
        failure: RunFailure {
            code: "runtime.outcome_unknown".into(),
            message: "the runtime stopped while an external operation may have been active".into(),
            outcome: OutcomeCertainty::Unknown,
            recoverable: false,
            http_status: None,
            retry_after_ms: None,
        },
    }
}

fn update_is_terminal(kind: &RunUpdateKind) -> bool {
    matches!(
        kind,
        RunUpdateKind::Result { .. }
            | RunUpdateKind::Failure { .. }
            | RunUpdateKind::Cancellation { .. }
    )
}

fn missing_run(caller: &CallerContext) -> ApiError {
    ApiError::not_found(
        ApiErrorReason::RunNotFound,
        "the requested run was not found",
    )
    .with_correlation_id(caller.request_id().clone())
}

fn capacity_error(caller: &CallerContext) -> ApiError {
    ApiError::resource_exhausted(
        ApiErrorReason::CapacityExceeded,
        "public API admission capacity is temporarily exhausted",
    )
    .with_correlation_id(caller.request_id().clone())
}

fn recovery_invariant(caller: &CallerContext) -> ApiError {
    ApiError::from_store(
        &StoreError::Adapter("public run recovery invariant failed".into()),
        caller.request_id(),
    )
}

fn runtime_failure(error: &RuntimeError) -> RunUpdateKind {
    let failure = released_runtime_failure(error);
    let status = if failure.outcome == OutcomeCertainty::Unknown {
        RunStatus::OutcomeUnknown
    } else {
        RunStatus::Failed
    };
    RunUpdateKind::Failure { status, failure }
}

fn released_runtime_failure(error: &RuntimeError) -> RunFailure {
    match error {
        RuntimeError::Agent(colossus_agent::AgentError::Provider(error))
        | RuntimeError::Context(colossus_ports::ContextError::Provider(error)) => {
            released_provider_failure(error)
        }
        RuntimeError::Agent(colossus_agent::AgentError::Tool(
            colossus_ports::ToolError::OutcomeUnknown(_),
        ))
        | RuntimeError::Store(StoreError::OutcomeUnknown(_))
        | RuntimeError::SearchPort(colossus_ports::SearchError::OutcomeUnknown(_)) => {
            generic_failure(
                "runtime.outcome_unknown",
                "an external effect has no trustworthy terminal outcome",
                OutcomeCertainty::Unknown,
            )
        }
        RuntimeError::Agent(colossus_agent::AgentError::Tool(
            colossus_ports::ToolError::Denied(_),
        )) => generic_failure(
            "tool.denied",
            "the requested tool was denied before execution; review policy, tool access, and approval settings",
            OutcomeCertainty::Known,
        ),
        RuntimeError::Gateway(error) => released_gateway_failure(error),
        RuntimeError::Agent(colossus_agent::AgentError::MaxTurns { .. }) => generic_failure(
            "agent.max_turns",
            "the model reached the configured turn limit before producing a final response",
            OutcomeCertainty::Known,
        ),
        RuntimeError::Agent(colossus_agent::AgentError::EmptyTurn) => generic_failure(
            "provider.empty_turn",
            "the provider returned no visible response or tool call",
            OutcomeCertainty::Known,
        ),
        RuntimeError::Agent(colossus_agent::AgentError::ToolArgumentRecoveryExhausted {
            ..
        }) => generic_failure(
            "provider.invalid_tool_arguments",
            "the provider repeatedly returned invalid tool arguments",
            OutcomeCertainty::Known,
        ),
        _ if error.outcome_unknown() => generic_failure(
            "runtime.outcome_unknown",
            "an external effect has no trustworthy terminal outcome",
            OutcomeCertainty::Unknown,
        ),
        _ => generic_failure(
            "runtime.failed",
            "the run failed with a known outcome",
            OutcomeCertainty::Known,
        ),
    }
}

fn released_provider_failure(error: &ModelProviderError) -> RunFailure {
    match error {
        ModelProviderError::Recoverable {
            code,
            http_status,
            retry_after_ms,
            ..
        } => RunFailure {
            code: code.clone(),
            message: released_recoverable_provider_message(code, *http_status).into(),
            outcome: OutcomeCertainty::Known,
            recoverable: true,
            http_status: *http_status,
            retry_after_ms: *retry_after_ms,
        },
        ModelProviderError::HttpStatus { status, .. } => RunFailure {
            code: "provider.http_status".into(),
            message: format!("provider endpoint returned HTTP {status}"),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: Some(*status),
            retry_after_ms: None,
        },
        ModelProviderError::ResponseDiagnostic { diagnostic } => RunFailure {
            code: "provider.http_status".into(),
            message: format!("provider endpoint returned HTTP {}", diagnostic.status),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: Some(diagnostic.status),
            retry_after_ms: None,
        },
        ModelProviderError::Configuration(_) => generic_failure(
            "provider.configuration",
            "the configured provider request is invalid",
            OutcomeCertainty::Known,
        ),
        ModelProviderError::Failed(_) => generic_failure(
            "provider.failed",
            "the provider request failed with a known outcome",
            OutcomeCertainty::Known,
        ),
        ModelProviderError::OutcomeUnknown(_) => generic_failure(
            "provider.outcome_unknown",
            "provider transport failed after execution began; the outcome is unknown",
            OutcomeCertainty::Unknown,
        ),
    }
}

fn released_recoverable_provider_message(code: &str, http_status: Option<u16>) -> &'static str {
    match code {
        "provider.temporarily_unavailable" => {
            "provider endpoint returned HTTP 503; retry after the endpoint reports ready"
        }
        "provider.invalid_tool_arguments" => "the provider returned invalid tool arguments",
        _ if http_status.is_some() => "the provider returned a recoverable HTTP response",
        _ => "the provider request failed with a recoverable error",
    }
}

fn released_gateway_failure(error: &colossus_policy::GatewayError) -> RunFailure {
    match error {
        colossus_policy::GatewayError::RecoverableExecution {
            code,
            message,
            http_status,
            retry_after_ms,
        } => RunFailure {
            code: code.clone(),
            message: message.clone(),
            outcome: OutcomeCertainty::Known,
            recoverable: true,
            http_status: *http_status,
            retry_after_ms: *retry_after_ms,
        },
        colossus_policy::GatewayError::HttpStatus { status, message } => RunFailure {
            code: "effect.http_status".into(),
            message: message.clone(),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: Some(*status),
            retry_after_ms: None,
        },
        colossus_policy::GatewayError::OutcomeUnknown(_) => generic_failure(
            "effect.outcome_unknown",
            "an external effect has no trustworthy terminal outcome",
            OutcomeCertainty::Unknown,
        ),
        colossus_policy::GatewayError::Denied(_) => generic_failure(
            "effect.denied",
            "policy denied the requested effect",
            OutcomeCertainty::Known,
        ),
        colossus_policy::GatewayError::Approval(_) => generic_failure(
            "effect.approval_required",
            "the requested effect was not approved",
            OutcomeCertainty::Known,
        ),
        _ => generic_failure(
            "runtime.failed",
            "the run failed with a known outcome",
            OutcomeCertainty::Known,
        ),
    }
}

fn generic_failure(code: &str, message: &str, outcome: OutcomeCertainty) -> RunFailure {
    RunFailure {
        code: code.into(),
        message: message.into(),
        outcome,
        recoverable: false,
        http_status: None,
        retry_after_ms: None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{
        GoalRecord, GoalStatus, PlanRecord, PlanStatus, ResearchRun, ResearchStatus,
    };

    #[test]
    fn research_result_reports_canonical_wall_time() {
        let update = research_result(ResearchRun {
            id: "research-1".into(),
            session_id: "session-1".into(),
            question: "What is Rust ownership?".into(),
            depth: ResearchDepth::Quick,
            source_kinds: vec![ResearchSourceKind::Web],
            status: ResearchStatus::Completed,
            queries: Vec::new(),
            lanes: Vec::new(),
            progress: Vec::new(),
            limitations: Vec::new(),
            report: "Report".into(),
            error: String::new(),
            created_at: "2026-08-15T17:44:00Z".into(),
            updated_at: "2026-08-15T17:44:42.250Z".into(),
            completed_at: Some("2026-08-15T17:44:42.250Z".into()),
        });
        let RunUpdateKind::Result { result } = update else {
            panic!("research must produce a result update");
        };
        assert_eq!(result.elapsed_seconds, 42.25);
    }

    fn draft_plan() -> PlanRecord {
        PlanRecord {
            id: "plan-1".into(),
            session_id: "session-1".into(),
            prompt: "Draft the change".into(),
            status: PlanStatus::Draft,
            revision: 1,
            content: "Plan".into(),
            steps: Vec::new(),
            created_at: "2026-07-29T00:00:00Z".into(),
            updated_at: "2026-07-29T00:00:00Z".into(),
            approved_at: None,
            executed_run_id: None,
        }
    }

    fn executed_plan() -> PlanRecord {
        let mut plan = draft_plan();
        plan.status = PlanStatus::Executed;
        plan.revision = 3;
        plan.approved_at = Some("2026-07-29T00:01:00Z".into());
        plan.executed_run_id = Some("run-1".into());
        plan
    }

    fn active_goal_result() -> GoalRunResult {
        GoalRunResult {
            goal: GoalRecord {
                id: "goal-1".into(),
                session_id: "session-1".into(),
                objective: "Execute the Plan".into(),
                source_plan_id: Some("plan-1".into()),
                status: GoalStatus::Active,
                summary: String::new(),
                blocked_reason: String::new(),
                iteration_budget: 3,
                iterations_completed: 1,
                created_at: "2026-07-29T00:00:00Z".into(),
                updated_at: "2026-07-29T00:01:00Z".into(),
            },
            iterations: Vec::new(),
            iteration_budget_exhausted: false,
            elapsed_seconds: 0.25,
        }
    }

    #[test]
    fn public_error_updates_do_not_release_internal_error_text() {
        let private = "password=must-not-leak /private/provider/socket";
        let update = public_event(RunEvent::Error {
            code: "provider.failed".into(),
            message: private.into(),
            recoverable: false,
            http_status: None,
            retry_after_ms: None,
            turn: Some(1),
            elapsed_seconds: 0.25,
        });
        let RunUpdateKind::Notice { notice } = update else {
            panic!("error must project to a notice");
        };
        assert_eq!(notice.reason, "provider.failed");
        assert!(!notice.message.contains(private));
        assert!(!notice.message.contains("/private/provider/socket"));
    }

    #[test]
    fn denied_public_error_updates_explain_safe_configuration_checks() {
        let update = public_event(RunEvent::Error {
            code: "tool.denied".into(),
            message: "private policy detail".into(),
            recoverable: false,
            http_status: None,
            retry_after_ms: None,
            turn: Some(1),
            elapsed_seconds: 0.25,
        });
        let RunUpdateKind::Notice { notice } = update else {
            panic!("error must project to a notice");
        };
        assert_eq!(notice.reason, "tool.denied");
        assert_eq!(
            notice.message,
            "the requested tool was denied before execution; review policy, tool access, and approval settings"
        );
        assert!(!notice.message.contains("private policy detail"));
    }

    #[test]
    fn unstarted_public_tools_are_cancelled_instead_of_failed() {
        let update = public_event(RunEvent::ToolCancelled {
            turn: 1,
            call: ToolCall {
                call_id: "call-skipped".into(),
                name: "filesystem.list".into(),
                arguments: serde_json::json!({"path": "."}),
            },
            elapsed_seconds: 0.25,
        });
        let RunUpdateKind::ToolActivity { activity } = update else {
            panic!("cancelled tool must project to tool activity");
        };
        assert_eq!(activity.state, ToolActivityState::Cancelled);
        assert_eq!(activity.tool_name, "filesystem.list");
        assert!(activity.summary.contains("cancelled before start"));
        assert_eq!(activity.input, None);
        assert_eq!(activity.preview, None);
    }

    #[test]
    fn started_public_tool_releases_the_validated_execution_input() {
        let update = public_event(RunEvent::ToolStarted {
            turn: 1,
            call: ToolCall {
                call_id: "call-shell".into(),
                name: "shell.run".into(),
                arguments: serde_json::json!({
                    "command": "git status --short",
                    "cwd": "."
                }),
            },
            elapsed_seconds: 0.25,
        });
        let RunUpdateKind::ToolActivity { activity } = update else {
            panic!("started tool must project to tool activity");
        };
        let input: serde_json::Value =
            serde_json::from_str(activity.input.as_deref().expect("started tool input"))
                .expect("valid input JSON");
        assert_eq!(
            input,
            serde_json::json!({"command": "git status --short", "cwd": "."})
        );
        assert_eq!(activity.preview, None);
    }

    #[test]
    fn successful_public_preview_releases_the_bounded_tool_output() {
        let output = serde_json::json!({
            "root": ".",
            "files": [{
                "path": "src/main.rs",
                "bytes": 42,
                "extension": "rs"
            }],
            "file_count": 1,
            "extension_counts": {"rs": 1},
            "truncated": false
        })
        .to_string();
        let update = public_event(RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-map".into(),
                name: "repo.map".into(),
                output: output.clone(),
                exit_code: 0,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        });
        let RunUpdateKind::ToolActivity { activity } = update else {
            panic!("completed repository map must project to tool activity");
        };
        assert_eq!(activity.preview.as_deref(), Some(output.as_str()));
    }

    #[test]
    fn public_preview_truncation_is_bounded_marked_and_utf8_safe() {
        let output = "é".repeat(MAX_TOOL_ACTIVITY_PREVIEW_BYTES);
        let update = public_event(RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "call-shell".into(),
                name: "shell.run".into(),
                output,
                exit_code: 0,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.2,
        });
        let RunUpdateKind::ToolActivity { activity } = update else {
            panic!("completed tool must project to tool activity");
        };
        let preview = activity.preview.expect("successful tool preview");
        assert_eq!(preview.len(), MAX_TOOL_ACTIVITY_PREVIEW_BYTES);
        assert!(preview.ends_with(TOOL_ACTIVITY_PREVIEW_TRUNCATION));
        assert!(preview.is_char_boundary(preview.len()));
    }

    #[test]
    fn unsuccessful_and_empty_outputs_do_not_release_a_public_preview() {
        for result in [
            ToolResult {
                call_id: "call-failed".into(),
                name: "repo.map".into(),
                output: "failed tool detail".into(),
                exit_code: 1,
            },
            ToolResult {
                call_id: "call-empty".into(),
                name: "shell.run".into(),
                output: " \n ".into(),
                exit_code: 0,
            },
        ] {
            let update = public_event(RunEvent::ToolCompleted {
                turn: 1,
                result,
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            });
            let RunUpdateKind::ToolActivity { activity } = update else {
                panic!("completed tool must project to tool activity");
            };
            assert_eq!(activity.preview, None);
        }
    }

    #[test]
    fn public_plan_cancellation_preserves_the_written_plan_identity() {
        let update = public_cancellation(AgentRunCancellation {
            run_id: "run-1".into(),
            session_id: "session-1".into(),
            turn: 2,
            plan: Some(draft_plan()),
            event_count: 7,
            elapsed_seconds: 0.25,
        });

        let RunUpdateKind::Cancellation { cancellation } = update else {
            panic!("cancellation must remain terminal");
        };
        assert_eq!(cancellation.plan_id.as_deref(), Some("plan-1"));
    }

    #[test]
    fn public_plan_result_preserves_the_written_plan_identity() {
        let update = public_result(AgentRunResult {
            run_id: "run-1".into(),
            session_id: Some("session-1".into()),
            role: "primary".into(),
            profile: "default".into(),
            model_profile: "default".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            plan: Some(draft_plan()),
            output: "Plan saved".into(),
            event_count: 7,
            elapsed_seconds: 0.25,
        });

        let RunUpdateKind::Result { result } = update else {
            panic!("result must remain terminal");
        };
        assert_eq!(result.plan_id.as_deref(), Some("plan-1"));
    }

    #[test]
    fn direct_plan_failure_preserves_unknown_outcome_certainty() {
        let update = public_plan_execution(PlanExecutionOutcome::Direct {
            plan: executed_plan(),
            terminal: ControlledAgentTerminal::Failed {
                run_id: "run-1".into(),
                message: "effect outcome is unknown".into(),
                outcome_unknown: true,
            },
        });

        let RunUpdateKind::Failure { status, failure } = update else {
            panic!("failure must remain terminal");
        };
        assert_eq!(status, RunStatus::OutcomeUnknown);
        assert_eq!(failure.outcome, OutcomeCertainty::Unknown);
        assert!(!failure.recoverable);
    }

    #[test]
    fn goal_plan_failure_preserves_unknown_outcome_certainty() {
        let update = public_plan_execution(PlanExecutionOutcome::Goal {
            plan: executed_plan(),
            terminal: GoalRunOutcome::Failed {
                result: active_goal_result(),
                run_id: Some("run-1".into()),
                message: "effect outcome is unknown".into(),
                outcome_unknown: true,
            },
        });

        let RunUpdateKind::Failure { status, failure } = update else {
            panic!("failure must remain terminal");
        };
        assert_eq!(status, RunStatus::OutcomeUnknown);
        assert_eq!(failure.outcome, OutcomeCertainty::Unknown);
        assert!(
            !failure.recoverable,
            "unknown Goal outcomes cannot be retried"
        );
    }

    #[test]
    fn public_terminal_failure_preserves_safe_provider_response_metadata() {
        let update = released_runtime_failure(&RuntimeError::Agent(
            colossus_agent::AgentError::Provider(ModelProviderError::Recoverable {
                code: "provider.temporarily_unavailable".into(),
                message: "provider endpoint returned HTTP 503".into(),
                http_status: Some(503),
                retry_after_ms: Some(7_000),
            }),
        ));
        assert_eq!(update.code, "provider.temporarily_unavailable");
        assert_eq!(
            update.message,
            "provider endpoint returned HTTP 503; retry after the endpoint reports ready"
        );
        assert_eq!(update.outcome, OutcomeCertainty::Known);
        assert!(update.recoverable);
        assert_eq!(update.http_status, Some(503));
        assert_eq!(update.retry_after_ms, Some(7_000));
    }

    #[test]
    fn recoverable_provider_failures_do_not_release_provider_controlled_text() {
        let private = "call_id=hidden-prompt tool=/private/provider/socket";
        let update = released_runtime_failure(&RuntimeError::Agent(
            colossus_agent::AgentError::Provider(ModelProviderError::Recoverable {
                code: "provider.invalid_tool_arguments".into(),
                message: private.into(),
                http_status: None,
                retry_after_ms: None,
            }),
        ));
        assert_eq!(update.code, "provider.invalid_tool_arguments");
        assert_eq!(
            update.message,
            "the provider returned invalid tool arguments"
        );
        assert!(!update.message.contains(private));
        assert!(!update.message.contains("hidden-prompt"));
        assert!(!update.message.contains("/private/provider/socket"));
    }

    #[test]
    fn denied_tool_failures_are_typed_without_releasing_policy_details() {
        let private = "decision=secret path=/private/workspace args=password";
        let update = released_runtime_failure(&RuntimeError::Agent(
            colossus_agent::AgentError::Tool(colossus_ports::ToolError::Denied(private.into())),
        ));
        assert_eq!(update.code, "tool.denied");
        assert_eq!(
            update.message,
            "the requested tool was denied before execution; review policy, tool access, and approval settings"
        );
        assert_eq!(update.outcome, OutcomeCertainty::Known);
        assert!(!update.recoverable);
        assert!(!update.message.contains(private));
        assert!(!update.message.contains("password"));
        assert!(!update.message.contains("/private/workspace"));
    }

    #[test]
    fn provider_http_failures_release_only_the_numeric_status() {
        let private = "HTTP 418 body=password=must-not-leak";
        let update = released_runtime_failure(&RuntimeError::Agent(
            colossus_agent::AgentError::Provider(ModelProviderError::HttpStatus {
                status: 418,
                message: private.into(),
            }),
        ));
        assert_eq!(update.code, "provider.http_status");
        assert_eq!(update.message, "provider endpoint returned HTTP 418");
        assert_eq!(update.http_status, Some(418));
        assert!(!update.message.contains("password"));
    }

    #[test]
    fn unknown_provider_outcomes_do_not_release_transport_targets() {
        let private = "error sending request for url (http://localhost:8080/chat/completions)";
        let update =
            released_runtime_failure(&RuntimeError::Agent(colossus_agent::AgentError::Provider(
                ModelProviderError::OutcomeUnknown(private.into()),
            )));
        assert_eq!(update.code, "provider.outcome_unknown");
        assert_eq!(update.outcome, OutcomeCertainty::Unknown);
        assert!(!update.recoverable);
        assert!(!update.message.contains("localhost"));
        assert!(!update.message.contains("chat/completions"));
    }
}
