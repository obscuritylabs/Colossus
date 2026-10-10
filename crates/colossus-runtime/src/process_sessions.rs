//! Runtime-owned managed shell sessions. Only released chunks enter this registry.
use super::*;
use colossus_contracts::{
    ProcessLifetime, ProcessOutputChunk, ProcessSessionPage, ProcessSessionSnapshot,
    ProcessSessionStatus, ProcessSessionSummary,
};
use colossus_policy::{QuarantinedEffectObserver, StreamingEffectExecutor};
use colossus_ports::AgentRunLifecycle;
use colossus_sandbox::ProcessControl;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

mod concurrency;
mod output;
mod persistence;
mod public;
#[cfg(test)]
mod tests;
use concurrency::ConcurrencyScopes;
use output::LogDecoder;

const RETAINED_SESSIONS: usize = 256;
const MAX_ACTIVE_SESSIONS: usize = 32;
const RETAINED_OUTPUT: usize = 64 * 1024;
const CATALOG_STREAM: &str = "process-session-catalog";

pub(super) struct ProcessSessions {
    journal: Arc<dyn EventJournal>,
    gateway: Arc<EffectGateway>,
    executor: Arc<SandboxProcessExecutor>,
    identity: workspace_lease::WorkspaceIdentity,
    lease: Arc<workspace_lease::WorkspaceOwnershipLease>,
    scoped_active: Arc<ConcurrencyScopes>,
    active: Arc<AtomicUsize>,
    registry: StdMutex<Registry>,
}
#[derive(Default)]
struct Registry {
    runs: BTreeMap<String, RunOwner>,
    sessions: BTreeMap<String, Arc<ManagedSession>>,
    catalog_version: u64,
}
#[derive(Clone)]
struct RunOwner {
    actor: Actor,
    control: RunControl,
    context: ExecutionContext,
}
struct ManagedSession {
    journal: Arc<dyn EventJournal>,
    context: ExecutionContext,
    state: StdMutex<SessionState>,
    control: ProcessControl,
    changed: tokio::sync::Notify,
    done: AtomicBool,
    launched: AtomicBool,
    executing: AtomicBool,
}
struct SessionState {
    summary: ProcessSessionSummary,
    version: u64,
    chunks: VecDeque<ProcessOutputChunk>,
    bytes: usize,
    stdout: LogDecoder,
    stderr: LogDecoder,
    logs_unavailable: bool,
}

fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(0)
}
fn process_failure(message: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(message.to_string())
}
fn state(session: &ManagedSession) -> std::sync::MutexGuard<'_, SessionState> {
    session
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ProcessSessions {
    pub(super) fn journal_for_workflows(&self) -> &dyn EventJournal {
        self.journal.as_ref()
    }

    pub(super) fn registry_for_workflows(
        &self,
        context: &ExecutionContext,
    ) -> Result<Actor, ToolError> {
        let registry = self.registry();
        let owner = context
            .run_id
            .as_ref()
            .and_then(|id| registry.runs.get(id))
            .ok_or_else(|| ToolError::Denied("active run ownership is required".into()))?;
        if owner.control.is_cancelled()
            || owner.context.session_id != context.session_id
            || owner.context.subagent_id != context.subagent_id
        {
            return Err(ToolError::Denied(
                "active run lineage does not match".into(),
            ));
        }
        Ok(owner.actor.clone())
    }
    pub(super) fn open(
        journal: Arc<dyn EventJournal>,
        gateway: Arc<EffectGateway>,
        executor: Arc<SandboxProcessExecutor>,
        lease: Arc<workspace_lease::WorkspaceOwnershipLease>,
    ) -> Result<Self, StoreError> {
        let registry = persistence::recover(&journal)?;
        Ok(Self {
            journal,
            gateway,
            executor,
            identity: lease.identity(),
            lease,
            scoped_active: Arc::new(ConcurrencyScopes::default()),
            active: Arc::new(AtomicUsize::new(0)),
            registry: StdMutex::new(registry),
        })
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) async fn launch(
        &self,
        mut request: EffectRequest,
        lifetime: ProcessLifetime,
        isolated: Option<tempfile::TempDir>,
        wait_ms: u64,
    ) -> Result<ProcessSessionSnapshot, ToolError> {
        let run_id = request
            .context
            .run_id
            .as_deref()
            .ok_or_else(|| process_failure("managed shell requires an agent run"))?;
        let display = colossus_policy::command_approval_context(&request)
            .map_err(tool_gateway_error)?
            .ok_or_else(|| process_failure("managed shell requires command intent"))?;
        let session;
        let owner;
        {
            let mut registry = self.registry();
            owner = registry
                .runs
                .get(run_id)
                .cloned()
                .ok_or_else(|| process_failure("agent run is no longer active"))?;
            if owner.control.is_cancelled() {
                return Err(process_failure("agent run was cancelled"));
            }
            if owner.context.session_id != request.context.session_id
                || owner.context.subagent_id != request.context.subagent_id
            {
                return Err(ToolError::Denied(
                    "managed shell provenance mismatch".into(),
                ));
            }
            if self.active.load(Ordering::Acquire) >= MAX_ACTIVE_SESSIONS {
                return Err(process_failure(
                    "workspace managed shell capacity reached; stop or await an existing session",
                ));
            }
            while registry.sessions.len() >= RETAINED_SESSIONS {
                let id = registry
                    .sessions
                    .iter()
                    .find(|(_, session)| session.done.load(Ordering::Acquire))
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| process_failure("managed shell capacity reached"))?;
                registry.sessions.remove(&id);
            }
            let summary = ProcessSessionSummary {
                id: Uuid::now_v7().to_string(),
                session_id: request
                    .context
                    .session_id
                    .clone()
                    .ok_or_else(|| process_failure("managed shell requires a conversation"))?,
                run_id: run_id.into(),
                owner: owner.actor.clone(),
                subagent_id: request.context.subagent_id.clone(),
                lifetime,
                status: ProcessSessionStatus::Starting,
                command: format!("{} {}", display.executable, display.arguments.join(" "))
                    .chars()
                    .take(8192)
                    .collect(),
                cwd: display.working_directory,
                created_at_ms: now_ms(),
                deadline_ms: None,
                exit_code: None,
                reason: None,
                truncated: false,
                output_sequence: 0,
            };
            session = Arc::new(ManagedSession::new(
                Arc::clone(&self.journal),
                request.context.clone(),
                summary,
            ));
            persistence::insert(&mut registry, &session, self.journal.as_ref())
                .map_err(process_failure)?;
            self.active.fetch_add(1, Ordering::AcqRel);
        }
        request.content["lifetime"] = serde_json::to_value(lifetime).map_err(process_failure)?;
        let executor = ManagedExecutor {
            inner: self.executor.controlled(session.control.clone()),
            identity: self.identity.clone(),
            session: Arc::clone(&session),
            scoped_active: Arc::clone(&self.scoped_active),
        };
        let gateway = Arc::clone(&self.gateway);
        let job = Arc::clone(&session);
        let active = Arc::clone(&self.active);
        let lease = Arc::clone(&self.lease);
        let instructions = active_instruction_snapshot();
        let plugins = active_plugin_catalog().unwrap_or_default();
        let boundary = colossus_policy::SandboxBoundaryScope::capture();
        let mut supervisor = Box::pin(scope_run_snapshots(instructions, plugins, boundary.scope(async move {
            let _lease = lease;
            let _isolated = isolated;
            let _unwind = SessionTaskGuard { session: Arc::clone(&job), active: Arc::clone(&active) };
            let mut observer = SessionObserver { session: Arc::clone(&job), run_control: owner.control.clone() };
            let mut execution = Box::pin(gateway.execute_stream(request, &executor, &mut observer));
            let result = loop {
                tokio::select! {
                    result = &mut execution => break result,
                    () = tokio::time::sleep(Duration::from_millis(20)) => {
                        let current = state(&job);
                        if owner.control.is_cancelled() && (current.summary.lifetime == ProcessLifetime::Run || current.summary.status == ProcessSessionStatus::Starting) { job.control.cancel(); }
                        drop(current);
                        if job.control.is_cancelled() && !job.executing.load(Ordering::Acquire) {
                            break Err(GatewayError::Denied("managed shell stopped before launch".into()));
                        }
                    }
                }
            };
            // The gateway future (including pending approval) is dropped before finalization.
            drop(execution);
            job.complete(result);
            active.fetch_sub(1, Ordering::AcqRel);
            job.done.store(true, Ordering::Release);
            job.changed.notify_waiters();
        })));
        // Task-local public/interactive approval routing belongs to the caller.
        // Resolve approval here; transfer only an authorized invocation to the
        // detached supervisor. Yield time never expires an unanswered approval.
        loop {
            let notified = session.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if session.executing.load(Ordering::Acquire) {
                tokio::spawn(supervisor);
                break;
            }
            tokio::select! {
                () = &mut supervisor => break,
                () = notified => {}
            }
        }
        session
            .wait_for_snapshot(0, wait_ms, RETAINED_OUTPUT, true)
            .await
    }

    fn authorized(
        &self,
        id: &str,
        context: &ExecutionContext,
    ) -> Result<Arc<ManagedSession>, ToolError> {
        let registry = self.registry();
        let owner = context
            .run_id
            .as_ref()
            .and_then(|id| registry.runs.get(id))
            .ok_or_else(|| ToolError::Denied("active run ownership is required".into()))?;
        let session = registry.sessions.get(id).ok_or_else(|| {
            ToolError::Denied("shell session is not available in this scope".into())
        })?;
        let summary = &state(session).summary;
        if summary.owner != owner.actor
            || Some(&summary.session_id) != context.session_id.as_ref()
            || summary.subagent_id != context.subagent_id
            || (summary.lifetime == ProcessLifetime::Run
                && Some(&summary.run_id) != context.run_id.as_ref())
        {
            return Err(ToolError::Denied(
                "shell session is not available in this scope".into(),
            ));
        }
        Ok(Arc::clone(session))
    }

    pub(super) async fn tool(
        &self,
        call: &ToolCall,
        context: &ExecutionContext,
    ) -> Result<Value, ToolError> {
        if call.name == "shell.list" {
            let registry = self.registry();
            let owner = context
                .run_id
                .as_ref()
                .and_then(|id| registry.runs.get(id))
                .ok_or_else(|| ToolError::Denied("active run ownership is required".into()))?;
            return serde_json::to_value(list_page(
                &registry,
                |summary| {
                    summary.owner == owner.actor
                        && Some(&summary.session_id) == context.session_id.as_ref()
                        && summary.subagent_id == context.subagent_id
                        && (summary.lifetime == ProcessLifetime::Workspace
                            || Some(&summary.run_id) == context.run_id.as_ref())
                },
                call.arguments.get("after").and_then(Value::as_str),
            ))
            .map_err(process_failure);
        }
        let id = call
            .arguments
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| process_failure("session_id is required"))?;
        let session = self.authorized(id, context)?;
        if call.name == "shell.stop" {
            session.stop();
        }
        let after = call
            .arguments
            .get("after_sequence")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let wait = if call.name == "shell.wait" {
            call.arguments
                .get("yield_time_ms")
                .and_then(Value::as_u64)
                .unwrap_or(10_000)
        } else {
            0
        };
        let limit = usize::try_from(
            call.arguments
                .get("max_output_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(RETAINED_OUTPUT as u64),
        )
        .map_err(process_failure)?;
        serde_json::to_value(session.wait(after, wait, limit).await?).map_err(process_failure)
    }

    pub(super) fn stop_all(&self) {
        for session in self.registry().sessions.values() {
            session.stop();
        }
    }
}

