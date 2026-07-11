//! Authenticated local IPC for the single-writer Colossus worker.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{AgentRunResult, ProviderEvent};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeError};
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use uuid::Uuid;

const PROTOCOL_VERSION: u16 = 1;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLOCK_SKEW_MS: i128 = 30_000;
const REPLAY_WINDOW: usize = 4_096;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
type HmacSha256 = Hmac<Sha256>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientHello {
    version: u16,
    challenge: String,
}

#[derive(Serialize)]
struct UnsignedServerHello<'a> {
    version: u16,
    challenge: &'a str,
    server_nonce: &'a str,
    timestamp_ms: i128,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerHello {
    version: u16,
    challenge: String,
    server_nonce: String,
    timestamp_ms: i128,
    authentication_tag: String,
}

/// Local worker transport or strict-contract failure.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// Local transport failed.
    #[error("worker transport failed: {0}")]
    Io(#[from] std::io::Error),
    /// Runtime composition or operation failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Journal or repository operation failed.
    #[error(transparent)]
    Store(#[from] colossus_ports::StoreError),
    /// Workflow validation or lifecycle operation failed.
    #[error(transparent)]
    Workflow(#[from] colossus_workflow::WorkflowError),
    /// Strict JSON serialization failed.
    #[error("worker JSON contract failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Request or response violated the authenticated protocol.
    #[error("worker protocol rejected message: {0}")]
    Protocol(String),
    /// The worker returned an application error.
    #[error("worker operation failed: {0}")]
    Remote(String),
    /// No worker answered at the configured endpoint.
    #[error("worker is unavailable at {0}")]
    Unavailable(String),
}

/// Versioned operations exposed by the local worker application API.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerOperation {
    /// Authenticate the endpoint and return bounded readiness metadata.
    Ping,
    /// Verify the authoritative journal chain and anchors.
    AuditVerify,
    /// Read bounded redacted event envelopes.
    AuditRead {
        /// First global sequence.
        from: u64,
        /// Maximum records.
        limit: usize,
    },
    /// Check policy readiness.
    PolicyDoctor,
    /// List projection positions and lag.
    ProjectionStatus,
    /// Drain projection outbox work.
    ProjectionDrain,
    /// Rebuild one or every disposable projection.
    ProjectionRebuild {
        /// Optional exact projection name.
        name: Option<String>,
    },
    /// Inspect storage and writer readiness.
    StateDoctor,
    /// Inspect sandbox readiness.
    SandboxDoctor,
    /// List provider profile readiness without network access.
    ProviderProfiles,
    /// Exercise one provider diagnostic path.
    ProviderDoctor {
        /// Optional exact profile.
        profile: Option<String>,
    },
    /// List normalized models for one provider.
    ProviderModels {
        /// Optional exact profile.
        profile: Option<String>,
    },
    /// Show role-to-profile routing.
    ProviderRoutes,
    /// List active model-visible tool schemas.
    ToolsList,
    /// Execute the normal audited model application path.
    RunModel {
        /// Logical role.
        role: String,
        /// Composed caller instructions.
        instructions: String,
        /// User prompt.
        prompt: String,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Optional exact durable session.
        session_id: Option<String>,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// REPL-sticky declarative skills.
        sticky_skills: Vec<String>,
    },
    /// Execute the permit-bound offline echo effect.
    Echo {
        /// Exact echo input.
        message: String,
    },
    /// Create an empty durable session.
    SessionCreate {
        /// Optional bounded title.
        title: Option<String>,
    },
    /// Reconstruct one session summary.
    SessionGet {
        /// Exact session identifier.
        session_id: String,
    },
    /// List recent sessions.
    SessionList {
        /// Bounded result limit.
        limit: usize,
    },
    /// Reconstruct append-only messages for one session.
    SessionMessages {
        /// Exact session identifier.
        session_id: String,
    },
    /// Resolve the newest session.
    SessionLatest,
    /// Show context budget status.
    ContextStatus {
        /// Exact session identifier.
        session_id: String,
    },
    /// List immutable context snapshots.
    ContextList {
        /// Exact session identifier.
        session_id: String,
    },
    /// Force context compaction.
    ContextCompact {
        /// Exact session identifier.
        session_id: String,
    },
    /// Activate one context snapshot.
    ContextRestore {
        /// Exact session identifier.
        session_id: String,
        /// Exact snapshot identifier.
        snapshot_id: String,
    },
    /// List metadata-only run telemetry.
    TelemetryRuns {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum runs.
        limit: usize,
    },
    /// Show one metadata-only run timeline.
    TelemetryShow {
        /// Full or uniquely prefixed run identifier.
        id_or_prefix: String,
        /// Maximum timeline records.
        limit: usize,
    },
    /// Aggregate metadata-only run metrics.
    TelemetryMetrics {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum runs.
        limit: usize,
    },
    /// Validate a workflow file through the normal filesystem policy boundary.
    WorkflowValidate {
        /// Server-local workflow path.
        path: String,
    },
    /// Register a workflow file through the normal filesystem policy boundary.
    WorkflowRegister {
        /// Server-local workflow path.
        path: String,
    },
    /// List registered workflow definition audit metadata.
    WorkflowList,
    /// Show one exact registered workflow.
    WorkflowShow {
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
    },
    /// Start a workflow run.
    WorkflowStart {
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Inline JSON or a server-local `@path` reference.
        inputs_source: String,
    },
    /// Reconstruct one workflow run.
    WorkflowStatus {
        /// Exact run identifier.
        run_id: String,
    },
    /// Resume one safe interrupted/waiting workflow run.
    WorkflowResume {
        /// Exact run identifier.
        run_id: String,
    },
    /// Provide structured input to a waiting run.
    WorkflowInput {
        /// Exact run identifier.
        run_id: String,
        /// Inline JSON or a server-local `@path` reference.
        input_source: String,
    },
    /// Cancel a non-terminal workflow run.
    WorkflowCancel {
        /// Exact run identifier.
        run_id: String,
    },
    /// Drain safe queued work and projections once.
    Drain,
    /// Request clean checkpoint and worker shutdown.
    Shutdown,
}

#[derive(Serialize)]
struct UnsignedRequest<'a> {
    version: u16,
    request_id: &'a str,
    timestamp_ms: i128,
    nonce: &'a str,
    connection_nonce: &'a str,
    operation: &'a WorkerOperation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerRequest {
    version: u16,
    request_id: String,
    timestamp_ms: i128,
    nonce: String,
    connection_nonce: String,
    operation: WorkerOperation,
    authentication_tag: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerFrameContent {
    Event { event: ProviderEvent },
    Complete { result: Value },
    Error { message: String },
}

#[derive(Serialize)]
struct UnsignedFrame<'a> {
    version: u16,
    request_id: &'a str,
    sequence: u64,
    timestamp_ms: i128,
    content: &'a WorkerFrameContent,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerFrame {
    version: u16,
    request_id: String,
    sequence: u64,
    timestamp_ms: i128,
    content: WorkerFrameContent,
    authentication_tag: String,
}

