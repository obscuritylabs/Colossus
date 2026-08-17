use crate::{
    ApiError, ApiErrorReason, ApiResult, CallerContext, CreateRunRequest, Idempotent, Interaction,
    InteractionKind, InteractionResponse, InteractionStatus, ListRunsRequest, ListRunsResponse,
    NewRun, Run, RunExecutionRequest, RunStatus, RunUpdate, RunUpdateKind, ThreadLifecycle,
    identity::scopes,
    validate_public_approval_display,
    validation::{
        MAX_IDENTIFIER_BYTES, MAX_INPUT_BYTES, MAX_PAGE_SIZE, MAX_ROLE_BYTES, MAX_TOOL_BYTES,
        MAX_UPDATE_PAGE_SIZE, bounded_text, token,
    },
};
use colossus_contracts::{
    ActorType, EventClassification, EventEnvelope, ExecutionContext, NewEvent,
};
use colossus_ports::{EventJournal, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const CREATE_OPERATION: &str = "agent_run.create.v1";
const CANCEL_OPERATION: &str = "agent_run.cancel.v1";
const RESPOND_OPERATION: &str = "agent_run.interaction.respond.v1";
const ARCHIVE_THREAD_OPERATION: &str = "agent_run.thread.archive.v1";
const RESTORE_THREAD_OPERATION: &str = "agent_run.thread.restore.v1";
const IDEMPOTENCY_EVENT: &str = "api.idempotency.claimed.v1";
const RUN_CREATED_EVENT: &str = "api.run.created.v1";
const RUN_INDEXED_EVENT: &str = "api.run.indexed.v1";
const RUN_UPDATE_EVENT: &str = "api.run.update.v1";
const THREAD_ATTACHED_EVENT: &str = "api.thread.run.attached.v1";
const THREAD_ARCHIVED_EVENT: &str = "api.thread.archived.v1";
const THREAD_RESTORED_EVENT: &str = "api.thread.restored.v1";
const LIST_INDEX_READ_BATCH: usize = 8;
const MAX_LIST_INDEX_EVENTS_SCANNED: usize = 64;
const MAX_RUN_STREAM_EVENTS: usize = 4_099;
const MAX_LIST_RUN_EVENTS_RECONSTRUCTED: usize = MAX_RUN_STREAM_EVENTS * 4;
const MAX_CREATE_INDEX_CONFLICT_RETRIES: usize = 64;
const LIST_CURSOR_FORMAT_VERSION: u8 = 1;
const STORED_UPDATE_FORMAT_VERSION: u8 = 1;
const LIST_CURSOR_BYTES: usize = 1 + 8 + 8 + 32 + 32;
const LIST_CURSOR_HEX_BYTES: usize = LIST_CURSOR_BYTES * 2;
const MAX_NONTERMINAL_RUN_SEQUENCE: u64 = 4_096;
const MAX_RELEASED_BYTES_PER_RUN: usize = 16 * 1_048_576;
const MAX_THREAD_STREAM_EVENTS: u64 = 4_096;
const MAX_THREAD_RUNS_SCANNED: usize = 4_096;

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct CreateFingerprint<'request> {
    input: &'request [crate::ContentPart],
    session_id: &'request Option<String>,
    end_user_id: &'request Option<String>,
    role: &'request Option<String>,
    mode: crate::RunMode,
    research_depth: &'request Option<crate::ResearchDepth>,
    research_sources: &'request [crate::ResearchSourceKind],
    skill_ids: Vec<&'request str>,
    plan_action: &'request Option<crate::PlanRunAction>,
    branch: &'request Option<crate::RunBranch>,
    max_turns: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdempotencyClaim {
    operation: String,
    request_fingerprint: String,
    run_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCreated {
    id: String,
    session_id: String,
    role: String,
    mode: crate::RunMode,
    skill_ids: Vec<String>,
    execution: RunExecutionRequest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunIndexed {
    run_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadLifecycleEvent {
    session_id: String,
    archived: bool,
}

#[derive(Clone, Copy)]
struct RunListCursor {
    snapshot_version: u64,
    before_version: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRunState {
    status: RunStatus,
    started_at: Option<String>,
    finished_at: Option<String>,
    result: Option<crate::RunResult>,
    failure: Option<crate::RunFailure>,
    cancellation: Option<crate::RunCancellation>,
    pending_interaction: Option<Interaction>,
}

impl StoredRunState {
    fn capture(run: &Run) -> Self {
        Self {
            status: run.status,
            started_at: run.started_at.clone(),
            finished_at: run.finished_at.clone(),
            result: run.result.clone(),
            failure: run.failure.clone(),
            cancellation: run.cancellation.clone(),
            pending_interaction: run.pending_interaction.clone(),
        }
    }

    fn restore(&self, initial: &Run, sequence: u64) -> Run {
        let mut run = initial.clone();
        run.status = self.status;
        run.started_at.clone_from(&self.started_at);
        run.finished_at.clone_from(&self.finished_at);
        run.result.clone_from(&self.result);
        run.failure.clone_from(&self.failure);
        run.cancellation.clone_from(&self.cancellation);
        run.pending_interaction
            .clone_from(&self.pending_interaction);
        run.last_sequence = sequence;
        run.etag = run_etag(&run.id, sequence);
        run
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ReleasedUpdate<'update> {
    kind: &'update RunUpdateKind,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredUpdate {
    format_version: u8,
    prior_state: StoredRunState,
    released_bytes_before: u64,
    kind: RunUpdateKind,
}

/// Canonical durable run-feed repository used by every public API backend.
pub trait RunRepository: Send + Sync {
    /// Resolve an exact create idempotency replay without claiming a new run.
    ///
    /// Admission coordinators use this before checking capacity so a successful
    /// replay is never rejected merely because unrelated work filled a limit.
    fn resolve_create_run(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
    ) -> ApiResult<Option<Run>>;

    /// Atomically claim idempotency and create the initial queued run update.
    fn create_run(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
        new_run: &NewRun,
    ) -> ApiResult<Idempotent<Run>>;

    /// Reconstruct one current run.
    fn get_run(&self, caller: &CallerContext, run_id: &str) -> ApiResult<Option<Run>>;

    /// Recover the encrypted accepted execution request for one caller-owned run.
    fn execution_request(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<Option<RunExecutionRequest>>;

    /// Recover one same-application run and its accepted authority snapshot.
    ///
    /// This coordinator-only method accepts any run-management scope because it is
    /// used to settle orphaned resources after process loss. Execution always uses the
    /// captured grant returned here, never the triggering caller's current grant.
    fn recoverable_run(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<Option<(Run, RunExecutionRequest)>>;

    /// Return one stable newest-first page.
    fn list_runs(
        &self,
        caller: &CallerContext,
        request: &ListRunsRequest,
    ) -> ApiResult<ListRunsResponse>;

    /// Durably hide one terminal caller-owned thread.
    fn archive_thread(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
    ) -> ApiResult<ThreadLifecycle>;

    /// Durably restore one caller-owned thread.
    fn restore_thread(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
    ) -> ApiResult<ThreadLifecycle>;

    /// Append one safe released update with optimistic sequence concurrency.
    fn append_update(
        &self,
        caller: &CallerContext,
        run_id: &str,
        expected_sequence: u64,
        kind: RunUpdateKind,
    ) -> ApiResult<RunUpdate>;

    /// Atomically claim cancellation idempotency and persist the cancelling state.
    fn request_cancellation(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
    ) -> ApiResult<Idempotent<Run>>;

    /// Replay a bounded update page after an exclusive sequence cursor.
    fn updates_after(
        &self,
        caller: &CallerContext,
        run_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> ApiResult<Vec<RunUpdate>>;

    /// Resolve one caller-bound prompt or approval exactly once.
    fn respond_interaction(
        &self,
        caller: &CallerContext,
        run_id: &str,
        interaction_id: &str,
        etag: &str,
        idempotency_key: &crate::IdempotencyKey,
        response: InteractionResponse,
    ) -> ApiResult<Idempotent<Interaction>>;

    /// Resolve an exact interaction-response replay without mutating run state.
    fn resolve_interaction_response(
        &self,
        caller: &CallerContext,
        run_id: &str,
        interaction_id: &str,
        etag: &str,
        idempotency_key: &crate::IdempotencyKey,
        response: &InteractionResponse,
    ) -> ApiResult<Option<Interaction>>;
}

/// Immutable-journal implementation of [`RunRepository`].
pub struct EventSourcedRunRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedRunRepository {
    /// Bind public run resources to the authoritative encrypted journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn run_stream(run_id: &str) -> String {
        format!("api-run:{run_id}")
    }

    fn run_index_stream(caller: &CallerContext) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"colossus-api-run-index-v1\0");
        hasher.update(caller.principal().application_id().as_bytes());
        format!("api-run-index:{}", hex::encode(hasher.finalize()))
    }

    fn thread_stream(caller: &CallerContext, session_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"colossus-api-thread-v1\0");
        hasher.update(caller.principal().application_id().as_bytes());
        hasher.update(b"\0");
        hasher.update(session_id.as_bytes());
        format!("api-thread:{}", hex::encode(hasher.finalize()))
    }

    fn idempotency_stream(
        caller: &CallerContext,
        operation: &str,
        key: &crate::IdempotencyKey,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"colossus-api-idempotency-v1\0");
        hasher.update(caller.principal().application_id().as_bytes());
        hasher.update(b"\0");
        hasher.update(operation.as_bytes());
        hasher.update(b"\0");
        hasher.update(key.as_str().as_bytes());
        format!("api-idempotency:{}", hex::encode(hasher.finalize()))
    }

    fn request_fingerprint(request: &CreateRunRequest) -> ApiResult<String> {
        let mut skill_ids = request
            .skill_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        skill_ids.sort_unstable();
        let canonical = serde_json::to_vec(&CreateFingerprint {
            input: &request.input,
            session_id: &request.session_id,
            end_user_id: &request.end_user_id,
            role: &request.role,
            mode: request.mode,
            research_depth: &request.research_depth,
            research_sources: &request.research_sources,
            skill_ids,
            plan_action: &request.plan_action,
            branch: &request.branch,
            max_turns: request.max_turns,
        })
        .map_err(|_| ApiError::internal("the run request could not be normalized"))?;
        Ok(hex::encode(Sha256::digest(canonical)))
    }

    fn thread_operation_fingerprint(operation: &str, run_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"colossus-api-thread-operation-v1\0");
        hasher.update(operation.as_bytes());
        hasher.update(b"\0");
        hasher.update(run_id.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn thread_state(
        &self,
        caller: &CallerContext,
        session_id: &str,
    ) -> ApiResult<(ThreadLifecycle, u64)> {
        let stream_id = Self::thread_stream(caller, session_id);
        let events = self
            .journal
            .read_stream_backwards(&stream_id, None, 1)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(event) = events.first() else {
            return Ok((
                ThreadLifecycle {
                    session_id: session_id.into(),
                    archived: false,
                },
                0,
            ));
        };
        if event.stream_version == 0 || event.stream_version > MAX_THREAD_STREAM_EVENTS {
            return Err(invariant(
                caller,
                "the durable thread lifecycle exceeds its bounded event budget",
            ));
        }
        let lifecycle =
            decode_thread_lifecycle(self.journal.as_ref(), caller, &stream_id, session_id, event)?;
        Ok((lifecycle, event.stream_version))
    }

    fn replay_thread_operation(
        &self,
        caller: &CallerContext,
        idempotency_stream: &str,
        operation: &str,
        request_fingerprint: &str,
        archived: bool,
    ) -> ApiResult<Option<ThreadLifecycle>> {
        let events = self
            .journal
            .read_stream(idempotency_stream)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        if events.len() != 1
            || first.event_type != IDEMPOTENCY_EVENT
            || first.actor.actor_type != ActorType::Application
            || first.actor.id != caller.principal().application_id()
        {
            return Err(invariant(
                caller,
                "durable thread idempotency evidence could not be verified",
            ));
        }
        let claim: IdempotencyClaim = self
            .journal
            .decrypt_payload(first)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))
            .and_then(|payload| {
                serde_json::from_value(payload).map_err(|_| {
                    invariant(
                        caller,
                        "durable thread idempotency evidence could not be decoded",
                    )
                })
            })?;
        if claim.operation != operation {
            return Err(invariant(
                caller,
                "durable thread idempotency operation could not be verified",
            ));
        }
        if claim.request_fingerprint != request_fingerprint {
            return Err(ApiError::conflict(
                ApiErrorReason::IdempotencyKeyReused,
                "the idempotency key was already used for a different request",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let run = self.load(caller, &claim.run_id)?.ok_or_else(|| {
            invariant(
                caller,
                "the thread idempotency claim references an absent durable run",
            )
        })?;
        Ok(Some(ThreadLifecycle {
            session_id: run.session_id,
            archived,
        }))
    }

    fn ensure_thread_terminal(&self, caller: &CallerContext, session_id: &str) -> ApiResult<()> {
        let index_stream = Self::run_index_stream(caller);
        let mut before_version = None;
        let mut scanned = 0_usize;
        loop {
            let remaining = MAX_THREAD_RUNS_SCANNED.saturating_sub(scanned);
            if remaining == 0 {
                let has_more = !self
                    .journal
                    .read_stream_backwards(&index_stream, before_version, 1)
                    .map_err(|error| ApiError::from_store(&error, caller.request_id()))?
                    .is_empty();
                if has_more {
                    return Err(ApiError::bounded_resource_exhausted(
                        ApiErrorReason::CapacityExceeded,
                        "the thread is too large to archive safely",
                    )
                    .with_correlation_id(caller.request_id().clone()));
                }
                return Ok(());
            }
            let read_limit = remaining.min(colossus_ports::MAX_STREAM_READ_BATCH);
            let events = self
                .journal
                .read_stream_backwards(&index_stream, before_version, read_limit)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
            if events.is_empty() {
                return Ok(());
            }
            for event in &events {
                let indexed = decode_indexed(self.journal.as_ref(), caller, &index_stream, event)?;
                scanned = scanned.saturating_add(1);
                before_version = Some(event.stream_version);
                if event.context.session_id.as_deref() != Some(session_id) {
                    continue;
                }
                let (run, _) = self
                    .load_append_state(caller, &indexed.run_id)?
                    .ok_or_else(|| {
                        invariant(caller, "the thread index references an absent run")
                    })?;
                if !run.status.is_terminal() {
                    return Err(ApiError::failed_precondition(
                        ApiErrorReason::InvalidRunTransition,
                        "finish or cancel all work in this thread before archiving it",
                    )
                    .with_correlation_id(caller.request_id().clone()));
                }
            }
            if events.len() < read_limit {
                return Ok(());
            }
        }
    }

    fn set_thread_archived(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
        archived: bool,
    ) -> ApiResult<ThreadLifecycle> {
        caller.require_scope(scopes::RUNS_CONTROL)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let operation = if archived {
            ARCHIVE_THREAD_OPERATION
        } else {
            RESTORE_THREAD_OPERATION
        };
        let fingerprint = Self::thread_operation_fingerprint(operation, run_id);
        let idempotency_stream = Self::idempotency_stream(caller, operation, idempotency_key);
        if let Some(replay) = self.replay_thread_operation(
            caller,
            &idempotency_stream,
            operation,
            &fingerprint,
            archived,
        )? {
            return Ok(replay);
        }
        let run = self.load(caller, run_id)?.ok_or_else(|| {
            ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested run was not found",
            )
            .with_correlation_id(caller.request_id().clone())
        })?;
        let stream_id = Self::thread_stream(caller, &run.session_id);
        let actor = caller.actor();
        let context = ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            session_id: Some(run.session_id.clone()),
            run_id: Some(run.id.clone()),
            ..ExecutionContext::default()
        };
        for _ in 0..MAX_CREATE_INDEX_CONFLICT_RETRIES {
            if let Some(replay) = self.replay_thread_operation(
                caller,
                &idempotency_stream,
                operation,
                &fingerprint,
                archived,
            )? {
                return Ok(replay);
            }
            if archived {
                self.ensure_thread_terminal(caller, &run.session_id)?;
            }
            let (current, expected_version) = self.thread_state(caller, &run.session_id)?;
            let claim = NewEvent {
                event_version: 1,
                stream_id: idempotency_stream.clone(),
                expected_stream_version: 0,
                classification: EventClassification::System,
                event_type: IDEMPOTENCY_EVENT.into(),
                actor: actor.clone(),
                context: context.clone(),
                payload: json!({
                    "operation": operation,
                    "request_fingerprint": fingerprint,
                    "run_id": run.id,
                }),
            };
            let mut events = vec![claim];
            if current.archived != archived {
                if expected_version >= MAX_THREAD_STREAM_EVENTS {
                    return Err(ApiError::bounded_resource_exhausted(
                        ApiErrorReason::CapacityExceeded,
                        "the thread reached its lifecycle event budget",
                    )
                    .with_correlation_id(caller.request_id().clone()));
                }
                events.push(NewEvent {
                    event_version: 1,
                    stream_id: stream_id.clone(),
                    expected_stream_version: expected_version,
                    classification: EventClassification::Domain,
                    event_type: if archived {
                        THREAD_ARCHIVED_EVENT.into()
                    } else {
                        THREAD_RESTORED_EVENT.into()
                    },
                    actor: actor.clone(),
                    context: context.clone(),
                    payload: serde_json::to_value(ThreadLifecycleEvent {
                        session_id: run.session_id.clone(),
                        archived,
                    })
                    .map_err(|_| invariant(caller, "the thread lifecycle could not be encoded"))?,
                });
            }
            match self.journal.append_batch(events) {
                Ok(_) => {
                    return Ok(ThreadLifecycle {
                        session_id: run.session_id.clone(),
                        archived,
                    });
                }
                Err(StoreError::Conflict {
                    stream_id: conflict,
                    ..
                }) if conflict == idempotency_stream => {
                    return self
                        .replay_thread_operation(
                            caller,
                            &idempotency_stream,
                            operation,
                            &fingerprint,
                            archived,
                        )?
                        .ok_or_else(|| {
                            invariant(
                                caller,
                                "the concurrent thread idempotency claim disappeared",
                            )
                        });
                }
                Err(StoreError::Conflict {
                    stream_id: conflict,
                    ..
                }) if conflict == stream_id => {
                    continue;
                }
                Err(error) => return Err(ApiError::from_store(&error, caller.request_id())),
            }
        }
        Err(ApiError::resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "the thread lifecycle is contending with other requests; retry with the same idempotency key",
        )
        .with_correlation_id(caller.request_id().clone()))
    }

    fn replay(
        &self,
        caller: &CallerContext,
        idempotency_stream: &str,
        operation: &str,
        request_fingerprint: &str,
    ) -> ApiResult<Option<Idempotent<Run>>> {
        let events = self
            .journal
            .read_stream(idempotency_stream)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        if events.len() != 1
            || first.event_type != IDEMPOTENCY_EVENT
            || first.actor.actor_type != ActorType::Application
            || first.actor.id != caller.principal().application_id()
        {
            return Err(invariant(
                caller,
                "durable idempotency evidence could not be verified",
            ));
        }
        let claim: IdempotencyClaim = self
            .journal
            .decrypt_payload(first)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))
            .and_then(|payload| {
                serde_json::from_value(payload).map_err(|_| {
                    invariant(caller, "durable idempotency evidence could not be decoded")
                })
            })?;
        if claim.operation != operation {
            return Err(invariant(
                caller,
                "durable idempotency operation could not be verified",
            ));
        }
        if claim.request_fingerprint != request_fingerprint {
            return Err(ApiError::conflict(
                ApiErrorReason::IdempotencyKeyReused,
                "the idempotency key was already used for a different request",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let run = self.load(caller, &claim.run_id)?.ok_or_else(|| {
            invariant(
                caller,
                "the idempotency claim references an absent durable run",
            )
        })?;
        Ok(Some(Idempotent {
            value: run,
            replayed: true,
        }))
    }

    fn load(&self, caller: &CallerContext, run_id: &str) -> ApiResult<Option<Run>> {
        let events = self
            .journal
            .read_stream(&Self::run_stream(run_id))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        if events.is_empty() {
            return Ok(None);
        }
        if !visible_to(caller, &events)? {
            return Ok(None);
        }
        let mut run = reconstruct(self.journal.as_ref(), caller, run_id, &events)?;
        run.archived = self.thread_state(caller, &run.session_id)?.0.archived;
        Ok(Some(run))
    }

    fn read_bounded_run_stream(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<Vec<EventEnvelope>> {
        let stream_id = Self::run_stream(run_id);
        let mut events = Vec::new();
        let mut after_version = 0_u64;
        loop {
            let remaining = MAX_RUN_STREAM_EVENTS
                .saturating_add(1)
                .saturating_sub(events.len());
            if remaining == 0 {
                return Err(invariant(
                    caller,
                    "the durable run exceeds its bounded event budget",
                ));
            }
            let read_limit = remaining.min(colossus_ports::MAX_STREAM_READ_BATCH);
            let page = self
                .journal
                .read_stream_from(&stream_id, after_version, read_limit)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
            if page.is_empty() {
                break;
            }
            after_version = page
                .last()
                .map_or(after_version, |event| event.stream_version);
            let page_len = page.len();
            events.extend(page);
            if events.len() > MAX_RUN_STREAM_EVENTS {
                return Err(invariant(
                    caller,
                    "the durable run exceeds its bounded event budget",
                ));
            }
            if page_len < read_limit {
                break;
            }
        }
        Ok(events)
    }

    fn load_append_state(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<Option<(Run, u64)>> {
        let stream_id = Self::run_stream(run_id);
        let owner_events = self
            .journal
            .read_stream_from(&stream_id, 0, 1)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(first) = owner_events.first() else {
            return Ok(None);
        };
        if !visible_to(caller, &owner_events)? {
            return Ok(None);
        }
        let created = decode_created(self.journal.as_ref(), caller, run_id, first)?;
        let initial = initial_run_from_created(first, &created);
        let tail = self
            .journal
            .read_stream_backwards(&stream_id, None, 2)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(event) = tail.first() else {
            return Err(invariant(
                caller,
                "the durable run tail could not be verified",
            ));
        };
        if event.stream_version == 1 {
            if tail.len() != 1 || event != first {
                return Err(invariant(
                    caller,
                    "the durable run creation tail could not be verified",
                ));
            }
            return Ok(Some((initial, 0)));
        }
        if event.stream_version > u64::try_from(MAX_RUN_STREAM_EVENTS).unwrap_or(u64::MAX) {
            return Err(invariant(
                caller,
                "the durable run exceeds its bounded event budget",
            ));
        }
        let previous = tail.get(1).ok_or_else(|| {
            invariant(
                caller,
                "the durable run tail predecessor could not be verified",
            )
        })?;
        if previous.stream_version.checked_add(1) != Some(event.stream_version) {
            return Err(invariant(
                caller,
                "the durable run tail sequence could not be verified",
            ));
        }
        let (mut run, released_bytes_before) = if previous.stream_version == 1 {
            if previous != first {
                return Err(invariant(
                    caller,
                    "the durable run creation predecessor could not be verified",
                ));
            }
            (initial.clone(), 0)
        } else {
            let previous_stored =
                validated_stored_update(self.journal.as_ref(), caller, run_id, first, previous)?;
            if previous.stream_version == 2
                && (previous_stored.prior_state != StoredRunState::capture(&initial)
                    || previous_stored.released_bytes_before != 0)
            {
                return Err(invariant(
                    caller,
                    "the durable run initial projection could not be verified",
                ));
            }
            let previous_prior_sequence =
                previous.stream_version.checked_sub(1).ok_or_else(|| {
                    invariant(caller, "the durable run predecessor sequence is invalid")
                })?;
            let mut previous_run = previous_stored
                .prior_state
                .restore(&initial, previous_prior_sequence);
            validate_stored_state(caller, &previous_run)?;
            validate_update_owner(caller, &previous_stored.kind)?;
            apply_update(
                &mut previous_run,
                previous.stream_version,
                &previous.occurred_at,
                &previous_stored.kind,
            )
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
            let previous_released_bytes = previous_stored
                .released_bytes_before
                .checked_add(released_update_bytes(caller, &previous_stored.kind)?)
                .ok_or_else(|| invariant(caller, "the released run byte count overflowed"))?;
            (previous_run, previous_released_bytes)
        };
        let stored = validated_stored_update(self.journal.as_ref(), caller, run_id, first, event)?;
        if stored.prior_state != StoredRunState::capture(&run)
            || stored.released_bytes_before != released_bytes_before
        {
            return Err(invariant(
                caller,
                "the durable run tail projection could not be verified",
            ));
        }
        validate_update_owner(caller, &stored.kind)?;
        apply_update(
            &mut run,
            event.stream_version,
            &event.occurred_at,
            &stored.kind,
        )
        .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let released_bytes = released_bytes_before
            .checked_add(released_update_bytes(caller, &stored.kind)?)
            .ok_or_else(|| invariant(caller, "the released run byte count overflowed"))?;
        Ok(Some((run, released_bytes)))
    }

    fn append_unchecked(
        &self,
        caller: &CallerContext,
        run: &Run,
        released_bytes_before: u64,
        kind: RunUpdateKind,
    ) -> ApiResult<RunUpdate> {
        validate_update_owner(caller, &kind)?;
        validate_update(run, &kind).map_err(|error| {
            if error.correlation_id.is_none() {
                error.with_correlation_id(caller.request_id().clone())
            } else {
                error
            }
        })?;
        let envelope = self
            .journal
            .append(NewEvent {
                event_version: 1,
                stream_id: Self::run_stream(&run.id),
                expected_stream_version: run.last_sequence,
                classification: EventClassification::Domain,
                event_type: RUN_UPDATE_EVENT.into(),
                actor: caller.actor(),
                context: ExecutionContext {
                    correlation_id: caller.request_id().as_str().into(),
                    session_id: Some(run.session_id.clone()),
                    run_id: Some(run.id.clone()),
                    ..ExecutionContext::default()
                },
                payload: stored_update_payload(caller, run, released_bytes_before, kind.clone())?,
            })
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        Ok(RunUpdate {
            run_id: run.id.clone(),
            sequence: envelope.stream_version,
            occurred_at: envelope.occurred_at,
            kind,
        })
    }

    fn replay_interaction(
        &self,
        caller: &CallerContext,
        idempotency_stream: &str,
        request_fingerprint: &str,
        interaction_id: &str,
    ) -> ApiResult<Option<Idempotent<Interaction>>> {
        let events = self
            .journal
            .read_stream(idempotency_stream)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        if events.len() != 1
            || first.event_type != IDEMPOTENCY_EVENT
            || first.actor.actor_type != ActorType::Application
            || first.actor.id != caller.principal().application_id()
        {
            return Err(invariant(
                caller,
                "durable interaction idempotency evidence could not be verified",
            ));
        }
        let claim: IdempotencyClaim = self
            .journal
            .decrypt_payload(first)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))
            .and_then(|payload| {
                serde_json::from_value(payload).map_err(|_| {
                    invariant(
                        caller,
                        "durable interaction idempotency evidence could not be decoded",
                    )
                })
            })?;
        if claim.operation != RESPOND_OPERATION {
            return Err(invariant(
                caller,
                "durable interaction idempotency operation could not be verified",
            ));
        }
        if claim.request_fingerprint != request_fingerprint {
            return Err(ApiError::conflict(
                ApiErrorReason::IdempotencyKeyReused,
                "the idempotency key was already used for a different request",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let interaction = self
            .interaction_history(caller, &claim.run_id, interaction_id)?
            .filter(|interaction| interaction.status == InteractionStatus::Responded)
            .ok_or_else(|| {
                invariant(
                    caller,
                    "the idempotency claim references an absent interaction response",
                )
            })?;
        require_interaction_response_scope(caller, interaction.kind)?;
        Ok(Some(Idempotent {
            value: interaction,
            replayed: true,
        }))
    }

    fn interaction_history(
        &self,
        caller: &CallerContext,
        run_id: &str,
        interaction_id: &str,
    ) -> ApiResult<Option<Interaction>> {
        if self.load(caller, run_id)?.is_none() {
            return Ok(None);
        }
        let events = self
            .journal
            .read_stream(&Self::run_stream(run_id))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        for event in events.iter().rev() {
            if event.event_type != RUN_UPDATE_EVENT {
                continue;
            }
            let stored = decode_stored_update(self.journal.as_ref(), caller, event)?;
            if stored.format_version != STORED_UPDATE_FORMAT_VERSION {
                return Err(invariant(
                    caller,
                    "the durable run update format is unsupported",
                ));
            }
            if let RunUpdateKind::Interaction { interaction } = stored.kind
                && interaction.id == interaction_id
            {
                return Ok(Some(interaction));
            }
        }
        Ok(None)
    }
}

impl RunRepository for EventSourcedRunRepository {
    fn resolve_create_run(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
    ) -> ApiResult<Option<Run>> {
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        request.validate().map_err(|error| {
            if error.correlation_id.is_none() {
                error.with_correlation_id(caller.request_id().clone())
            } else {
                error
            }
        })?;
        let fingerprint = Self::request_fingerprint(request)?;
        let idempotency_stream =
            Self::idempotency_stream(caller, CREATE_OPERATION, &request.idempotency_key);
        self.replay(caller, &idempotency_stream, CREATE_OPERATION, &fingerprint)
            .map(|replay| replay.map(|replay| replay.value))
    }

    fn create_run(
        &self,
        caller: &CallerContext,
        request: &CreateRunRequest,
        new_run: &NewRun,
    ) -> ApiResult<Idempotent<Run>> {
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        request.validate().map_err(|error| {
            if error.correlation_id.is_none() {
                error.with_correlation_id(caller.request_id().clone())
            } else {
                error
            }
        })?;
        caller.require_role(new_run.role())?;
        let fingerprint = Self::request_fingerprint(request)?.to_owned();
        let idempotency_stream =
            Self::idempotency_stream(caller, CREATE_OPERATION, &request.idempotency_key);
        if let Some(replay) =
            self.replay(caller, &idempotency_stream, CREATE_OPERATION, &fingerprint)?
        {
            return Ok(replay);
        }

        let created_payload = serde_json::to_value(RunCreated {
            id: new_run.id().into(),
            session_id: new_run.session_id().into(),
            role: new_run.role().into(),
            mode: request.mode,
            skill_ids: request.skill_ids.clone(),
            execution: RunExecutionRequest::capture(caller, request),
        })
        .map_err(|_| invariant(caller, "the initial run could not be encoded"))?;
        let claim_payload = json!({
            "operation": CREATE_OPERATION,
            "request_fingerprint": fingerprint,
            "run_id": new_run.id(),
        });
        let index_payload = serde_json::to_value(RunIndexed {
            run_id: new_run.id().into(),
        })
        .map_err(|_| invariant(caller, "the run index entry could not be encoded"))?;
        let thread_payload = serde_json::to_value(ThreadLifecycleEvent {
            session_id: new_run.session_id().into(),
            archived: false,
        })
        .map_err(|_| invariant(caller, "the thread membership could not be encoded"))?;
        let context = ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            session_id: Some(new_run.session_id().into()),
            run_id: Some(new_run.id().into()),
            ..ExecutionContext::default()
        };
        let actor = caller.actor();
        let index_stream = Self::run_index_stream(caller);
        let thread_stream = Self::thread_stream(caller, new_run.session_id());
        for _ in 0..MAX_CREATE_INDEX_CONFLICT_RETRIES {
            if let Some(replay) =
                self.replay(caller, &idempotency_stream, CREATE_OPERATION, &fingerprint)?
            {
                return Ok(replay);
            }
            let (thread, expected_thread_version) =
                self.thread_state(caller, new_run.session_id())?;
            if thread.archived {
                return Err(ApiError::failed_precondition(
                    ApiErrorReason::InvalidRunTransition,
                    "restore this thread before adding more work",
                )
                .with_correlation_id(caller.request_id().clone()));
            }
            if expected_thread_version >= MAX_THREAD_STREAM_EVENTS {
                return Err(ApiError::bounded_resource_exhausted(
                    ApiErrorReason::CapacityExceeded,
                    "the thread reached its durable run budget",
                )
                .with_correlation_id(caller.request_id().clone()));
            }
            let tail = self
                .journal
                .read_stream_backwards(&index_stream, None, 1)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
            let expected_index_version = match tail.as_slice() {
                [] => 0,
                [event] => {
                    decode_indexed(self.journal.as_ref(), caller, &index_stream, event)?;
                    event.stream_version
                }
                _ => {
                    return Err(invariant(
                        caller,
                        "the durable run index tail is not bounded",
                    ));
                }
            };
            let append = self.journal.append_batch(vec![
                NewEvent {
                    event_version: 1,
                    stream_id: idempotency_stream.clone(),
                    expected_stream_version: 0,
                    classification: EventClassification::System,
                    event_type: IDEMPOTENCY_EVENT.into(),
                    actor: actor.clone(),
                    context: context.clone(),
                    payload: claim_payload.clone(),
                },
                NewEvent {
                    event_version: 1,
                    stream_id: Self::run_stream(new_run.id()),
                    expected_stream_version: 0,
                    classification: EventClassification::Domain,
                    event_type: RUN_CREATED_EVENT.into(),
                    actor: actor.clone(),
                    context: context.clone(),
                    payload: created_payload.clone(),
                },
                NewEvent {
                    event_version: 1,
                    stream_id: index_stream.clone(),
                    expected_stream_version: expected_index_version,
                    classification: EventClassification::System,
                    event_type: RUN_INDEXED_EVENT.into(),
                    actor: actor.clone(),
                    context: context.clone(),
                    payload: index_payload.clone(),
                },
                NewEvent {
                    event_version: 1,
                    stream_id: thread_stream.clone(),
                    expected_stream_version: expected_thread_version,
                    classification: EventClassification::Domain,
                    event_type: THREAD_ATTACHED_EVENT.into(),
                    actor: actor.clone(),
                    context: context.clone(),
                    payload: thread_payload.clone(),
                },
            ]);
            let envelopes = match append {
                Ok(envelopes) => envelopes,
                Err(StoreError::Conflict { stream_id, .. }) if stream_id == idempotency_stream => {
                    return self
                        .replay(caller, &idempotency_stream, CREATE_OPERATION, &fingerprint)?
                        .ok_or_else(|| {
                            invariant(caller, "the concurrent idempotency claim disappeared")
                        });
                }
                Err(StoreError::Conflict { stream_id, .. }) if stream_id == index_stream => {
                    continue;
                }
                Err(StoreError::Conflict { stream_id, .. }) if stream_id == thread_stream => {
                    continue;
                }
                Err(error) => return Err(ApiError::from_store(&error, caller.request_id())),
            };
            let run_envelope = envelopes.get(1).ok_or_else(|| {
                invariant(
                    caller,
                    "atomic run creation did not return its durable envelope",
                )
            })?;
            let index_envelope = envelopes.get(2).ok_or_else(|| {
                invariant(
                    caller,
                    "atomic run creation did not return its durable index entry",
                )
            })?;
            let thread_envelope = envelopes.get(3).ok_or_else(|| {
                invariant(
                    caller,
                    "atomic run creation did not return its durable thread membership",
                )
            })?;
            if envelopes.len() != 4
                || index_envelope.stream_id != index_stream
                || index_envelope.stream_version != expected_index_version.saturating_add(1)
                || thread_envelope.stream_id != thread_stream
                || thread_envelope.stream_version != expected_thread_version.saturating_add(1)
            {
                return Err(invariant(
                    caller,
                    "atomic run creation returned invalid durable evidence",
                ));
            }
            let run = initial_run(run_envelope, new_run, request);
            return Ok(Idempotent {
                value: run,
                replayed: false,
            });
        }
        Err(ApiError::resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "run creation is contending with other requests; retry with the same idempotency key",
        )
        .with_correlation_id(caller.request_id().clone()))
    }

    fn get_run(&self, caller: &CallerContext, run_id: &str) -> ApiResult<Option<Run>> {
        caller.require_scope(scopes::RUNS_READ)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        self.load(caller, run_id)
    }

    fn execution_request(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<Option<RunExecutionRequest>> {
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let events = self
            .journal
            .read_stream(&Self::run_stream(run_id))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        if events.is_empty() || !visible_to(caller, &events)? {
            return Ok(None);
        }
        let first = events
            .first()
            .ok_or_else(|| invariant(caller, "the durable run creation event disappeared"))?;
        let created = decode_created(self.journal.as_ref(), caller, run_id, first)?;
        Ok(Some(created.execution))
    }

    fn recoverable_run(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<Option<(Run, RunExecutionRequest)>> {
        require_run_management_scope(caller)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let events = self
            .journal
            .read_stream(&Self::run_stream(run_id))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        if events.is_empty() || !visible_to(caller, &events)? {
            return Ok(None);
        }
        let mut run = reconstruct(self.journal.as_ref(), caller, run_id, &events)?;
        run.archived = self.thread_state(caller, &run.session_id)?.0.archived;
        let first = events
            .first()
            .ok_or_else(|| invariant(caller, "the durable run creation event disappeared"))?;
        let created = decode_created(self.journal.as_ref(), caller, run_id, first)?;
        Ok(Some((run, created.execution)))
    }

    fn list_runs(
        &self,
        caller: &CallerContext,
        request: &ListRunsRequest,
    ) -> ApiResult<ListRunsResponse> {
        caller.require_scope(scopes::RUNS_READ)?;
        if let Some(session_id) = &request.session_id {
            token(session_id, "session_id", MAX_IDENTIFIER_BYTES)
                .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        }
        if request.statuses.len() > 9
            || request.statuses.iter().collect::<BTreeSet<_>>().len() != request.statuses.len()
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "statuses",
                "statuses must contain distinct lifecycle states",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let cursor = request
            .page_token
            .as_deref()
            .map(|value| decode_list_cursor(caller, request, value))
            .transpose()?;
        let index_stream = Self::run_index_stream(caller);
        if let Some(cursor) = cursor {
            validate_cursor_index_version(
                self.journal.as_ref(),
                caller,
                &index_stream,
                cursor.snapshot_version,
            )?;
            if cursor.before_version != cursor.snapshot_version {
                validate_cursor_index_version(
                    self.journal.as_ref(),
                    caller,
                    &index_stream,
                    cursor.before_version,
                )?;
            }
        }

        let page_size = request.bounded_page_size().min(MAX_PAGE_SIZE);
        let target_count = page_size.saturating_add(1);
        let mut before_version = cursor.map(|cursor| cursor.before_version);
        let mut snapshot_version = cursor.map(|cursor| cursor.snapshot_version);
        let mut scanned = 0_usize;
        let mut reconstructed_events = 0_usize;
        let mut last_processed_version = None;
        let mut index_has_more = false;
        let mut runs = Vec::with_capacity(target_count);
        let mut archived_threads = BTreeMap::<String, bool>::new();

        'read_index: loop {
            let remaining = MAX_LIST_INDEX_EVENTS_SCANNED.saturating_sub(scanned);
            if remaining == 0 {
                index_has_more = !self
                    .journal
                    .read_stream_backwards(&index_stream, before_version, 1)
                    .map_err(|error| ApiError::from_store(&error, caller.request_id()))?
                    .is_empty();
                break;
            }
            let read_limit = LIST_INDEX_READ_BATCH.min(remaining);
            let index_events = self
                .journal
                .read_stream_backwards(&index_stream, before_version, read_limit)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
            if index_events.is_empty() {
                break;
            }
            if snapshot_version.is_none() {
                snapshot_version = index_events.first().map(|event| event.stream_version);
            }
            for index_event in &index_events {
                if index_event.stream_version > snapshot_version.unwrap_or(u64::MAX)
                    || before_version.map_or_else(
                        || Some(index_event.stream_version) != snapshot_version,
                        |version| index_event.stream_version.checked_add(1) != Some(version),
                    )
                {
                    return Err(invariant(
                        caller,
                        "the durable run index sequence could not be verified",
                    ));
                }
                let indexed =
                    decode_indexed(self.journal.as_ref(), caller, &index_stream, index_event)?;
                let run_events = self.read_bounded_run_stream(caller, &indexed.run_id)?;
                if reconstructed_events != 0
                    && reconstructed_events.saturating_add(run_events.len())
                        > MAX_LIST_RUN_EVENTS_RECONSTRUCTED
                {
                    index_has_more = true;
                    break 'read_index;
                }
                reconstructed_events = reconstructed_events.saturating_add(run_events.len());
                let mut run =
                    reconstruct(self.journal.as_ref(), caller, &indexed.run_id, &run_events)?;
                let created = run_events.first().ok_or_else(|| {
                    invariant(caller, "the durable run index references an absent run")
                })?;
                if run.id != indexed.run_id
                    || index_event.context.session_id.as_deref() != Some(run.session_id.as_str())
                    || index_event.global_sequence <= created.global_sequence
                {
                    return Err(invariant(
                        caller,
                        "the durable run index entry could not be verified",
                    ));
                }

                run.archived = if let Some(archived) = archived_threads.get(&run.session_id) {
                    *archived
                } else {
                    let archived = self.thread_state(caller, &run.session_id)?.0.archived;
                    archived_threads.insert(run.session_id.clone(), archived);
                    archived
                };

                scanned = scanned.saturating_add(1);
                before_version = Some(index_event.stream_version);
                last_processed_version = Some(index_event.stream_version);
                if request
                    .session_id
                    .as_ref()
                    .is_none_or(|session_id| &run.session_id == session_id)
                    && (request.statuses.is_empty() || request.statuses.contains(&run.status))
                    && (request.include_archived || !run.archived)
                {
                    runs.push((run, index_event.stream_version));
                    if runs.len() >= target_count {
                        index_has_more = true;
                        break 'read_index;
                    }
                }
            }
            if index_events.len() < read_limit {
                break;
            }
        }

        let next_before_version = if runs.len() > page_size {
            runs.get(page_size.saturating_sub(1))
                .map(|(_, version)| *version)
        } else if index_has_more {
            last_processed_version
        } else {
            None
        };
        let mut page = runs
            .into_iter()
            .take(page_size)
            .map(|(run, _)| run)
            .collect::<Vec<_>>();
        let next_page_token =
            next_before_version
                .zip(snapshot_version)
                .map(|(before_version, snapshot_version)| {
                    encode_list_cursor(
                        caller,
                        request,
                        RunListCursor {
                            snapshot_version,
                            before_version,
                        },
                    )
                });
        page.shrink_to_fit();
        Ok(ListRunsResponse {
            runs: page,
            next_page_token,
        })
    }

    fn archive_thread(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
    ) -> ApiResult<ThreadLifecycle> {
        self.set_thread_archived(caller, run_id, idempotency_key, true)
    }

    fn restore_thread(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
    ) -> ApiResult<ThreadLifecycle> {
        self.set_thread_archived(caller, run_id, idempotency_key, false)
    }

    fn append_update(
        &self,
        caller: &CallerContext,
        run_id: &str,
        expected_sequence: u64,
        kind: RunUpdateKind,
    ) -> ApiResult<RunUpdate> {
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let (run, released_bytes) = self.load_append_state(caller, run_id)?.ok_or_else(|| {
            ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested run was not found",
            )
            .with_correlation_id(caller.request_id().clone())
        })?;
        if run.last_sequence != expected_sequence {
            return Err(ApiError::conflict(
                ApiErrorReason::ConcurrentModification,
                "the run changed concurrently",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        if !update_is_terminal(&kind) {
            let next_bytes = released_update_bytes(caller, &kind)?;
            let released_limit = u64::try_from(MAX_RELEASED_BYTES_PER_RUN).unwrap_or(u64::MAX);
            if expected_sequence >= MAX_NONTERMINAL_RUN_SEQUENCE
                || released_bytes.saturating_add(next_bytes) > released_limit
            {
                return Err(ApiError::bounded_resource_exhausted(
                    ApiErrorReason::CapacityExceeded,
                    "the run reached its released update budget",
                )
                .with_correlation_id(caller.request_id().clone()));
            }
        }
        self.append_unchecked(caller, &run, released_bytes, kind)
    }

    fn request_cancellation(
        &self,
        caller: &CallerContext,
        run_id: &str,
        idempotency_key: &crate::IdempotencyKey,
    ) -> ApiResult<Idempotent<Run>> {
        caller.require_scope(scopes::RUNS_CONTROL)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let fingerprint = hex::encode(Sha256::digest(
            [b"colossus-api-cancel-v1\0".as_slice(), run_id.as_bytes()].concat(),
        ));
        let idempotency_stream =
            Self::idempotency_stream(caller, CANCEL_OPERATION, idempotency_key);
        if let Some(replay) =
            self.replay(caller, &idempotency_stream, CANCEL_OPERATION, &fingerprint)?
        {
            return Ok(replay);
        }
        let (run, released_bytes) = self.load_append_state(caller, run_id)?.ok_or_else(|| {
            ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested run was not found",
            )
            .with_correlation_id(caller.request_id().clone())
        })?;
        let context = ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            session_id: Some(run.session_id.clone()),
            run_id: Some(run.id.clone()),
            ..ExecutionContext::default()
        };
        let actor = caller.actor();
        let claim = NewEvent {
            event_version: 1,
            stream_id: idempotency_stream.clone(),
            expected_stream_version: 0,
            classification: EventClassification::System,
            event_type: IDEMPOTENCY_EVENT.into(),
            actor: actor.clone(),
            context: context.clone(),
            payload: json!({
                "operation": CANCEL_OPERATION,
                "request_fingerprint": fingerprint,
                "run_id": run.id,
            }),
        };
        let mut events = vec![claim];
        let mut run_updates = Vec::new();
        let mut validated = run.clone();
        if let Some(mut interaction) = run.pending_interaction.clone() {
            interaction.status = InteractionStatus::Cancelled;
            let kind = RunUpdateKind::Interaction { interaction };
            validate_update(&validated, &kind)
                .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
            let next_sequence = validated.last_sequence.saturating_add(1);
            let occurred_at = validated.updated_at.clone();
            apply_update(&mut validated, next_sequence, &occurred_at, &kind)
                .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
            run_updates.push(kind);
        }
        if !run.status.is_terminal() && run.status != RunStatus::Cancelling {
            let kind = RunUpdateKind::State {
                status: RunStatus::Cancelling,
            };
            validate_update(&validated, &kind)
                .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
            run_updates.push(kind);
        }
        let mut projected = run.clone();
        let mut projected_released_bytes = released_bytes;
        for kind in &run_updates {
            let expected_stream_version = projected.last_sequence;
            events.push(NewEvent {
                event_version: 1,
                stream_id: Self::run_stream(&run.id),
                expected_stream_version,
                classification: EventClassification::Domain,
                event_type: RUN_UPDATE_EVENT.into(),
                actor: actor.clone(),
                context: context.clone(),
                payload: stored_update_payload(
                    caller,
                    &projected,
                    projected_released_bytes,
                    kind.clone(),
                )?,
            });
            projected_released_bytes = projected_released_bytes
                .checked_add(released_update_bytes(caller, kind)?)
                .ok_or_else(|| invariant(caller, "the released run byte count overflowed"))?;
            let next_sequence = expected_stream_version.saturating_add(1);
            let occurred_at = projected.updated_at.clone();
            apply_update(&mut projected, next_sequence, &occurred_at, kind)
                .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        }
        let append = self.journal.append_batch(events);
        let envelopes = match append {
            Ok(envelopes) => envelopes,
            Err(StoreError::Conflict { stream_id, .. }) if stream_id == idempotency_stream => {
                return self
                    .replay(caller, &idempotency_stream, CANCEL_OPERATION, &fingerprint)?
                    .ok_or_else(|| {
                        invariant(caller, "the concurrent cancellation claim disappeared")
                    });
            }
            Err(error) => return Err(ApiError::from_store(&error, caller.request_id())),
        };
        let mut current = run;
        for (envelope, kind) in envelopes.iter().skip(1).zip(run_updates.iter()) {
            apply_update(
                &mut current,
                envelope.stream_version,
                &envelope.occurred_at,
                kind,
            )
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        }
        Ok(Idempotent {
            value: current,
            replayed: false,
        })
    }

    fn updates_after(
        &self,
        caller: &CallerContext,
        run_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> ApiResult<Vec<RunUpdate>> {
        caller.require_scope(scopes::RUNS_READ)?;
        token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let stream_id = Self::run_stream(run_id);
        let owner_event = self
            .journal
            .read_stream_from(&stream_id, 0, 1)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        if owner_event.is_empty() {
            return Err(ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested run was not found",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        if !visible_to(caller, &owner_event)? {
            return Err(ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested run was not found",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let events = self
            .journal
            .read_stream_from(
                &stream_id,
                after_sequence,
                limit.clamp(1, MAX_UPDATE_PAGE_SIZE),
            )
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        events
            .iter()
            .map(|event| update_from_event(self.journal.as_ref(), caller, run_id, event))
            .collect()
    }

    fn respond_interaction(
        &self,
        caller: &CallerContext,
        run_id: &str,
        interaction_id: &str,
        etag: &str,
        idempotency_key: &crate::IdempotencyKey,
        response: InteractionResponse,
    ) -> ApiResult<Idempotent<Interaction>> {
        let fingerprint =
            interaction_response_fingerprint(caller, run_id, interaction_id, etag, &response)?;
        let idempotency_stream =
            Self::idempotency_stream(caller, RESPOND_OPERATION, idempotency_key);
        if let Some(replay) =
            self.replay_interaction(caller, &idempotency_stream, &fingerprint, interaction_id)?
        {
            return Ok(replay);
        }
        let (run, released_bytes) = self.load_append_state(caller, run_id)?.ok_or_else(|| {
            ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "the requested run was not found",
            )
            .with_correlation_id(caller.request_id().clone())
        })?;
        if run.etag != etag {
            return Err(ApiError::conflict(
                ApiErrorReason::ConcurrentModification,
                "the run changed concurrently",
            )
            .with_correlation_id(caller.request_id().clone()));
        }
        let interaction = run
            .pending_interaction
            .as_ref()
            .filter(|interaction| interaction.id == interaction_id)
            .ok_or_else(|| unavailable_interaction(caller))?;
        if run.status != RunStatus::Waiting
            || interaction.status != InteractionStatus::Pending
            || interaction.application_id != caller.principal().application_id()
        {
            return Err(unavailable_interaction(caller));
        }
        require_interaction_response_scope(caller, interaction.kind)?;
        let expiry = OffsetDateTime::parse(&interaction.expires_at, &Rfc3339)
            .map_err(|_| invariant(caller, "the interaction expiry could not be verified"))?;
        if expiry <= OffsetDateTime::now_utc() {
            return Err(unavailable_interaction(caller));
        }
        validate_response(interaction, &response)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let mut resolved = interaction.clone();
        resolved.status = InteractionStatus::Responded;
        resolved.response = Some(response);
        resolved.responded_at = Some(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| invariant(caller, "the interaction time could not be encoded"))?,
        );
        let kind = RunUpdateKind::Interaction {
            interaction: resolved.clone(),
        };
        validate_update(&run, &kind)
            .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
        let context = ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            session_id: Some(run.session_id.clone()),
            run_id: Some(run.id.clone()),
            ..ExecutionContext::default()
        };
        let actor = caller.actor();
        let append = self.journal.append_batch(vec![
            NewEvent {
                event_version: 1,
                stream_id: idempotency_stream.clone(),
                expected_stream_version: 0,
                classification: EventClassification::System,
                event_type: IDEMPOTENCY_EVENT.into(),
                actor: actor.clone(),
                context: context.clone(),
                payload: json!({
                    "operation": RESPOND_OPERATION,
                    "request_fingerprint": fingerprint,
                    "run_id": run.id,
                }),
            },
            NewEvent {
                event_version: 1,
                stream_id: Self::run_stream(&run.id),
                expected_stream_version: run.last_sequence,
                classification: EventClassification::Domain,
                event_type: RUN_UPDATE_EVENT.into(),
                actor,
                context,
                payload: stored_update_payload(caller, &run, released_bytes, kind)?,
            },
        ]);
        match append {
            Ok(_) => Ok(Idempotent {
                value: resolved,
                replayed: false,
            }),
            Err(StoreError::Conflict { stream_id, .. }) if stream_id == idempotency_stream => self
                .replay_interaction(caller, &idempotency_stream, &fingerprint, interaction_id)?
                .ok_or_else(|| {
                    invariant(
                        caller,
                        "the concurrent interaction idempotency claim disappeared",
                    )
                }),
            Err(error) => Err(ApiError::from_store(&error, caller.request_id())),
        }
    }

    fn resolve_interaction_response(
        &self,
        caller: &CallerContext,
        run_id: &str,
        interaction_id: &str,
        etag: &str,
        idempotency_key: &crate::IdempotencyKey,
        response: &InteractionResponse,
    ) -> ApiResult<Option<Interaction>> {
        let fingerprint =
            interaction_response_fingerprint(caller, run_id, interaction_id, etag, response)?;
        let idempotency_stream =
            Self::idempotency_stream(caller, RESPOND_OPERATION, idempotency_key);
        self.replay_interaction(caller, &idempotency_stream, &fingerprint, interaction_id)
            .map(|replay| replay.map(|replay| replay.value))
    }
}

fn initial_run(envelope: &EventEnvelope, new_run: &NewRun, request: &CreateRunRequest) -> Run {
    Run {
        id: new_run.id().into(),
        session_id: new_run.session_id().into(),
        title: request.display_title(),
        status: RunStatus::Queued,
        mode: request.mode,
        role: new_run.role().into(),
        skill_ids: request.skill_ids.clone(),
        created_at: envelope.occurred_at.clone(),
        updated_at: envelope.occurred_at.clone(),
        started_at: None,
        finished_at: None,
        last_sequence: envelope.stream_version,
        result: None,
        failure: None,
        cancellation: None,
        pending_interaction: None,
        etag: run_etag(new_run.id(), envelope.stream_version),
        archived: false,
    }
}

fn initial_run_from_created(envelope: &EventEnvelope, created: &RunCreated) -> Run {
    Run {
        id: created.id.clone(),
        session_id: created.session_id.clone(),
        title: created.execution.request.display_title(),
        status: RunStatus::Queued,
        mode: created.mode,
        role: created.role.clone(),
        skill_ids: created.skill_ids.clone(),
        created_at: envelope.occurred_at.clone(),
        updated_at: envelope.occurred_at.clone(),
        started_at: None,
        finished_at: None,
        last_sequence: envelope.stream_version,
        result: None,
        failure: None,
        cancellation: None,
        pending_interaction: None,
        etag: run_etag(&created.id, envelope.stream_version),
        archived: false,
    }
}

fn stored_update_payload(
    caller: &CallerContext,
    prior: &Run,
    released_bytes_before: u64,
    kind: RunUpdateKind,
) -> ApiResult<serde_json::Value> {
    serde_json::to_value(StoredUpdate {
        format_version: STORED_UPDATE_FORMAT_VERSION,
        prior_state: StoredRunState::capture(prior),
        released_bytes_before,
        kind,
    })
    .map_err(|_| invariant(caller, "the run update could not be encoded"))
}

#[cfg(test)]
pub(crate) fn stored_update_payload_for_test(
    caller: &CallerContext,
    prior: &Run,
    earlier_kinds: &[RunUpdateKind],
    kind: RunUpdateKind,
) -> ApiResult<serde_json::Value> {
    let released_bytes_before = earlier_kinds.iter().try_fold(0_u64, |total, earlier| {
        total
            .checked_add(released_update_bytes(caller, earlier)?)
            .ok_or_else(|| invariant(caller, "the released run byte count overflowed"))
    })?;
    stored_update_payload(caller, prior, released_bytes_before, kind)
}

#[cfg(test)]
pub(crate) fn replay_preview_stored_update_for_test(
    initial: &Run,
    sequence: u64,
    occurred_at: &str,
    payload: serde_json::Value,
) -> ApiResult<Run> {
    let stored: StoredUpdate = serde_json::from_value(payload)
        .map_err(|_| ApiError::internal("a durable run update is invalid"))?;
    let mut run = stored
        .prior_state
        .restore(initial, sequence.saturating_sub(1));
    apply_update(&mut run, sequence, occurred_at, &stored.kind)?;
    Ok(run)
}

fn released_update_bytes(caller: &CallerContext, kind: &RunUpdateKind) -> ApiResult<u64> {
    let encoded = serde_json::to_vec(&ReleasedUpdate { kind })
        .map_err(|_| invariant(caller, "the run update could not be encoded"))?;
    u64::try_from(encoded.len())
        .map_err(|_| invariant(caller, "the released run byte count overflowed"))
}

fn decode_stored_update(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    event: &EventEnvelope,
) -> ApiResult<StoredUpdate> {
    journal
        .decrypt_payload(event)
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))
        .and_then(|payload| {
            serde_json::from_value(payload)
                .map_err(|_| invariant(caller, "a durable run update is invalid"))
        })
}

fn validated_stored_update(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    run_id: &str,
    first: &EventEnvelope,
    event: &EventEnvelope,
) -> ApiResult<StoredUpdate> {
    validate_update_envelope(caller, run_id, first, event)?;
    let stored = decode_stored_update(journal, caller, event)?;
    if stored.format_version != STORED_UPDATE_FORMAT_VERSION {
        return Err(invariant(
            caller,
            "the durable run update format is unsupported",
        ));
    }
    Ok(stored)
}

fn validate_update_envelope(
    caller: &CallerContext,
    run_id: &str,
    first: &EventEnvelope,
    event: &EventEnvelope,
) -> ApiResult<()> {
    if event.event_version != 1
        || event.stream_id != EventSourcedRunRepository::run_stream(run_id)
        || event.classification != EventClassification::Domain
        || event.event_type != RUN_UPDATE_EVENT
        || event.stream_version < 2
        || event.context.run_id.as_deref() != Some(run_id)
        || event.context.session_id.as_deref() != first.context.session_id.as_deref()
        || event.actor.actor_type != ActorType::Application
        || event.actor.id != first.actor.id
    {
        return Err(invariant(
            caller,
            "the durable run update envelope could not be verified",
        ));
    }
    Ok(())
}

fn validate_stored_state(caller: &CallerContext, run: &Run) -> ApiResult<()> {
    let terminal_payload_is_valid = match run.status {
        RunStatus::Completed => {
            run.result.is_some() && run.failure.is_none() && run.cancellation.is_none()
        }
        RunStatus::Failed | RunStatus::Interrupted | RunStatus::OutcomeUnknown => {
            run.result.is_none() && run.failure.is_some() && run.cancellation.is_none()
        }
        RunStatus::Cancelled => {
            run.result.is_none() && run.failure.is_none() && run.cancellation.is_some()
        }
        RunStatus::Queued | RunStatus::Running | RunStatus::Waiting | RunStatus::Cancelling => {
            run.result.is_none() && run.failure.is_none() && run.cancellation.is_none()
        }
    };
    let terminal_time_is_valid = run.status.is_terminal() == run.finished_at.is_some();
    let pending_interaction_is_valid = run.pending_interaction.as_ref().is_none_or(|interaction| {
        run.status == RunStatus::Waiting && interaction.status == InteractionStatus::Pending
    });
    let timestamps_are_valid = [&run.created_at, &run.updated_at]
        .into_iter()
        .all(|timestamp| OffsetDateTime::parse(timestamp, &Rfc3339).is_ok())
        && [&run.started_at, &run.finished_at]
            .into_iter()
            .flatten()
            .all(|timestamp| OffsetDateTime::parse(timestamp, &Rfc3339).is_ok());
    if !terminal_payload_is_valid
        || !terminal_time_is_valid
        || !pending_interaction_is_valid
        || !timestamps_are_valid
    {
        return Err(invariant(
            caller,
            "the durable run tail state could not be verified",
        ));
    }
    Ok(())
}

fn interaction_response_fingerprint(
    caller: &CallerContext,
    run_id: &str,
    interaction_id: &str,
    etag: &str,
    response: &InteractionResponse,
) -> ApiResult<String> {
    token(run_id, "run_id", MAX_IDENTIFIER_BYTES)
        .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
    token(interaction_id, "interaction_id", MAX_IDENTIFIER_BYTES)
        .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
    let response_bytes = serde_json::to_vec(response)
        .map_err(|_| invariant(caller, "the interaction response could not be normalized"))?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"colossus-api-interaction-response-v1\0");
    fingerprint.update(run_id.as_bytes());
    fingerprint.update(b"\0");
    fingerprint.update(interaction_id.as_bytes());
    fingerprint.update(b"\0");
    fingerprint.update(etag.as_bytes());
    fingerprint.update(b"\0");
    fingerprint.update(response_bytes);
    Ok(hex::encode(fingerprint.finalize()))
}

fn reconstruct(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    run_id: &str,
    events: &[EventEnvelope],
) -> ApiResult<Run> {
    let first = events
        .first()
        .filter(|event| event.event_type == RUN_CREATED_EVENT && event.stream_version == 1)
        .ok_or_else(|| invariant(caller, "the durable run creation event is invalid"))?;
    if first.actor.actor_type != ActorType::Application
        || first.actor.id != caller.principal().application_id()
    {
        return Err(invariant(
            caller,
            "the durable run owner could not be verified",
        ));
    }
    let created = decode_created(journal, caller, run_id, first)?;
    let mut run = initial_run_from_created(first, &created);
    let mut released_bytes = 0_u64;
    for event in events.iter().skip(1) {
        if event.stream_version != run.last_sequence.saturating_add(1) {
            return Err(invariant(
                caller,
                "the durable run update sequence could not be verified",
            ));
        }
        validate_update_envelope(caller, run_id, first, event)?;
        let stored = decode_stored_update(journal, caller, event)?;
        if stored.format_version != STORED_UPDATE_FORMAT_VERSION
            || stored.prior_state != StoredRunState::capture(&run)
            || stored.released_bytes_before != released_bytes
        {
            return Err(invariant(
                caller,
                "the durable run tail projection could not be verified",
            ));
        }
        validate_update_owner(caller, &stored.kind)?;
        released_bytes = released_bytes
            .checked_add(released_update_bytes(caller, &stored.kind)?)
            .ok_or_else(|| invariant(caller, "the released run byte count overflowed"))?;
        apply_update(
            &mut run,
            event.stream_version,
            &event.occurred_at,
            &stored.kind,
        )
        .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
    }
    Ok(run)
}

fn decode_created(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    run_id: &str,
    first: &EventEnvelope,
) -> ApiResult<RunCreated> {
    let created: RunCreated = journal
        .decrypt_payload(first)
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))
        .and_then(|payload| {
            serde_json::from_value(payload)
                .map_err(|_| invariant(caller, "the durable run creation payload is invalid"))
        })?;
    created
        .execution
        .request
        .validate()
        .map_err(|error| error.with_correlation_id(caller.request_id().clone()))?;
    let execution = &created.execution;
    let request = &execution.request;
    let has_execute_scope = execution
        .scopes
        .iter()
        .any(|scope| scope.as_str() == scopes::RUNS_EXECUTE);
    let role_allowed = execution
        .allowed_roles
        .iter()
        .any(|role| role == &created.role);
    let invalid_grant_snapshot = execution.scopes.len() > 512
        || execution.allowed_roles.len() > 512
        || execution.allowed_tools.len() > 512
        || execution.scopes.windows(2).any(|pair| pair[0] >= pair[1])
        || execution
            .allowed_roles
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || execution
            .allowed_tools
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || token(
            &execution.application_id,
            "application_id",
            MAX_IDENTIFIER_BYTES,
        )
        .is_err()
        || execution
            .allowed_roles
            .iter()
            .any(|role| token(role, "allowed_role", MAX_ROLE_BYTES).is_err())
        || execution
            .allowed_tools
            .iter()
            .any(|tool| token(tool, "allowed_tool", MAX_TOOL_BYTES).is_err())
        || execution.request.validate().is_err()
        || !execution.request.skill_ids.is_empty();
    if created.id != run_id
        || first.context.run_id.as_deref() != Some(run_id)
        || first.context.session_id.as_deref() != Some(created.session_id.as_str())
        || execution.application_id != first.actor.id
        || !has_execute_scope
        || !role_allowed
        || invalid_grant_snapshot
        || request
            .session_id
            .as_ref()
            .is_some_and(|session_id| session_id != &created.session_id)
        || request
            .role
            .as_ref()
            .is_some_and(|role| role != &created.role)
        || request.mode != created.mode
        || request.skill_ids != created.skill_ids
    {
        return Err(invariant(
            caller,
            "the durable run execution request could not be verified",
        ));
    }
    Ok(created)
}

fn update_from_event(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    run_id: &str,
    event: &EventEnvelope,
) -> ApiResult<RunUpdate> {
    if event.actor.actor_type != ActorType::Application
        || event.actor.id != caller.principal().application_id()
    {
        return Err(invariant(
            caller,
            "the durable run update owner could not be verified",
        ));
    }
    let kind = if event.event_type == RUN_CREATED_EVENT && event.stream_version == 1 {
        RunUpdateKind::State {
            status: RunStatus::Queued,
        }
    } else if event.event_type == RUN_UPDATE_EVENT {
        let stored = decode_stored_update(journal, caller, event)?;
        if stored.format_version != STORED_UPDATE_FORMAT_VERSION {
            return Err(invariant(
                caller,
                "the durable run update format is unsupported",
            ));
        }
        stored.kind
    } else {
        return Err(invariant(
            caller,
            "the durable run feed contains an unknown event",
        ));
    };
    validate_update_owner(caller, &kind)?;
    Ok(RunUpdate {
        run_id: run_id.into(),
        sequence: event.stream_version,
        occurred_at: event.occurred_at.clone(),
        kind,
    })
}

fn validate_update(run: &Run, kind: &RunUpdateKind) -> ApiResult<()> {
    if run.status.is_terminal() {
        return Err(ApiError::failed_precondition(
            ApiErrorReason::InvalidRunTransition,
            "terminal runs cannot accept additional updates",
        ));
    }
    if let Some(pending) = &run.pending_interaction {
        let resolves_pending = matches!(
            kind,
            RunUpdateKind::Interaction { interaction }
                if interaction.id == pending.id
                    && interaction.status != InteractionStatus::Pending
        ) || matches!(
            kind,
            RunUpdateKind::Failure { .. } | RunUpdateKind::Cancellation { .. }
        );
        if !resolves_pending {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::InvalidRunTransition,
                "a pending interaction must be resolved before other run updates",
            ));
        }
    }
    match kind {
        RunUpdateKind::State { status } => {
            if status.is_terminal() || !run.status.permits(*status) {
                return Err(invalid_transition());
            }
        }
        RunUpdateKind::OutputDelta { text } => {
            bounded_text(text, "update.output_delta.text", MAX_INPUT_BYTES, true)?;
        }
        RunUpdateKind::ReasoningSummary { summary } => {
            bounded_text(summary, "update.reasoning_summary.summary", 65_536, true)?;
        }
        RunUpdateKind::ToolActivity { activity } => {
            token(
                &activity.call_id,
                "update.tool_activity.call_id",
                MAX_IDENTIFIER_BYTES,
            )?;
            token(
                &activity.tool_name,
                "update.tool_activity.tool_name",
                MAX_TOOL_BYTES,
            )?;
            bounded_text(
                &activity.summary,
                "update.tool_activity.summary",
                65_536,
                true,
            )?;
            if let Some(input) = &activity.input {
                bounded_text(input, "update.tool_activity.input", 65_536, false)?;
            }
            if let Some(preview) = &activity.preview {
                bounded_text(preview, "update.tool_activity.preview", 65_536, false)?;
            }
        }
        RunUpdateKind::Usage { usage } => {
            if usage
                .input_tokens
                .checked_add(usage.output_tokens)
                .is_none()
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "update.usage",
                    "token accounting exceeds the supported range",
                ));
            }
        }
        RunUpdateKind::Interaction { interaction } => validate_interaction(run, interaction)?,
        RunUpdateKind::Message { message } => validate_released_message(message)?,
        RunUpdateKind::Notice { notice } => {
            token(&notice.reason, "update.notice.reason", MAX_IDENTIFIER_BYTES)?;
            bounded_text(&notice.message, "update.notice.message", 65_536, true)?;
        }
        RunUpdateKind::Result { result } => {
            if !run.status.permits(RunStatus::Completed) {
                return Err(invalid_transition());
            }
            bounded_text(&result.output, "result.output", MAX_INPUT_BYTES, true)?;
            token(&result.profile, "result.profile", MAX_IDENTIFIER_BYTES)?;
            token(&result.model, "result.model", MAX_IDENTIFIER_BYTES)?;
            if !result.elapsed_seconds.is_finite() || result.elapsed_seconds < 0.0 {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "result.elapsed_seconds",
                    "elapsed_seconds must be finite and non-negative",
                ));
            }
        }
        RunUpdateKind::Failure { status, failure } => {
            if !matches!(
                status,
                RunStatus::Failed | RunStatus::Interrupted | RunStatus::OutcomeUnknown
            ) || !run.status.permits(*status)
            {
                return Err(invalid_transition());
            }
            token(&failure.code, "failure.code", MAX_IDENTIFIER_BYTES)?;
            bounded_text(&failure.message, "failure.message", 4_096, false)?;
            if *status == RunStatus::OutcomeUnknown
                && failure.outcome != crate::OutcomeCertainty::Unknown
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "failure.outcome",
                    "an outcome-unknown failure must declare unknown outcome certainty",
                ));
            }
            if *status != RunStatus::OutcomeUnknown
                && failure.outcome != crate::OutcomeCertainty::Known
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "failure.outcome",
                    "known terminal failures must declare known outcome certainty",
                ));
            }
        }
        RunUpdateKind::Cancellation { cancellation } => {
            if !run.status.permits(RunStatus::Cancelled) {
                return Err(invalid_transition());
            }
            bounded_text(&cancellation.message, "cancellation.message", 4_096, false)?;
        }
    }
    Ok(())
}