fn list_page(
    registry: &Registry,
    allow: impl Fn(&ProcessSessionSummary) -> bool,
    after: Option<&str>,
) -> ProcessSessionPage {
    let mut sessions = registry
        .sessions
        .values()
        .map(|session| state(session).summary.clone())
        .filter(|summary| after.is_none_or(|after| summary.id.as_str() > after) && allow(summary))
        .take(101)
        .collect::<Vec<_>>();
    let next_cursor = (sessions.len() > 100).then(|| sessions[99].id.clone());
    sessions.truncate(100);
    ProcessSessionPage {
        sessions,
        next_cursor,
    }
}

impl Drop for ProcessSessions {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[async_trait]
impl AgentRunLifecycle for ProcessSessions {
    fn begin_run(
        &self,
        context: &ExecutionContext,
        initiator: &Actor,
        control: RunControl,
    ) -> Result<(), ToolError> {
        let mut registry = self.registry();
        let id = context
            .run_id
            .as_ref()
            .ok_or_else(|| process_failure("run id is required"))?;
        if registry.runs.contains_key(id) {
            return Err(process_failure("duplicate active run"));
        }
        registry.runs.insert(
            id.clone(),
            RunOwner {
                actor: initiator.clone(),
                control,
                context: context.clone(),
            },
        );
        Ok(())
    }
    fn cancel_run(&self, run_id: &str) {
        let mut registry = self.registry();
        registry.runs.remove(run_id);
        for session in registry.sessions.values() {
            let mut current = state(session);
            if current.summary.run_id == run_id
                && (current.summary.lifetime == ProcessLifetime::Run
                    || current.summary.status == ProcessSessionStatus::Starting)
            {
                session.stop_locked(&mut current);
            }
        }
    }
    async fn finish_run(&self, run_id: &str) {
        self.cancel_run(run_id);
        let sessions = self
            .registry()
            .sessions
            .values()
            .filter(|session| {
                let current = state(session);
                current.summary.run_id == run_id && session.control.is_cancelled()
            })
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            loop {
                let notified = session.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if session.done.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        }
    }
}

struct ManagedExecutor {
    inner: SandboxProcessExecutor,
    identity: workspace_lease::WorkspaceIdentity,
    session: Arc<ManagedSession>,
    scoped_active: Arc<ConcurrencyScopes>,
}
#[async_trait]
impl StreamingEffectExecutor for ManagedExecutor {
    async fn execute_stream(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.identity
            .revalidate()
            .map_err(|_| ExecutionError::Failed("workspace identity changed".into()))?;
        if self.session.control.is_cancelled() {
            return Err(ExecutionError::Failed(
                "managed process stopped before launch".into(),
            ));
        }
        let _scope = self
            .scoped_active
            .acquire(request, permit.obligations().max_concurrency)?;
        // Once the adapter starts, await its authenticated cleanup result even
        // when cancellation races the first Started frame.
        self.session.executing.store(true, Ordering::Release);
        self.session.changed.notify_waiters();
        let mut launch_observer = LaunchObserver {
            session: &self.session,
            inner: observer,
        };
        self.inner
            .execute_stream(request, permit, &mut launch_observer)
            .await
    }
}

// Remember authenticated launch evidence before release policy. A denied Started
// frame must not make an already-spawned process look like a pre-launch failure.
struct LaunchObserver<'a> {
    session: &'a ManagedSession,
    inner: &'a mut dyn QuarantinedEffectObserver,
}
#[async_trait]
impl QuarantinedEffectObserver for LaunchObserver<'_> {
    async fn observe(&mut self, chunk: QuarantinedEffectResult) -> Result<(), ExecutionError> {
        let frame: Value = serde_json::from_slice(&chunk.bytes)
            .map_err(|_| ExecutionError::OutcomeUnknown("invalid process frame".into()))?;
        if frame["kind"] == "started" {
            self.session.launched.store(true, Ordering::Release);
        }
        self.inner.observe(chunk).await
    }
}