#[derive(Default)]
struct ReplayGuard {
    order: VecDeque<String>,
    entries: BTreeSet<String>,
}

impl ReplayGuard {
    fn accept(&mut self, nonce: &str) -> Result<(), WorkerError> {
        if nonce.is_empty() || nonce.len() > 128 || !self.entries.insert(nonce.into()) {
            return Err(WorkerError::Protocol(
                "empty, oversized, or replayed request nonce".into(),
            ));
        }
        self.order.push_back(nonce.into());
        while self.order.len() > REPLAY_WINDOW {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
        Ok(())
    }
}

/// Authenticated one-request-per-connection worker client.
pub struct WorkerClient {
    endpoint: String,
    authentication_key: [u8; 32],
}

impl WorkerClient {
    /// Resolve a client only when a platform endpoint may currently exist.
    pub fn discover(config: &RuntimeConfig) -> Result<Option<Self>, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Ok(None);
        }
        Ok(Some(Self {
            endpoint,
            authentication_key: config.worker_ipc_auth_key()?,
        }))
    }

    /// Resolve the platform endpoint and authentication key from runtime configuration.
    pub fn from_config(config: &RuntimeConfig) -> Result<Self, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Err(WorkerError::Unavailable(endpoint));
        }
        Ok(Self {
            endpoint,
            authentication_key: config.worker_ipc_auth_key()?,
        })
    }

    /// Exact configured Unix socket or Windows named-pipe endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Return readiness metadata when an authenticated worker is listening.
    pub async fn ping(&self) -> Result<Value, WorkerError> {
        self.call(WorkerOperation::Ping).await
    }

    /// Execute a non-streaming worker operation.
    pub async fn call(&self, operation: WorkerOperation) -> Result<Value, WorkerError> {
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, &self.authentication_key),
        )
        .await
        .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))??;
        let request = signed_request(&self.authentication_key, operation, &connection_nonce)?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let mut sequence = 0_u64;
        let frame: WorkerFrame = read_message(&mut stream, MAX_FRAME_BYTES).await?;
        validate_frame(
            &self.authentication_key,
            &request.request_id,
            &mut sequence,
            &frame,
        )?;
        match frame.content {
            WorkerFrameContent::Event { .. } => Err(WorkerError::Protocol(
                "non-streaming call received a provider event".into(),
            )),
            WorkerFrameContent::Complete { result } => Ok(result),
            WorkerFrameContent::Error { message } => Err(WorkerError::Remote(message)),
        }
    }

    /// Execute one model run while forwarding authenticated released provider events.
    pub async fn run_model(
        &self,
        operation: WorkerOperation,
        observer: &mut dyn colossus_ports::ProviderEventObserver,
    ) -> Result<AgentRunResult, WorkerError> {
        if !matches!(operation, WorkerOperation::RunModel { .. }) {
            return Err(WorkerError::Protocol(
                "run_model requires a run_model operation".into(),
            ));
        }
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, &self.authentication_key),
        )
        .await
        .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))??;
        let request = signed_request(&self.authentication_key, operation, &connection_nonce)?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let mut sequence = 0_u64;
        loop {
            let frame: WorkerFrame = read_message(&mut stream, MAX_FRAME_BYTES).await?;
            validate_frame(
                &self.authentication_key,
                &request.request_id,
                &mut sequence,
                &frame,
            )?;
            match frame.content {
                WorkerFrameContent::Event { event } => observer
                    .observe(event)
                    .await
                    .map_err(|error| WorkerError::Remote(error.to_string()))?,
                WorkerFrameContent::Complete { result } => {
                    return serde_json::from_value(result).map_err(|error| {
                        WorkerError::Protocol(format!("invalid run result: {error}"))
                    });
                }
                WorkerFrameContent::Error { message } => return Err(WorkerError::Remote(message)),
            }
        }
    }

    async fn connect(&self) -> Result<platform::ClientStream, WorkerError> {
        tokio::time::timeout(CONNECT_TIMEOUT, platform::connect(&self.endpoint))
            .await
            .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))?
            .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))
    }
}