fn update_is_terminal(kind: &RunUpdateKind) -> bool {
    matches!(
        kind,
        RunUpdateKind::Result { .. }
            | RunUpdateKind::Failure { .. }
            | RunUpdateKind::Cancellation { .. }
    )
}

fn validate_interaction(run: &Run, interaction: &Interaction) -> ApiResult<()> {
    token(&interaction.id, "interaction.id", MAX_IDENTIFIER_BYTES)?;
    token(
        &interaction.application_id,
        "interaction.application_id",
        MAX_IDENTIFIER_BYTES,
    )?;
    bounded_text(&interaction.prompt, "interaction.prompt", 65_536, false)?;
    let created_at = OffsetDateTime::parse(&interaction.created_at, &Rfc3339).map_err(|_| {
        ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "interaction.created_at",
            "interaction creation time must be UTC RFC3339",
        )
    })?;
    if interaction.choices.len() > 64 {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "interaction.choices",
            "interactions support at most 64 choices",
        ));
    }
    for choice in &interaction.choices {
        bounded_text(choice, "interaction.choices", 4_096, false)?;
    }
    let expires_at = OffsetDateTime::parse(&interaction.expires_at, &Rfc3339).map_err(|_| {
        ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "interaction.expires_at",
            "interaction expiry must be UTC RFC3339",
        )
    })?;
    if expires_at <= created_at {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "interaction.expires_at",
            "interaction expiry must follow its creation time",
        ));
    }
    match interaction.status {
        InteractionStatus::Pending => {
            if !matches!(run.status, RunStatus::Running | RunStatus::Waiting)
                || run.pending_interaction.is_some()
                || interaction.response.is_some()
                || interaction.responded_at.is_some()
            {
                return Err(invalid_transition());
            }
        }
        InteractionStatus::Responded => {
            if run.pending_interaction.as_ref().is_none_or(|pending| {
                pending.id != interaction.id || !same_interaction_challenge(pending, interaction)
            }) || interaction.response.is_none()
                || interaction.responded_at.is_none()
            {
                return Err(invalid_transition());
            }
            let response = interaction
                .response
                .as_ref()
                .ok_or_else(invalid_transition)?;
            validate_response(interaction, response)?;
            OffsetDateTime::parse(
                interaction
                    .responded_at
                    .as_deref()
                    .ok_or_else(invalid_transition)?,
                &Rfc3339,
            )
            .map_err(|_| {
                ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.responded_at",
                    "interaction response time must be UTC RFC3339",
                )
            })?;
        }
        InteractionStatus::Expired | InteractionStatus::Cancelled => {
            if run.pending_interaction.as_ref().is_none_or(|pending| {
                pending.id != interaction.id || !same_interaction_challenge(pending, interaction)
            }) || interaction.response.is_some()
                || interaction.responded_at.is_some()
            {
                return Err(invalid_transition());
            }
        }
    }
    match interaction.kind {
        InteractionKind::Prompt => {
            if interaction.request_hash.is_some()
                || interaction.action.is_some()
                || interaction.resource.is_some()
                || interaction.risk.is_some()
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction",
                    "ordinary prompts do not accept approval metadata",
                ));
            }
            if interaction.choices.is_empty() && !interaction.allow_free_form {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.choices",
                    "a non-free-form prompt requires at least one choice",
                ));
            }
        }
        InteractionKind::Approval => {
            if !interaction.choices.is_empty() || interaction.allow_free_form {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.choices",
                    "approval interactions do not accept prompt choices or free-form input",
                ));
            }
            let hash = interaction.request_hash.as_deref().ok_or_else(|| {
                ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.request_hash",
                    "approval interactions require an exact one-use binding",
                )
            })?;
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.request_hash",
                    "approval binding must be a lowercase 256-bit hexadecimal value",
                ));
            }
            let action = interaction.action.as_deref().ok_or_else(|| {
                ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.action",
                    "approval interactions require a fixed public action category",
                )
            })?;
            let resource = interaction.resource.as_deref().ok_or_else(|| {
                ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "interaction.resource",
                    "approval interactions require a sanitized public resource",
                )
            })?;
            validate_public_approval_display(action, resource)?;
        }
    }
    Ok(())
}