struct SessionObserver {
    session: Arc<ManagedSession>,
    run_control: RunControl,
}
#[async_trait]
impl ReleasedEffectObserver for SessionObserver {
    async fn observe(&mut self, released: ReleasedEffectResult) -> Result<(), ExecutionError> {
        let value: Value = serde_json::from_slice(&released.bytes)
            .map_err(|_| ExecutionError::OutcomeUnknown("invalid released process frame".into()))?;
        let mut current = state(&self.session);
        match value.get("kind").and_then(Value::as_str) {
            Some("started") => {
                if self.run_control.is_cancelled() {
                    self.session.control.cancel();
                }
                current.summary.status = if self.session.control.is_cancelled() {
                    ProcessSessionStatus::Stopping
                } else {
                    ProcessSessionStatus::Running
                };
                current.summary.deadline_ms = Some(
                    value
                        .get("deadline_ms")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| {
                            ExecutionError::OutcomeUnknown("process deadline is absent".into())
                        })?,
                );
                self.session.save(&mut current).map_err(|_| {
                    ExecutionError::OutcomeUnknown("process lifecycle persistence failed".into())
                })?;
            }
            Some("output") => {
                let stdout = decode_frame(&value, "stdout_base64")?;
                let stderr = decode_frame(&value, "stderr_base64")?;
                current.push(&stdout, &stderr, false);
            }
            Some("completed") => {
                current.push(&[], &[], true);
            }
            _ => {
                return Err(ExecutionError::OutcomeUnknown(
                    "invalid released process frame kind".into(),
                ));
            }
        }
        drop(current);
        self.session.changed.notify_waiters();
        Ok(())
    }
}
fn decode_frame(value: &Value, field: &str) -> Result<Vec<u8>, ExecutionError> {
    BASE64
        .decode(value.get(field).and_then(Value::as_str).unwrap_or(""))
        .map_err(|_| ExecutionError::OutcomeUnknown("invalid released process output".into()))
}