/// Long-running single-writer runtime owner and authenticated IPC server.
pub struct WorkerServer {
    endpoint: String,
    authentication_key: [u8; 32],
    runtime: Arc<Runtime>,
    replay: Arc<Mutex<ReplayGuard>>,
    maintenance: Arc<tokio::sync::Mutex<()>>,
}

impl WorkerServer {
    /// Open the runtime (and therefore acquire the writer lease) before binding IPC.
    pub fn open(
        config: &RuntimeConfig,
        approvals: Arc<dyn colossus_ports::ApprovalProvider>,
    ) -> Result<Self, WorkerError> {
        Ok(Self {
            endpoint: config.worker_ipc_endpoint()?,
            authentication_key: config.worker_ipc_auth_key()?,
            runtime: Arc::new(Runtime::open_with_approval(config, approvals)?),
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Exact bound endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Serve until Ctrl-C or an authenticated shutdown request, then checkpoint cleanly.
    pub async fn serve(self) -> Result<(), WorkerError> {
        let mut listener = platform::Listener::bind(&self.endpoint).await?;
        let runtime = Arc::clone(&self.runtime);
        let replay = Arc::clone(&self.replay);
        let maintenance = Arc::clone(&self.maintenance);
        let key = self.authentication_key;
        let mut drain_interval = tokio::time::interval(Duration::from_secs(1));
        drain_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        drain_interval.tick().await;
        let draining = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut tasks = tokio::task::JoinSet::new();
        let mut stop = false;
        while !stop {
            tokio::select! {
                accepted = listener.accept() => {
                    let mut stream = accepted?;
                    let runtime = Arc::clone(&runtime);
                    let replay = Arc::clone(&replay);
                    let maintenance = Arc::clone(&maintenance);
                    let shutdown = shutdown_tx.clone();
                    tasks.spawn(async move {
                        if handle_connection(
                            &mut stream,
                            &key,
                            runtime.as_ref(),
                            replay.as_ref(),
                            maintenance.as_ref(),
                        )
                            .await
                            .is_ok_and(|stopping| stopping)
                        {
                            let _ = shutdown.send(()).await;
                        }
                    });
                }
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    stop = true;
                }
                requested = shutdown_rx.recv() => {
                    stop = requested.is_some();
                }
                _ = drain_interval.tick() => {
                    if draining.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                        let runtime = Arc::clone(&runtime);
                        let draining = Arc::clone(&draining);
                        let maintenance = Arc::clone(&maintenance);
                        tasks.spawn(async move {
                            let _ = drain_once(runtime.as_ref(), maintenance.as_ref()).await;
                            draining.store(false, Ordering::Release);
                        });
                    }
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(result) = completed {
                        result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
                    }
                }
            }
        }
        drop(shutdown_tx);
        while let Some(result) = tasks.join_next().await {
            result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
        }
        runtime.checkpoint()?;
        listener.cleanup();
        Ok(())
    }
}

async fn handle_connection<S>(
    stream: &mut S,
    key: &[u8; 32],
    runtime: &Runtime,
    replay: &Mutex<ReplayGuard>,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<bool, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let connection_nonce =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, server_handshake(stream, key))
            .await
            .map_err(|_| WorkerError::Protocol("worker client handshake timed out".into()))
            .and_then(std::convert::identity)
        {
            Ok(nonce) => nonce,
            Err(error) => {
                runtime.record_worker_ipc_audit(false, None, None, Some(&error.to_string()))?;
                return Err(error);
            }
        };
    let request: WorkerRequest =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_message(stream, MAX_REQUEST_BYTES))
            .await
            .map_err(|_| WorkerError::Protocol("worker request framing timed out".into()))
            .and_then(std::convert::identity)
        {
            Ok(request) => request,
            Err(error) => {
                runtime.record_worker_ipc_audit(false, None, None, Some(&error.to_string()))?;
                return Err(error);
            }
        };
    if let Err(error) = validate_request(key, &request, replay, &connection_nonce) {
        runtime.record_worker_ipc_audit(
            false,
            Some(&request.request_id),
            Some(operation_name(&request.operation)),
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    runtime.record_worker_ipc_audit(
        true,
        Some(&request.request_id),
        Some(operation_name(&request.operation)),
        None,
    )?;
    let request_id = request.request_id.clone();
    match request.operation {
        WorkerOperation::RunModel {
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let mut observer = IpcProviderObserver {
                stream,
                key,
                request_id: &request_id,
                sequence: 0,
            };
            let result = runtime
                .run_model_with_skills_stream(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    session_id.as_deref(),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            match result {
                Ok(result) => observer.complete(serde_json::to_value(result)?).await?,
                Err(error) => observer.error(error.to_string()).await?,
            }
            Ok(false)
        }
        operation => {
            let shutdown = matches!(operation, WorkerOperation::Shutdown);
            let result = dispatch(runtime, operation, maintenance).await;
            let succeeded = result.is_ok();
            let content = match result {
                Ok(result) => WorkerFrameContent::Complete { result },
                Err(error) => WorkerFrameContent::Error {
                    message: bounded_error(&error.to_string()),
                },
            };
            write_signed_frame(stream, key, &request_id, 1, content).await?;
            Ok(shutdown && succeeded)
        }
    }
}

fn operation_name(operation: &WorkerOperation) -> &'static str {
    match operation {
        WorkerOperation::Ping => "ping",
        WorkerOperation::AuditVerify => "audit_verify",
        WorkerOperation::AuditRead { .. } => "audit_read",
        WorkerOperation::PolicyDoctor => "policy_doctor",
        WorkerOperation::ProjectionStatus => "projection_status",
        WorkerOperation::ProjectionDrain => "projection_drain",
        WorkerOperation::ProjectionRebuild { .. } => "projection_rebuild",
        WorkerOperation::StateDoctor => "state_doctor",
        WorkerOperation::SandboxDoctor => "sandbox_doctor",
        WorkerOperation::ProviderProfiles => "provider_profiles",
        WorkerOperation::ProviderDoctor { .. } => "provider_doctor",
        WorkerOperation::ProviderModels { .. } => "provider_models",
        WorkerOperation::ProviderRoutes => "provider_routes",
        WorkerOperation::ToolsList => "tools_list",
        WorkerOperation::RunModel { .. } => "run_model",
        WorkerOperation::Echo { .. } => "echo",
        WorkerOperation::SessionCreate { .. } => "session_create",
        WorkerOperation::SessionGet { .. } => "session_get",
        WorkerOperation::SessionList { .. } => "session_list",
        WorkerOperation::SessionMessages { .. } => "session_messages",
        WorkerOperation::SessionLatest => "session_latest",
        WorkerOperation::ContextStatus { .. } => "context_status",
        WorkerOperation::ContextList { .. } => "context_list",
        WorkerOperation::ContextCompact { .. } => "context_compact",
        WorkerOperation::ContextRestore { .. } => "context_restore",
        WorkerOperation::TelemetryRuns { .. } => "telemetry_runs",
        WorkerOperation::TelemetryShow { .. } => "telemetry_show",
        WorkerOperation::TelemetryMetrics { .. } => "telemetry_metrics",
        WorkerOperation::WorkflowValidate { .. } => "workflow_validate",
        WorkerOperation::WorkflowRegister { .. } => "workflow_register",
        WorkerOperation::WorkflowList => "workflow_list",
        WorkerOperation::WorkflowShow { .. } => "workflow_show",
        WorkerOperation::WorkflowStart { .. } => "workflow_start",
        WorkerOperation::WorkflowStatus { .. } => "workflow_status",
        WorkerOperation::WorkflowResume { .. } => "workflow_resume",
        WorkerOperation::WorkflowInput { .. } => "workflow_input",
        WorkerOperation::WorkflowCancel { .. } => "workflow_cancel",
        WorkerOperation::Drain => "drain",
        WorkerOperation::Shutdown => "shutdown",
    }
}

async fn client_handshake<S>(stream: &mut S, key: &[u8; 32]) -> Result<String, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut challenge = [0_u8; 32];
    getrandom::fill(&mut challenge).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let challenge = hex::encode(challenge);
    write_message(
        stream,
        &ClientHello {
            version: PROTOCOL_VERSION,
            challenge: challenge.clone(),
        },
        1024,
    )
    .await?;
    let hello: ServerHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge != challenge
        || hello.server_nonce.len() != 64
        || hex::decode(&hello.server_nonce).map_or(true, |bytes| bytes.len() != 32)
        || (now_ms() - hello.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "worker server handshake version, challenge, or timestamp is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedServerHello {
            version: hello.version,
            challenge: &hello.challenge,
            server_nonce: &hello.server_nonce,
            timestamp_ms: hello.timestamp_ms,
        },
        &hello.authentication_tag,
    )?;
    Ok(hello.server_nonce)
}