fn validate_released_message(message: &crate::ReleasedSessionMessage) -> ApiResult<()> {
    token(
        &message.session_id,
        "update.message.session_id",
        MAX_IDENTIFIER_BYTES,
    )?;
    token(
        &message.run_id,
        "update.message.run_id",
        MAX_IDENTIFIER_BYTES,
    )?;
    if message.sequence == 0 {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "update.message.sequence",
            "message sequence must be non-zero",
        ));
    }
    OffsetDateTime::parse(&message.created_at, &Rfc3339).map_err(|_| {
        ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "update.message.created_at",
            "message creation time must be UTC RFC3339",
        )
    })?;
    if message.content.is_empty() || message.content.len() > 128 {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "update.message.content",
            "message content must contain between 1 and 128 parts",
        ));
    }
    for part in &message.content {
        match part {
            crate::ReleasedContentPart::Text { text } => {
                bounded_text(text, "update.message.content.text", MAX_INPUT_BYTES, true)?;
            }
            crate::ReleasedContentPart::Artifact { artifact } => {
                token(
                    &artifact.artifact_id,
                    "update.message.content.artifact.artifact_id",
                    MAX_IDENTIFIER_BYTES,
                )?;
                bounded_text(
                    &artifact.file_name,
                    "update.message.content.artifact.file_name",
                    4_096,
                    false,
                )?;
                bounded_text(
                    &artifact.media_type,
                    "update.message.content.artifact.media_type",
                    256,
                    false,
                )?;
                if artifact.sha256.len() != 64
                    || !artifact
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "update.message.content.artifact.sha256",
                        "artifact SHA-256 must be a lowercase hexadecimal digest",
                    ));
                }
                OffsetDateTime::parse(&artifact.created_at, &Rfc3339).map_err(|_| {
                    ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "update.message.content.artifact.created_at",
                        "artifact creation time must be UTC RFC3339",
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn same_interaction_challenge(expected: &Interaction, actual: &Interaction) -> bool {
    expected.id == actual.id
        && expected.kind == actual.kind
        && expected.application_id == actual.application_id
        && expected.created_at == actual.created_at
        && expected.prompt == actual.prompt
        && expected.choices == actual.choices
        && expected.allow_free_form == actual.allow_free_form
        && expected.request_hash == actual.request_hash
        && expected.action == actual.action
        && expected.resource == actual.resource
        && expected.risk == actual.risk
        && expected.expires_at == actual.expires_at
}

fn validate_response(interaction: &Interaction, response: &InteractionResponse) -> ApiResult<()> {
    match (interaction.kind, response) {
        (
            InteractionKind::Prompt,
            InteractionResponse::Prompt {
                answer,
                selected_index,
            },
        ) => {
            bounded_text(answer, "response.answer", 65_536, false)?;
            if let Some(index) = selected_index {
                let index = usize::try_from(*index).map_err(|_| {
                    ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "response.selected_index",
                        "selected_index is outside the choice list",
                    )
                })?;
                if interaction
                    .choices
                    .get(index)
                    .is_none_or(|choice| choice != answer)
                {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "response.selected_index",
                        "selected_index and answer do not identify the same choice",
                    ));
                }
            } else if !interaction.allow_free_form {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "response.selected_index",
                    "this prompt requires a listed choice",
                ));
            }
        }
        (InteractionKind::Approval, InteractionResponse::Approval { request_hash, .. })
            if interaction.request_hash.as_deref() == Some(request_hash.as_str()) => {}
        (InteractionKind::Approval, InteractionResponse::Approval { .. }) => {
            return Err(ApiError::permission_denied(
                ApiErrorReason::InteractionUnavailable,
                "the approval response does not match the request shown to the user",
            ));
        }
        _ => {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "response",
                "response type does not match the interaction",
            ));
        }
    }
    Ok(())
}