impl ManagedSession {
    fn new(
        journal: Arc<dyn EventJournal>,
        context: ExecutionContext,
        summary: ProcessSessionSummary,
    ) -> Self {
        Self {
            journal,
            context,
            state: StdMutex::new(SessionState {
                summary,
                version: 0,
                chunks: VecDeque::new(),
                bytes: 0,
                stdout: LogDecoder::default(),
                stderr: LogDecoder::default(),
                logs_unavailable: false,
            }),
            control: ProcessControl::default(),
            changed: tokio::sync::Notify::new(),
            done: AtomicBool::new(false),
            launched: AtomicBool::new(false),
            executing: AtomicBool::new(false),
        }
    }
    fn stop(&self) {
        self.stop_locked(&mut state(self));
    }
    fn stop_locked(&self, current: &mut SessionState) {
        if !current.summary.status.is_active() {
            return;
        }
        self.control.cancel();
        current.summary.status = ProcessSessionStatus::Stopping;
        self.changed.notify_waiters();
    }
    fn complete(&self, result: Result<ReleasedEffectResult, GatewayError>) {
        let mut current = state(self);
        match result {
            Ok(released) => {
                let result = serde_json::from_slice::<Value>(&released.bytes)
                    .ok()
                    .and_then(|value| value.get("result").cloned());
                if let Some(result) = result {
                    current.summary.status = if result["stopped"] == true {
                        ProcessSessionStatus::Stopped
                    } else if result["timed_out"] == true {
                        ProcessSessionStatus::TimedOut
                    } else if result["resource_limit_exceeded"].is_string() {
                        ProcessSessionStatus::Failed
                    } else {
                        ProcessSessionStatus::Exited
                    };
                    current.summary.exit_code = result["exit_code"]
                        .as_i64()
                        .and_then(|code| i32::try_from(code).ok());
                    current.summary.truncated |= result["output_truncated"] == true;
                    current.summary.reason = result["resource_limit_exceeded"]
                        .as_str()
                        .map(|limit| format!("Process exceeded its {limit} limit"));
                } else {
                    current.summary.status = ProcessSessionStatus::OutcomeUnknown;
                }
            }
            Err(error) => {
                let uncertain = self.launched.load(Ordering::Acquire)
                    || matches!(error, GatewayError::OutcomeUnknown(_));
                current.summary.status = if uncertain {
                    ProcessSessionStatus::OutcomeUnknown
                } else if self.control.is_cancelled() {
                    ProcessSessionStatus::Stopped
                } else {
                    ProcessSessionStatus::Failed
                };
                current.summary.reason = Some(
                    if uncertain {
                        "Process result or cleanup could not be confirmed through policy release"
                    } else {
                        launch_failure_reason(&error)
                    }
                    .into(),
                );
            }
        }
        if self.save(&mut current).is_err() {
            current.summary.status = ProcessSessionStatus::OutcomeUnknown;
            current.summary.reason = Some("Process lifecycle persistence failed".into());
        }
    }
    async fn wait(
        &self,
        after: u64,
        wait_ms: u64,
        limit: usize,
    ) -> Result<ProcessSessionSnapshot, ToolError> {
        self.wait_for_snapshot(after, wait_ms, limit, false).await
    }