async fn server_handshake<S>(stream: &mut S, key: &[u8; 32]) -> Result<String, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello: ClientHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge.len() != 64
        || hex::decode(&hello.challenge).map_or(true, |bytes| bytes.len() != 32)
    {
        return Err(WorkerError::Protocol(
            "worker client handshake version or challenge is invalid".into(),
        ));
    }
    let mut server_nonce = [0_u8; 32];
    getrandom::fill(&mut server_nonce).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let server_nonce = hex::encode(server_nonce);
    let timestamp_ms = now_ms();
    let authentication_tag = request_tag(
        key,
        &UnsignedServerHello {
            version: PROTOCOL_VERSION,
            challenge: &hello.challenge,
            server_nonce: &server_nonce,
            timestamp_ms,
        },
    )?;
    write_message(
        stream,
        &ServerHello {
            version: PROTOCOL_VERSION,
            challenge: hello.challenge,
            server_nonce: server_nonce.clone(),
            timestamp_ms,
            authentication_tag,
        },
        1024,
    )
    .await?;
    Ok(server_nonce)
}

async fn dispatch(
    runtime: &Runtime,
    operation: WorkerOperation,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<Value, WorkerError> {
    match operation {
        WorkerOperation::Ping => Ok(json!({
            "ready": true,
            "protocol_version": PROTOCOL_VERSION,
            "pid": std::process::id(),
        })),
        WorkerOperation::AuditVerify => Ok(serde_json::to_value(runtime.journal().verify()?)?),
        WorkerOperation::AuditRead { from, limit } => Ok(serde_json::to_value(
            runtime
                .journal()
                .read_global(from.max(1), limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::PolicyDoctor => Ok(runtime.policy_doctor().await?),
        WorkerOperation::ProjectionStatus => {
            Ok(serde_json::to_value(runtime.projection_status()?)?)
        }
        WorkerOperation::ProjectionDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_projections()?)?)
        }
        WorkerOperation::ProjectionRebuild { name } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime.rebuild_projection(name.as_deref())?,
            )?)
        }
        WorkerOperation::StateDoctor => Ok(runtime.state_doctor()?),
        WorkerOperation::SandboxDoctor => Ok(serde_json::to_value(runtime.sandbox_doctor())?),
        WorkerOperation::ProviderProfiles => Ok(serde_json::to_value(runtime.provider_profiles())?),
        WorkerOperation::ProviderDoctor { profile } => Ok(serde_json::to_value(
            runtime.provider_doctor(profile.as_deref()).await?,
        )?),
        WorkerOperation::ProviderModels { profile } => Ok(serde_json::to_value(
            runtime.provider_models(profile.as_deref()).await?,
        )?),
        WorkerOperation::ProviderRoutes => Ok(runtime.provider_routes()),
        WorkerOperation::ToolsList => Ok(serde_json::to_value(runtime.tool_specs())?),
        WorkerOperation::Echo { message } => {
            let result = runtime.echo(&message).await?;
            Ok(json!({
                "media_type": result.media_type,
                "bytes_base64": BASE64.encode(result.bytes),
            }))
        }
        WorkerOperation::SessionCreate { title } => Ok(serde_json::to_value(
            runtime.create_session(title.as_deref())?,
        )?),
        WorkerOperation::SessionGet { session_id } => {
            Ok(serde_json::to_value(runtime.get_session(&session_id)?)?)
        }
        WorkerOperation::SessionList { limit } => Ok(serde_json::to_value(
            runtime.list_sessions(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::SessionMessages { session_id } => Ok(serde_json::to_value(
            runtime.session_messages(&session_id)?,
        )?),
        WorkerOperation::SessionLatest => Ok(serde_json::to_value(runtime.latest_session()?)?),
        WorkerOperation::ContextStatus { session_id } => {
            Ok(serde_json::to_value(runtime.context_status(&session_id)?)?)
        }
        WorkerOperation::ContextList { session_id } => Ok(serde_json::to_value(
            runtime.context_snapshots(&session_id)?,
        )?),
        WorkerOperation::ContextCompact { session_id } => Ok(serde_json::to_value(
            runtime.compact_context(&session_id).await?,
        )?),
        WorkerOperation::ContextRestore {
            session_id,
            snapshot_id,
        } => Ok(serde_json::to_value(
            runtime.restore_context(&session_id, &snapshot_id)?,
        )?),
        WorkerOperation::TelemetryRuns { session_id, limit } => Ok(serde_json::to_value(
            runtime.telemetry_runs(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::TelemetryShow {
            id_or_prefix,
            limit,
        } => Ok(serde_json::to_value(
            runtime.telemetry_run(&id_or_prefix, limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::TelemetryMetrics { session_id, limit } => Ok(serde_json::to_value(
            runtime.telemetry_metrics(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::WorkflowValidate { path } => {
            let validated = runtime.validate_workflow_path(path).await?;
            Ok(json!({
                "valid": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
            }))
        }
        WorkerOperation::WorkflowRegister { path } => {
            let provenance = format!("repo:{path}");
            let validated = runtime.register_workflow_path(path).await?;
            Ok(json!({
                "registered": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
                "provenance": provenance,
            }))
        }
        WorkerOperation::WorkflowList => {
            let definitions = runtime
                .journal()
                .read_global(1, usize::MAX)?
                .into_iter()
                .filter(|event| event.event_type.starts_with("workflow.definition."))
                .map(|event| {
                    json!({
                        "event_id": event.event_id,
                        "event_type": event.event_type,
                        "stream_id": event.stream_id,
                        "occurred_at": event.occurred_at,
                        "record_hash": event.record_hash,
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_value(definitions)?)
        }
        WorkerOperation::WorkflowShow { name, version } => {
            let (definition, content_hash) = runtime
                .workflow_repository()
                .definition(&name, &version)?
                .ok_or_else(|| {
                    WorkerError::Remote(format!("workflow {name}:{version} not found"))
                })?;
            Ok(json!({"definition": definition, "content_hash": content_hash}))
        }
        WorkerOperation::WorkflowStart {
            name,
            version,
            inputs_source,
        } => {
            let inputs = parse_json_source(runtime, &inputs_source).await?;
            Ok(serde_json::to_value(
                runtime
                    .workflows()
                    .start_run(&name, &version, inputs)
                    .await?,
            )?)
        }
        WorkerOperation::WorkflowStatus { run_id } => {
            Ok(serde_json::to_value(runtime.workflows().get_run(&run_id)?)?)
        }
        WorkerOperation::WorkflowResume { run_id } => Ok(serde_json::to_value(
            runtime.workflows().resume_run(&run_id).await?,
        )?),
        WorkerOperation::WorkflowInput {
            run_id,
            input_source,
        } => {
            let input = parse_json_source(runtime, &input_source).await?;
            Ok(serde_json::to_value(
                runtime.workflows().provide_input(&run_id, input).await?,
            )?)
        }
        WorkerOperation::WorkflowCancel { run_id } => Ok(serde_json::to_value(
            runtime.workflows().cancel_run(&run_id)?,
        )?),
        WorkerOperation::Drain => drain_once(runtime, maintenance).await,
        WorkerOperation::Shutdown => Ok(json!({"stopping": true})),
        WorkerOperation::RunModel { .. } => Err(WorkerError::Protocol(
            "run_model must use the streaming dispatch path".into(),
        )),
    }
}

async fn drain_once(
    runtime: &Runtime,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<Value, WorkerError> {
    let _guard = maintenance.lock().await;
    let workflows = runtime.workflows().drain().await?;
    let projections = runtime.drain_projections()?;
    let subagents = runtime.drain_subagents().await?;
    Ok(json!({
        "workflows": workflows,
        "projections": projections,
        "subagents": subagents,
    }))
}

async fn parse_json_source(runtime: &Runtime, source: &str) -> Result<Value, WorkerError> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.into()
    };
    serde_json::from_str(&document)
        .map_err(|error| WorkerError::Protocol(format!("invalid JSON input: {error}")))
}

struct IpcProviderObserver<'a, S> {
    stream: &'a mut S,
    key: &'a [u8; 32],
    request_id: &'a str,
    sequence: u64,
}

impl<S> IpcProviderObserver<'_, S>
where
    S: AsyncWrite + Unpin + Send,
{
    async fn complete(&mut self, result: Value) -> Result<(), WorkerError> {
        self.send(WorkerFrameContent::Complete { result }).await
    }

    async fn error(&mut self, message: String) -> Result<(), WorkerError> {
        self.send(WorkerFrameContent::Error {
            message: bounded_error(&message),
        })
        .await
    }

    async fn send(&mut self, content: WorkerFrameContent) -> Result<(), WorkerError> {
        self.sequence = self.sequence.saturating_add(1);
        write_signed_frame(
            self.stream,
            self.key,
            self.request_id,
            self.sequence,
            content,
        )
        .await
    }
}

#[async_trait]
impl<S> colossus_ports::ProviderEventObserver for IpcProviderObserver<'_, S>
where
    S: AsyncWrite + Unpin + Send,
{
    async fn observe(
        &mut self,
        event: ProviderEvent,
    ) -> Result<(), colossus_ports::ModelProviderError> {
        self.send(WorkerFrameContent::Event { event })
            .await
            .map_err(|error| colossus_ports::ModelProviderError::Failed(error.to_string()))
    }
}

fn signed_request(
    key: &[u8; 32],
    operation: WorkerOperation,
    connection_nonce: &str,
) -> Result<WorkerRequest, WorkerError> {
    let request_id = Uuid::now_v7().to_string();
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let nonce = hex::encode(nonce);
    let timestamp_ms = now_ms();
    let tag = request_tag(
        key,
        &UnsignedRequest {
            version: PROTOCOL_VERSION,
            request_id: &request_id,
            timestamp_ms,
            nonce: &nonce,
            connection_nonce,
            operation: &operation,
        },
    )?;
    Ok(WorkerRequest {
        version: PROTOCOL_VERSION,
        request_id,
        timestamp_ms,
        nonce,
        connection_nonce: connection_nonce.into(),
        operation,
        authentication_tag: tag,
    })
}

fn validate_request(
    key: &[u8; 32],
    request: &WorkerRequest,
    replay: &Mutex<ReplayGuard>,
    connection_nonce: &str,
) -> Result<(), WorkerError> {
    if request.version != PROTOCOL_VERSION
        || request.request_id.is_empty()
        || request.request_id.len() > 128
        || request.connection_nonce != connection_nonce
        || (now_ms() - request.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "unsupported version, invalid id, or expired timestamp".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedRequest {
            version: request.version,
            request_id: &request.request_id,
            timestamp_ms: request.timestamp_ms,
            nonce: &request.nonce,
            connection_nonce: &request.connection_nonce,
            operation: &request.operation,
        },
        &request.authentication_tag,
    )?;
    replay
        .lock()
        .map_err(|error| WorkerError::Protocol(error.to_string()))?
        .accept(&request.nonce)
}

fn validate_frame(
    key: &[u8; 32],
    request_id: &str,
    sequence: &mut u64,
    frame: &WorkerFrame,
) -> Result<(), WorkerError> {
    let expected_sequence = sequence.saturating_add(1);
    if frame.version != PROTOCOL_VERSION
        || frame.request_id != request_id
        || frame.sequence != expected_sequence
        || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "response version, request id, sequence, or timestamp is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedFrame {
            version: frame.version,
            request_id: &frame.request_id,
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            content: &frame.content,
        },
        &frame.authentication_tag,
    )?;
    *sequence = expected_sequence;
    Ok(())
}

async fn write_signed_frame<S>(
    stream: &mut S,
    key: &[u8; 32],
    request_id: &str,
    sequence: u64,
    content: WorkerFrameContent,
) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
{
    let timestamp_ms = now_ms();
    let authentication_tag = request_tag(
        key,
        &UnsignedFrame {
            version: PROTOCOL_VERSION,
            request_id,
            sequence,
            timestamp_ms,
            content: &content,
        },
    )?;
    write_message(
        stream,
        &WorkerFrame {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            sequence,
            timestamp_ms,
            content,
            authentication_tag,
        },
        MAX_FRAME_BYTES,
    )
    .await
}

fn request_tag<T: Serialize>(key: &[u8; 32], value: &T) -> Result<String, WorkerError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn verify_tag<T: Serialize>(key: &[u8; 32], value: &T, tag: &str) -> Result<(), WorkerError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let tag = hex::decode(tag)
        .map_err(|_| WorkerError::Protocol("authentication tag is not hexadecimal".into()))?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    mac.verify_slice(&tag)
        .map_err(|_| WorkerError::Protocol("authentication tag mismatch".into()))
}

async fn write_message<S, T>(stream: &mut S, value: &T, limit: usize) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes =
        serde_json::to_vec(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    if bytes.len() > limit || bytes.len() > u32::MAX as usize {
        return Err(WorkerError::Protocol("IPC message exceeds bound".into()));
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_message<S, T>(stream: &mut S, limit: usize) -> Result<T, WorkerError>
where
    S: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > limit {
        return Err(WorkerError::Protocol(
            "IPC message length is empty or exceeds bound".into(),
        ));
    }
    let mut bytes = vec![0_u8; length];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|error| WorkerError::Protocol(error.to_string()))
}

fn now_ms() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

fn bounded_error(message: &str) -> String {
    message.chars().take(4_096).collect()
}

#[cfg(unix)]
mod platform {
    use super::WorkerError;
    use std::{
        fs,
        os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _},
        path::Path,
    };
    use tokio::net::{UnixListener, UnixStream};

    pub type ClientStream = UnixStream;
    pub type ServerStream = UnixStream;

    pub struct Listener {
        inner: UnixListener,
        endpoint: String,
    }

    impl Listener {
        pub async fn bind(endpoint: &str) -> Result<Self, WorkerError> {
            let path = Path::new(endpoint);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if !metadata.file_type().is_socket() {
                    return Err(WorkerError::Protocol(format!(
                        "worker endpoint exists and is not a socket: {endpoint}"
                    )));
                }
                if UnixStream::connect(path).await.is_ok() {
                    return Err(WorkerError::Protocol(format!(
                        "worker endpoint is already active: {endpoint}"
                    )));
                }
                fs::remove_file(path)?;
            }
            let inner = UnixListener::bind(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(Self {
                inner,
                endpoint: endpoint.into(),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerStream, WorkerError> {
            self.inner
                .accept()
                .await
                .map(|(stream, _)| stream)
                .map_err(Into::into)
        }

        pub fn cleanup(&mut self) {
            let _ = fs::remove_file(&self.endpoint);
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    pub async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        UnixStream::connect(endpoint).await
    }

    pub fn endpoint_is_trusted(endpoint: &str) -> Result<bool, WorkerError> {
        let path = Path::new(endpoint);
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let Some(parent) = path.parent() else {
            return Err(WorkerError::Protocol(
                "worker endpoint has no parent directory".into(),
            ));
        };
        let parent = fs::metadata(parent)?;
        if !metadata.file_type().is_socket()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != parent.uid()
        {
            return Err(WorkerError::Protocol(
                "worker endpoint is not an owner-only socket in its owning directory".into(),
            ));
        }
        Ok(true)
    }
}

#[cfg(windows)]
mod platform {
    use super::WorkerError;
    use std::{io::ErrorKind, time::Duration};
    use tokio::{
        net::windows::named_pipe::{
            ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
        },
        time::{Instant, sleep},
    };

    pub type ClientStream = NamedPipeClient;
    pub type ServerStream = NamedPipeServer;

    pub struct Listener {
        endpoint: String,
        next: Option<NamedPipeServer>,
    }

    impl Listener {
        pub async fn bind(endpoint: &str) -> Result<Self, WorkerError> {
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .create(endpoint)?;
            Ok(Self {
                endpoint: endpoint.into(),
                next: Some(next),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerStream, WorkerError> {
            let server = self
                .next
                .take()
                .ok_or_else(|| WorkerError::Protocol("named pipe listener lost instance".into()))?;
            server.connect().await?;
            self.next = Some(ServerOptions::new().create(&self.endpoint)?);
            Ok(server)
        }

        pub fn cleanup(&mut self) {}
    }

    pub async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        let deadline = Instant::now() + Duration::from_millis(450);
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(client) => return Ok(client),
                Err(error)
                    if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::WouldBlock)
                        && Instant::now() < deadline =>
                {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn endpoint_is_trusted(_endpoint: &str) -> Result<bool, WorkerError> {
        Ok(true)
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("colossus-worker supports only Unix sockets or Windows named pipes");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_detects_tampering_and_replay() {
        let key = [7_u8; 32];
        let mut request =
            signed_request(&key, WorkerOperation::Ping, "connection-one").expect("request");
        request.operation = WorkerOperation::Echo {
            message: "tampered".into(),
        };
        let replay = Mutex::new(ReplayGuard::default());
        assert!(matches!(
            validate_request(&key, &request, &replay, "connection-one"),
            Err(WorkerError::Protocol(_))
        ));

        let request =
            signed_request(&key, WorkerOperation::Ping, "connection-two").expect("request");
        validate_request(&key, &request, &replay, "connection-two").expect("first request");
        assert!(matches!(
            validate_request(&key, &request, &replay, "connection-two"),
            Err(WorkerError::Protocol(message)) if message.contains("replayed")
        ));
    }

    #[tokio::test]
    async fn framing_rejects_oversized_lengths_before_allocation() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let task = tokio::spawn(async move {
            writer
                .write_u32((MAX_REQUEST_BYTES + 1) as u32)
                .await
                .expect("length");
        });
        let result = read_message::<_, WorkerRequest>(&mut reader, MAX_REQUEST_BYTES).await;
        assert!(matches!(result, Err(WorkerError::Protocol(_))));
        task.await.expect("writer");
    }

    #[tokio::test]
    async fn client_discloses_no_operation_to_an_unauthenticated_server() {
        let expected_key = [8_u8; 32];
        let fake_key = [9_u8; 32];
        let (mut client, mut server) = tokio::io::duplex(4_096);
        let fake = tokio::spawn(async move {
            let hello: ClientHello = read_message(&mut server, 1024).await.expect("hello");
            let server_nonce = hex::encode([3_u8; 32]);
            let timestamp_ms = now_ms();
            let authentication_tag = request_tag(
                &fake_key,
                &UnsignedServerHello {
                    version: PROTOCOL_VERSION,
                    challenge: &hello.challenge,
                    server_nonce: &server_nonce,
                    timestamp_ms,
                },
            )
            .expect("tag");
            write_message(
                &mut server,
                &ServerHello {
                    version: PROTOCOL_VERSION,
                    challenge: hello.challenge,
                    server_nonce,
                    timestamp_ms,
                    authentication_tag,
                },
                1024,
            )
            .await
            .expect("server hello");
            read_message::<_, WorkerRequest>(&mut server, MAX_REQUEST_BYTES).await
        });
        assert!(matches!(
            client_handshake(&mut client, &expected_key).await,
            Err(WorkerError::Protocol(_))
        ));
        drop(client);
        assert!(matches!(
            fake.await.expect("fake server"),
            Err(WorkerError::Io(_))
        ));
    }
}