fn require_interaction_response_scope(
    caller: &CallerContext,
    kind: InteractionKind,
) -> ApiResult<()> {
    match kind {
        InteractionKind::Prompt => caller.require_scope(scopes::PROMPTS_RESPOND),
        InteractionKind::Approval => caller.require_scope(scopes::APPROVALS_RESPOND),
    }
}

fn apply_update(
    run: &mut Run,
    sequence: u64,
    occurred_at: &str,
    kind: &RunUpdateKind,
) -> ApiResult<()> {
    validate_update(run, kind)?;
    match kind {
        RunUpdateKind::State { status } => {
            run.status = *status;
            if *status == RunStatus::Running && run.started_at.is_none() {
                run.started_at = Some(occurred_at.into());
            }
        }
        RunUpdateKind::OutputDelta { .. }
        | RunUpdateKind::ReasoningSummary { .. }
        | RunUpdateKind::ToolActivity { .. }
        | RunUpdateKind::Usage { .. }
        | RunUpdateKind::Message { .. }
        | RunUpdateKind::Notice { .. } => {}
        RunUpdateKind::Interaction { interaction } => match interaction.status {
            InteractionStatus::Pending => {
                run.status = RunStatus::Waiting;
                run.pending_interaction = Some(interaction.clone());
            }
            InteractionStatus::Responded
            | InteractionStatus::Expired
            | InteractionStatus::Cancelled => {
                run.pending_interaction = None;
            }
        },
        RunUpdateKind::Result { result } => {
            run.status = RunStatus::Completed;
            run.result = Some(result.clone());
            run.pending_interaction = None;
            run.finished_at = Some(occurred_at.into());
        }
        RunUpdateKind::Failure { status, failure } => {
            run.status = *status;
            run.failure = Some(failure.clone());
            run.pending_interaction = None;
            run.finished_at = Some(occurred_at.into());
        }
        RunUpdateKind::Cancellation { cancellation } => {
            run.status = RunStatus::Cancelled;
            run.cancellation = Some(cancellation.clone());
            run.pending_interaction = None;
            run.finished_at = Some(occurred_at.into());
        }
    }
    run.last_sequence = sequence;
    run.updated_at = occurred_at.into();
    run.etag = run_etag(&run.id, sequence);
    Ok(())
}