    async fn wait_for_snapshot(
        &self,
        after: u64,
        wait_ms: u64,
        limit: usize,
        until_exit: bool,
    ) -> Result<ProcessSessionSnapshot, ToolError> {
        if wait_ms > 30_000 || !(16_384..=RETAINED_OUTPUT).contains(&limit) {
            return Err(process_failure(
                "wait must be 0..=30000 ms and output must be 16384..=65536 bytes",
            ));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let ready = {
                let current = state(self);
                !current.summary.status.is_active()
                    || (!until_exit && current.summary.output_sequence > after)
            };
            if ready || wait_ms == 0 {
                break;
            }
            // Launch waits through progress until exit or its original deadline.
            // Read/wait consumers still wake for released output, but not Started.
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                break;
            }
        }
        let current = state(self);
        if after > current.summary.output_sequence {
            return Err(process_failure("output cursor is ahead of this session"));
        }
        let first = current
            .chunks
            .front()
            .map_or(current.summary.output_sequence.saturating_add(1), |chunk| {
                chunk.sequence
            });
        let gap = current.logs_unavailable || after.saturating_add(1) < first;
        let mut bytes = 0;
        let chunks = current
            .chunks
            .iter()
            .filter(|chunk| chunk.sequence > after)
            .take_while(|chunk| {
                bytes += chunk.stdout.len() + chunk.stderr.len();
                bytes <= limit
            })
            .cloned()
            .collect::<Vec<_>>();
        let next_sequence = chunks
            .last()
            .map_or(after.max(first.saturating_sub(1)), |chunk| chunk.sequence);
        Ok(ProcessSessionSnapshot {
            session: current.summary.clone(),
            chunks,
            next_sequence,
            gap,
        })
    }
}
impl SessionState {
    fn push(&mut self, stdout: &[u8], stderr: &[u8], final_output: bool) {
        let stdout = self.stdout.push(stdout, final_output);
        let stderr = self.stderr.push(stderr, final_output);
        if stdout.is_empty() && stderr.is_empty() {
            return;
        }
        self.summary.output_sequence += 1;
        self.bytes += stdout.len() + stderr.len();
        self.chunks.push_back(ProcessOutputChunk {
            sequence: self.summary.output_sequence,
            stdout,
            stderr,
        });
        while self.bytes > RETAINED_OUTPUT || self.chunks.len() > 256 {
            if let Some(chunk) = self.chunks.pop_front() {
                self.bytes -= chunk.stdout.len() + chunk.stderr.len();
                self.summary.truncated = true;
            }
        }
    }
}

struct SessionToolEffect<'a> {
    sessions: &'a ProcessSessions,
    call: &'a ToolCall,
}
#[async_trait]
impl EffectExecutor for SessionToolEffect<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.sessions
            .identity
            .revalidate()
            .map_err(|_| ExecutionError::Failed("workspace identity changed".into()))?;
        let value = self
            .sessions
            .tool(self.call, &request.context)
            .await
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if bytes.len() as u64 > permit.obligations().max_output_bytes {
            return Err(ExecutionError::Failed(
                "shell snapshot exceeds authorized output bound".into(),
            ));
        }
        Ok(QuarantinedEffectResult {
            bytes,
            media_type: "application/json".into(),
            effect_succeeded: true,
        })
    }
}
impl GatewayToolExecutor {
    pub(super) async fn execute_process_session(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let sessions = self
            .process_sessions
            .as_ref()
            .ok_or_else(|| process_failure("managed shell sessions unavailable"))?;
        let action = if call.name == "shell.wait" {
            "shell.read"
        } else {
            &call.name
        };
        let mut request = effect_request(
            model_actor(&call, &context),
            action,
            call.arguments
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("process-sessions"),
            call.arguments.clone(),
        );
        request.capabilities = vec![action.into()];
        request.context = context;
        let result = self
            .gateway
            .execute(
                request,
                &SessionToolEffect {
                    sessions,
                    call: &call,
                },
            )
            .await
            .map_err(tool_gateway_error)?;
        Ok(ToolResult {
            images: Vec::new(),
            call_id: call.call_id,
            name: call.name,
            output: String::from_utf8(result.bytes).map_err(process_failure)?,
            exit_code: 0,
        })
    }
}

fn launch_failure_reason(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::Approval(_) => "Process was not launched: approval was denied",
        GatewayError::Policy(_) => {
            "Process was not launched: the policy or approval service failed"
        }
        GatewayError::Denied(_) | GatewayError::Safety(_) => {
            "Process was not launched: blocked by policy or sandbox permissions"
        }
        GatewayError::Journal(_) => "Process was not launched: authorization could not be recorded",
        GatewayError::Contract(_) => "Process was not launched: invalid process request",
        _ => "Process was not launched: process startup failed",
    }
}

struct SessionTaskGuard {
    session: Arc<ManagedSession>,
    active: Arc<AtomicUsize>,
}
impl Drop for SessionTaskGuard {
    fn drop(&mut self) {
        if !self.session.done.swap(true, Ordering::AcqRel) {
            self.session.control.cancel();
            let error = if self.session.executing.load(Ordering::Acquire) {
                GatewayError::OutcomeUnknown("managed session task interrupted".into())
            } else {
                GatewayError::Denied("managed session cancelled before launch".into())
            };
            self.session.complete(Err(error));
            self.active.fetch_sub(1, Ordering::AcqRel);
            self.session.changed.notify_waiters();
        }
    }
}