fn decode_indexed(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    index_stream: &str,
    event: &EventEnvelope,
) -> ApiResult<RunIndexed> {
    if event.event_version != 1
        || event.stream_id != index_stream
        || event.stream_version == 0
        || event.classification != EventClassification::System
        || event.event_type != RUN_INDEXED_EVENT
        || event.actor.actor_type != ActorType::Application
        || event.actor.id != caller.principal().application_id()
    {
        return Err(invariant(
            caller,
            "the durable run index envelope could not be verified",
        ));
    }
    let indexed: RunIndexed = journal
        .decrypt_payload(event)
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))
        .and_then(|payload| {
            serde_json::from_value(payload)
                .map_err(|_| invariant(caller, "the durable run index payload is invalid"))
        })?;
    if token(&indexed.run_id, "run_id", MAX_IDENTIFIER_BYTES).is_err()
        || event.context.run_id.as_deref() != Some(indexed.run_id.as_str())
        || event
            .context
            .session_id
            .as_deref()
            .is_none_or(|session_id| token(session_id, "session_id", MAX_IDENTIFIER_BYTES).is_err())
    {
        return Err(invariant(
            caller,
            "the durable run index identity could not be verified",
        ));
    }
    Ok(indexed)
}

fn decode_thread_lifecycle(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    stream_id: &str,
    session_id: &str,
    event: &EventEnvelope,
) -> ApiResult<ThreadLifecycle> {
    if event.event_version != 1
        || event.stream_id != stream_id
        || event.stream_version == 0
        || event.classification != EventClassification::Domain
        || !matches!(
            event.event_type.as_str(),
            THREAD_ATTACHED_EVENT | THREAD_ARCHIVED_EVENT | THREAD_RESTORED_EVENT
        )
        || event.actor.actor_type != ActorType::Application
        || event.actor.id != caller.principal().application_id()
        || event.context.session_id.as_deref() != Some(session_id)
    {
        return Err(invariant(
            caller,
            "the durable thread lifecycle envelope could not be verified",
        ));
    }
    let lifecycle: ThreadLifecycleEvent = journal
        .decrypt_payload(event)
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))
        .and_then(|payload| {
            serde_json::from_value(payload)
                .map_err(|_| invariant(caller, "the durable thread lifecycle is invalid"))
        })?;
    let expected_archived = event.event_type == THREAD_ARCHIVED_EVENT;
    if lifecycle.session_id != session_id
        || token(&lifecycle.session_id, "session_id", MAX_IDENTIFIER_BYTES).is_err()
        || lifecycle.archived != expected_archived
    {
        return Err(invariant(
            caller,
            "the durable thread lifecycle identity could not be verified",
        ));
    }
    Ok(ThreadLifecycle {
        session_id: lifecycle.session_id,
        archived: lifecycle.archived,
    })
}

fn list_query_digest(caller: &CallerContext, request: &ListRunsRequest) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"colossus-api-run-list-query-v1\0");
    let application_id = caller.principal().application_id().as_bytes();
    hasher.update(
        u64::try_from(application_id.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(application_id);
    match request.session_id.as_deref() {
        Some(session_id) => {
            hasher.update([1]);
            hasher.update(
                u64::try_from(session_id.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            hasher.update(session_id.as_bytes());
        }
        None => hasher.update([0]),
    }
    let statuses = request.statuses.iter().copied().collect::<BTreeSet<_>>();
    hasher.update(
        u64::try_from(statuses.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for status in statuses {
        hasher.update([run_status_cursor_byte(status)]);
    }
    hasher.update([u8::from(request.include_archived)]);
    hasher.finalize().into()
}

fn run_status_cursor_byte(status: RunStatus) -> u8 {
    match status {
        RunStatus::Queued => 0,
        RunStatus::Running => 1,
        RunStatus::Waiting => 2,
        RunStatus::Cancelling => 3,
        RunStatus::Completed => 4,
        RunStatus::Failed => 5,
        RunStatus::Cancelled => 6,
        RunStatus::Interrupted => 7,
        RunStatus::OutcomeUnknown => 8,
    }
}

fn list_cursor_binding(
    caller: &CallerContext,
    query_digest: &[u8; 32],
    cursor: RunListCursor,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"colossus-api-run-list-cursor-v1\0");
    hasher.update(caller.principal().application_id().as_bytes());
    hasher.update(query_digest);
    hasher.update(cursor.snapshot_version.to_be_bytes());
    hasher.update(cursor.before_version.to_be_bytes());
    hasher.finalize().into()
}

fn encode_list_cursor(
    caller: &CallerContext,
    request: &ListRunsRequest,
    cursor: RunListCursor,
) -> String {
    let query_digest = list_query_digest(caller, request);
    let binding = list_cursor_binding(caller, &query_digest, cursor);
    let mut bytes = Vec::with_capacity(LIST_CURSOR_BYTES);
    bytes.push(LIST_CURSOR_FORMAT_VERSION);
    bytes.extend_from_slice(&cursor.snapshot_version.to_be_bytes());
    bytes.extend_from_slice(&cursor.before_version.to_be_bytes());
    bytes.extend_from_slice(&query_digest);
    bytes.extend_from_slice(&binding);
    hex::encode(bytes)
}

fn decode_list_cursor(
    caller: &CallerContext,
    request: &ListRunsRequest,
    value: &str,
) -> ApiResult<RunListCursor> {
    if value.len() != LIST_CURSOR_HEX_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_list_cursor(caller));
    }
    let bytes = hex::decode(value).map_err(|_| invalid_list_cursor(caller))?;
    if bytes.len() != LIST_CURSOR_BYTES || bytes[0] != LIST_CURSOR_FORMAT_VERSION {
        return Err(invalid_list_cursor(caller));
    }
    let snapshot_version = u64::from_be_bytes(
        bytes[1..9]
            .try_into()
            .map_err(|_| invalid_list_cursor(caller))?,
    );
    let before_version = u64::from_be_bytes(
        bytes[9..17]
            .try_into()
            .map_err(|_| invalid_list_cursor(caller))?,
    );
    let query_digest: [u8; 32] = bytes[17..49]
        .try_into()
        .map_err(|_| invalid_list_cursor(caller))?;
    let binding: [u8; 32] = bytes[49..81]
        .try_into()
        .map_err(|_| invalid_list_cursor(caller))?;
    let cursor = RunListCursor {
        snapshot_version,
        before_version,
    };
    let expected_query_digest = list_query_digest(caller, request);
    if snapshot_version == 0
        || before_version == 0
        || before_version > snapshot_version
        || query_digest != expected_query_digest
        || binding != list_cursor_binding(caller, &query_digest, cursor)
    {
        return Err(invalid_list_cursor(caller));
    }
    Ok(cursor)
}

fn validate_cursor_index_version(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    index_stream: &str,
    version: u64,
) -> ApiResult<()> {
    let events = journal
        .read_stream_from(index_stream, version.saturating_sub(1), 1)
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
    let Some(event) = events
        .first()
        .filter(|event| event.stream_version == version)
    else {
        return Err(invalid_list_cursor(caller));
    };
    decode_indexed(journal, caller, index_stream, event)?;
    Ok(())
}

fn invalid_list_cursor(caller: &CallerContext) -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "page_token",
        "page_token is not a valid cursor for this run listing",
    )
    .with_correlation_id(caller.request_id().clone())
}

fn run_etag(run_id: &str, sequence: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"colossus-api-run-etag-v1\0");
    hasher.update(run_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(sequence.to_be_bytes());
    hex::encode(hasher.finalize())
}

fn visible_to(caller: &CallerContext, events: &[EventEnvelope]) -> ApiResult<bool> {
    let first = events
        .first()
        .filter(|event| event.event_type == RUN_CREATED_EVENT && event.stream_version == 1)
        .ok_or_else(|| invariant(caller, "the durable run creation event is invalid"))?;
    if first.actor.actor_type != ActorType::Application {
        return Err(invariant(
            caller,
            "the durable run owner type could not be verified",
        ));
    }
    Ok(first.actor.id == caller.principal().application_id())
}

fn validate_update_owner(caller: &CallerContext, kind: &RunUpdateKind) -> ApiResult<()> {
    if let RunUpdateKind::Interaction { interaction } = kind {
        if interaction.application_id != caller.principal().application_id() {
            return Err(invariant(
                caller,
                "the durable interaction owner could not be verified",
            ));
        }
        if interaction.kind == InteractionKind::Approval {
            let action = interaction.action.as_deref().ok_or_else(|| {
                invariant(
                    caller,
                    "the durable approval action category could not be verified",
                )
            })?;
            let resource = interaction.resource.as_deref().ok_or_else(|| {
                invariant(
                    caller,
                    "the durable approval resource category could not be verified",
                )
            })?;
            validate_public_approval_display(action, resource).map_err(|_| {
                invariant(caller, "the durable approval display could not be verified")
            })?;
        }
    }
    Ok(())
}

fn require_run_management_scope(caller: &CallerContext) -> ApiResult<()> {
    if [
        scopes::RUNS_EXECUTE,
        scopes::RUNS_READ,
        scopes::RUNS_CONTROL,
        scopes::PROMPTS_RESPOND,
        scopes::APPROVALS_RESPOND,
    ]
    .into_iter()
    .any(|scope| caller.principal().has_scope(scope))
    {
        Ok(())
    } else {
        caller.require_scope(scopes::RUNS_READ)
    }
}

fn invalid_transition() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InvalidRunTransition,
        "the requested run state transition is invalid",
    )
}

fn unavailable_interaction(caller: &CallerContext) -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InteractionUnavailable,
        "the interaction is not available for this caller",
    )
    .with_correlation_id(caller.request_id().clone())
}

fn invariant(caller: &CallerContext, message: &str) -> ApiError {
    ApiError::internal(message).with_correlation_id(caller.request_id().clone())
}
