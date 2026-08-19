use super::*;

const DEFAULT_POLICY_INPUT_LIMIT: usize = 1024 * 1024;
const DEFAULT_POST_EFFECT_POLICY_INPUT_LIMIT: usize = 8 * 1024 * 1024;
pub(super) const PERMIT_LIFETIME_MS: i128 = 30_000;

/// Minimum timeout that leaves the OCI helper enough time to confirm container cleanup.
pub const MIN_OCI_EFFECT_TIMEOUT_MS: u64 = 5_000;
/// Minimum timeout for OCI jobs that must also create and remove proxy networks.
pub const MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS: u64 = 10_000;
/// Minimum timeout that leaves Windows enough time to confirm Job Object cleanup.
pub const MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS: u64 = 10_000;

pub(super) type HmacSha256 = Hmac<Sha256>;

pub(super) fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, GatewayError> {
    serde_json::to_vec(value).map_err(|error| GatewayError::Contract(error.to_string()))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn now_unix_ms() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

pub(super) fn approval_proof(
    request_hash: &str,
    approved_by: impl Into<String>,
) -> Result<ApprovalProof, PolicyError> {
    let approved_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
    Ok(ApprovalProof {
        approval_id: Uuid::now_v7().to_string(),
        request_hash: request_hash.into(),
        approved_by: approved_by.into(),
        approved_at,
    })
}

/// Effect gateway failure. Denied content is deliberately absent.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// The safety kernel rejected the request or policy obligations.
    #[error("safety kernel rejected request: {0}")]
    Safety(String),
    /// The policy denied execution or content release.
    #[error("policy denied effect: {0}")]
    Denied(String),
    /// Approval was required but not granted or did not authorize the request.
    #[error("effect was not approved: {0}")]
    Approval(String),
    /// Policy transport or response failure; always fail closed.
    #[error(transparent)]
    Policy(#[from] PolicyError),
    /// Audit durability failed; no effect may continue.
    #[error(transparent)]
    Journal(#[from] StoreError),
    /// Adapter reported a known failure.
    #[error("effect failed: {0}")]
    Execution(String),
    /// Adapter rejected output in a way that permits a bounded application correction.
    #[error("recoverable effect failure {code}: {message}")]
    RecoverableExecution {
        /// Stable application-neutral code.
        code: String,
        /// Bounded safe diagnostic.
        message: String,
        /// HTTP response status when the adapter supplied one.
        http_status: Option<u16>,
        /// Bounded provider retry lower bound when supplied.
        retry_after_ms: Option<u64>,
    },
    /// Adapter reported a known non-success HTTP response.
    #[error("effect failed: {message}")]
    HttpStatus {
        /// HTTP response status.
        status: u16,
        /// Bounded safe diagnostic without response headers or body.
        message: String,
    },
    /// Adapter outcome is unknown and must not be retried implicitly.
    #[error("effect outcome is unknown: {0}")]
    OutcomeUnknown(String),
    /// Internal strict-contract serialization failed.
    #[error("contract failure: {0}")]
    Contract(String),
}

/// Adapter execution failure classification.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Adapter knows the effect failed.
    #[error("{0}")]
    Failed(String),
    /// Adapter output was rejected but a corrected request may safely be attempted.
    #[error("{code}: {message}")]
    Recoverable {
        /// Stable application-neutral code.
        code: String,
        /// Bounded safe diagnostic.
        message: String,
        /// HTTP response status when the adapter supplied one.
        http_status: Option<u16>,
        /// Bounded provider retry lower bound when supplied.
        retry_after_ms: Option<u64>,
    },
    /// Adapter returned a known non-success HTTP response.
    #[error("{message}")]
    HttpStatus {
        /// HTTP response status.
        status: u16,
        /// Bounded safe diagnostic without response headers or body.
        message: String,
    },
    /// Adapter cannot prove whether the external effect occurred.
    #[error("{0}")]
    OutcomeUnknown(String),
    /// Gateway post-effect policy rejected one streamed result before observation.
    #[error("stream release denied: {0}")]
    ReleaseDenied(String),
}

/// Output released only after all required policy decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleasedEffectResult {
    /// Media type supplied by the adapter.
    pub media_type: String,
    /// Bounded bytes released to the requester.
    pub bytes: Vec<u8>,
}

#[derive(Serialize)]
pub(super) struct PermitClaims<'a> {
    pub(super) request_hash: &'a str,
    pub(super) decision_id: &'a str,
    pub(super) obligations_hash: &'a str,
    pub(super) actor_id: &'a str,
    pub(super) nonce: &'a str,
    pub(super) expires_at_unix_ms: i128,
}

/// Opaque, authenticated, one-use execution permit.
///
/// External crates can receive this type but cannot construct or clone it.
///
/// ```compile_fail
/// use colossus_policy::ExecutionPermit;
/// let _forged = ExecutionPermit {
///     request_hash: String::new(),
///     decision_id: String::new(),
///     obligations_hash: String::new(),
///     actor_id: String::new(),
///     nonce: String::new(),
///     expires_at_unix_ms: 0,
///     authentication_tag: Vec::new(),
///     consumed: std::sync::atomic::AtomicBool::new(false),
/// };
/// ```
pub struct ExecutionPermit {
    pub(super) request_hash: String,
    pub(super) decision_id: String,
    pub(super) obligations_hash: String,
    pub(super) actor_id: String,
    pub(super) nonce: String,
    pub(super) expires_at_unix_ms: i128,
    pub(super) authentication_tag: Vec<u8>,
    pub(super) obligations: PolicyObligations,
    pub(super) consumed: AtomicBool,
}

impl ExecutionPermit {
    /// Canonical request hash authenticated by this permit.
    pub fn request_hash(&self) -> &str {
        &self.request_hash
    }

    /// Decision that authorized this permit.
    pub fn decision_id(&self) -> &str {
        &self.decision_id
    }

    /// Nonce used for IPC replay protection.
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// Permit expiration as Unix milliseconds.
    pub fn expires_at_unix_ms(&self) -> i128 {
        self.expires_at_unix_ms
    }

    /// Obligations the receiving adapter must enforce inside its execution boundary.
    pub fn obligations(&self) -> &PolicyObligations {
        &self.obligations
    }
}

/// Effectful adapter boundary. A caller cannot invoke it without an opaque permit.
#[async_trait]
pub trait EffectExecutor: Send + Sync {
    /// Execute into quarantine. The permit is non-cloneable and already authenticated.
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError>;
}

/// Receives bounded adapter chunks that have not yet crossed post-effect policy.
#[async_trait]
pub trait QuarantinedEffectObserver: Send {
    /// Submit one normalized chunk for policy-controlled release.
    async fn observe(&mut self, result: QuarantinedEffectResult) -> Result<(), ExecutionError>;
}

/// Effectful adapter capable of producing ordered normalized chunks.
#[async_trait]
pub trait StreamingEffectExecutor: Send + Sync {
    /// Execute with one permit and submit every externally observable chunk to the sink.
    /// The returned terminal result must exactly match the last submitted chunk.
    async fn execute_stream(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError>;
}

/// Observer that can receive only results released by the effect gateway.
#[async_trait]
pub trait ReleasedEffectObserver: Send {
    /// Observe one ordered, bounded, post-authorized result.
    async fn observe(&mut self, result: ReleasedEffectResult) -> Result<(), ExecutionError>;
}

const MAX_SANDBOX_BOUNDARY_SESSION_ACKNOWLEDGEMENTS: usize = 4_096;

tokio::task_local! {
    static ACTIVE_SANDBOX_BOUNDARY_ACKNOWLEDGEMENT: Option<String>;
}

/// Scope one worker-issued acknowledgement to the current interactive operation.
///
/// Task-local state deliberately does not propagate into detached tasks, so process
/// effects outside the attached operation continue to fail closed.
pub async fn with_sandbox_boundary_acknowledgement<F>(
    acknowledgement: Option<String>,
    future: F,
) -> F::Output
where
    F: Future,
{
    ACTIVE_SANDBOX_BOUNDARY_ACKNOWLEDGEMENT
        .scope(acknowledgement, future)
        .await
}

#[derive(Clone)]
struct ScopedSandboxBoundaryAcknowledgement {
    session_id: String,
    mode: SandboxBoundaryMode,
}

/// Process-local acknowledgement state for backends that do not isolate processes.
pub struct SandboxBoundaryGate {
    mode: Option<SandboxBoundaryMode>,
    globally_acknowledged: bool,
    acknowledged_sessions: RwLock<BTreeSet<String>>,
    scoped_acknowledgements: RwLock<BTreeMap<String, ScopedSandboxBoundaryAcknowledgement>>,
}

impl SandboxBoundaryGate {
    /// Construct the gate for the configured backend and optional headless acknowledgement.
    pub fn new(mode: Option<SandboxBoundaryMode>, globally_acknowledged: bool) -> Self {
        Self {
            mode,
            globally_acknowledged,
            acknowledged_sessions: RwLock::new(BTreeSet::new()),
            scoped_acknowledgements: RwLock::new(BTreeMap::new()),
        }
    }

    /// Direct-execution mode configured for this runtime, if any.
    pub const fn mode(&self) -> Option<SandboxBoundaryMode> {
        self.mode
    }

    /// Whether configuration explicitly acknowledged this boundary for headless callers.
    pub const fn globally_acknowledged(&self) -> bool {
        self.globally_acknowledged
    }

    /// Whether this session still needs an interactive acknowledgement.
    pub fn pending_for_session(&self, session_id: &str) -> Option<SandboxBoundaryMode> {
        let mode = self.mode?;
        if self.globally_acknowledged
            || self
                .acknowledged_sessions
                .read()
                .is_ok_and(|sessions| sessions.contains(session_id))
        {
            None
        } else {
            Some(mode)
        }
    }

    /// Acknowledge the configured direct-execution mode for one process-local session.
    pub fn acknowledge_session(
        &self,
        session_id: &str,
        mode: SandboxBoundaryMode,
    ) -> Result<(), GatewayError> {
        if session_id.is_empty() {
            return Err(GatewayError::Safety(
                "sandbox boundary acknowledgement requires a nonempty session id".into(),
            ));
        }
        if self.mode != Some(mode) {
            return Err(GatewayError::Safety(format!(
                "cannot acknowledge {} when that sandbox backend is not configured",
                mode.as_backend()
            )));
        }
        let mut sessions = self.acknowledged_sessions.write().map_err(|_| {
            GatewayError::Safety("sandbox boundary acknowledgement lock is poisoned".into())
        })?;
        if !sessions.contains(session_id)
            && sessions.len() >= MAX_SANDBOX_BOUNDARY_SESSION_ACKNOWLEDGEMENTS
        {
            return Err(GatewayError::Safety(
                "sandbox boundary acknowledgement capacity is exhausted; restart the runtime"
                    .into(),
            ));
        }
        sessions.insert(session_id.into());
        Ok(())
    }

    /// Roll back a just-recorded acknowledgement when durable audit append fails.
    pub fn revoke_session_acknowledgement(&self, session_id: &str) {
        if let Ok(mut sessions) = self.acknowledged_sessions.write() {
            sessions.remove(session_id);
        }
    }

    /// Register an opaque acknowledgement issued to one attached interactive client.
    pub fn acknowledge_interactive_client(
        &self,
        acknowledgement: &str,
        session_id: &str,
        mode: SandboxBoundaryMode,
    ) -> Result<(), GatewayError> {
        if acknowledgement.len() != 64
            || !acknowledgement
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || session_id.is_empty()
        {
            return Err(GatewayError::Safety(
                "interactive sandbox boundary acknowledgement requires an exact opaque capability and nonempty session id"
                    .into(),
            ));
        }
        if self.mode != Some(mode) {
            return Err(GatewayError::Safety(format!(
                "cannot acknowledge {} when that sandbox backend is not configured",
                mode.as_backend()
            )));
        }
        let mut acknowledgements = self.scoped_acknowledgements.write().map_err(|_| {
            GatewayError::Safety(
                "interactive sandbox boundary acknowledgement lock is poisoned".into(),
            )
        })?;
        if acknowledgements.contains_key(acknowledgement) {
            return Err(GatewayError::Safety(
                "interactive sandbox boundary acknowledgement was replayed".into(),
            ));
        }
        if acknowledgements.len() >= MAX_SANDBOX_BOUNDARY_SESSION_ACKNOWLEDGEMENTS {
            return Err(GatewayError::Safety(
                "interactive sandbox boundary acknowledgement capacity is exhausted; restart the runtime"
                    .into(),
            ));
        }
        acknowledgements.insert(
            acknowledgement.into(),
            ScopedSandboxBoundaryAcknowledgement {
                session_id: session_id.into(),
                mode,
            },
        );
        Ok(())
    }

    /// Roll back a worker-issued acknowledgement when durable audit append fails.
    pub fn revoke_interactive_client_acknowledgement(&self, acknowledgement: &str) {
        if let Ok(mut acknowledgements) = self.scoped_acknowledgements.write() {
            acknowledgements.remove(acknowledgement);
        }
    }

    /// Configured boundary when this session already accepted it, otherwise `None`.
    pub fn acknowledged_mode(&self, session_id: Option<&str>) -> Option<SandboxBoundaryMode> {
        let mode = self.mode?;
        self.acknowledged(session_id, mode).then_some(mode)
    }

    fn acknowledged(&self, session_id: Option<&str>, mode: SandboxBoundaryMode) -> bool {
        if self.globally_acknowledged {
            return true;
        }
        session_id.is_some_and(|session_id| {
            self.acknowledged_sessions
                .read()
                .is_ok_and(|sessions| sessions.contains(session_id))
                || self.active_scoped_acknowledgement_matches(session_id, mode)
        })
    }

    fn active_scoped_acknowledgement_matches(
        &self,
        session_id: &str,
        mode: SandboxBoundaryMode,
    ) -> bool {
        ACTIVE_SANDBOX_BOUNDARY_ACKNOWLEDGEMENT
            .try_with(|active| {
                active.as_deref().is_some_and(|acknowledgement| {
                    self.scoped_acknowledgements
                        .read()
                        .is_ok_and(|acknowledgements| {
                            acknowledgements.get(acknowledgement).is_some_and(|entry| {
                                entry.session_id == session_id && entry.mode == mode
                            })
                        })
                })
            })
            .unwrap_or(false)
    }

    fn validate(
        &self,
        request: &EffectRequest,
        mode: SandboxBoundaryMode,
    ) -> Result<(), GatewayError> {
        if self.mode != Some(mode) {
            return Err(GatewayError::Safety(format!(
                "policy selected {} but the runtime was not configured for that direct-execution boundary",
                mode.as_backend()
            )));
        }
        if self.acknowledged(request.context.session_id.as_deref(), mode) {
            return Ok(());
        }
        let requirement = match mode {
            SandboxBoundaryMode::External => {
                "set sandbox.acknowledgeExternalBoundary: true for an operator-managed headless runtime or acknowledge the external boundary in the TUI"
            }
            SandboxBoundaryMode::DangerFullAccess => {
                "set sandbox.acknowledgeDangerFullAccess: true for an operator-managed headless runtime or acknowledge danger full access in the TUI"
            }
        };
        Err(GatewayError::Safety(format!(
            "{} process execution is not acknowledged; {requirement}",
            mode.as_backend()
        )))
    }
}

/// Hard safety checks policy is never allowed to override.
pub struct SafetyKernel {
    known_capabilities: BTreeSet<String>,
    policy_input_limit: usize,
    post_effect_policy_input_limit: usize,
    sandbox_boundary_gate: Option<Arc<SandboxBoundaryGate>>,
}

impl SafetyKernel {
    /// Construct a kernel with signed/known capability identities.
    pub fn new(known_capabilities: impl IntoIterator<Item = String>) -> Self {
        Self {
            known_capabilities: known_capabilities.into_iter().collect(),
            policy_input_limit: DEFAULT_POLICY_INPUT_LIMIT,
            post_effect_policy_input_limit: DEFAULT_POST_EFFECT_POLICY_INPUT_LIMIT,
            sandbox_boundary_gate: None,
        }
    }

    /// Override the disclosure cap for bounded tests or stricter deployments.
    pub fn with_policy_input_limit(mut self, bytes: usize) -> Self {
        self.policy_input_limit = bytes;
        self.post_effect_policy_input_limit = bytes;
        self
    }

    /// Require runtime acknowledgement before direct-execution permits may be minted.
    pub fn with_sandbox_boundary_gate(mut self, gate: Arc<SandboxBoundaryGate>) -> Self {
        self.sandbox_boundary_gate = Some(gate);
        self
    }

    /// Direct-execution boundary configured for this kernel, when one is active.
    pub fn sandbox_boundary_mode(&self) -> Option<SandboxBoundaryMode> {
        self.sandbox_boundary_gate
            .as_deref()
            .and_then(SandboxBoundaryGate::mode)
    }

    /// Direct-execution boundary already acknowledged for one session, when one is active.
    pub fn acknowledged_sandbox_boundary_mode(
        &self,
        session_id: Option<&str>,
    ) -> Option<SandboxBoundaryMode> {
        self.sandbox_boundary_gate
            .as_deref()
            .and_then(|gate| gate.acknowledged_mode(session_id))
    }

    pub(super) fn prepare(&self, request: &EffectRequest) -> Result<EffectRequest, GatewayError> {
        if request.schema_version != 1 || request.request_id.is_empty() {
            return Err(GatewayError::Safety(
                "unsupported schema version or empty request id".into(),
            ));
        }
        if request.action.is_empty() || request.resource.is_empty() {
            return Err(GatewayError::Safety(
                "action and resource must be non-empty".into(),
            ));
        }
        for capability in &request.capabilities {
            if !self.known_capabilities.contains(capability) {
                return Err(GatewayError::Safety(format!(
                    "unknown or unsigned capability {capability}"
                )));
            }
        }
        let mut prepared = request.clone();
        redact_hard_secrets(&mut prepared.content);
        let size = canonical_bytes(&prepared)?.len();
        let limit = if prepared.phase == EffectPhase::PostEffect {
            self.post_effect_policy_input_limit
        } else {
            self.policy_input_limit
        };
        if size > limit {
            return Err(GatewayError::Policy(PolicyError::InputTooLarge { limit }));
        }
        Ok(prepared)
    }

    pub(super) fn validate_decision(
        &self,
        request: &EffectRequest,
        decision: &PolicyDecision,
    ) -> Result<(), GatewayError> {
        let obligations = &decision.obligations;
        if decision.decision_id.is_empty()
            || decision.policy_revision.is_empty()
            || decision.reason.is_empty()
            || obligations.sandbox_backend.is_empty()
            || obligations.sandbox_profile.is_empty()
            || obligations.timeout_ms == 0
            || obligations.max_output_bytes == 0
            || obligations.max_processes == 0
            || obligations.max_memory_bytes == 0
            || obligations.max_concurrency == 0
            || obligations.retention.is_empty()
        {
            return Err(GatewayError::Policy(PolicyError::InvalidDecision(
                "required decision field or obligation is absent/zero".into(),
            )));
        }
        if !matches!(
            obligations.sandbox_backend.as_str(),
            "broker" | "native" | "oci" | "windows_job" | "external" | "danger_full_access"
        ) {
            return Err(GatewayError::Safety(format!(
                "unknown sandbox backend {}",
                obligations.sandbox_backend
            )));
        }
        if obligations.resource_authority == ResourceAuthority::Ambient
            && obligations.sandbox_backend != SandboxBoundaryMode::DangerFullAccess.as_backend()
        {
            return Err(GatewayError::Safety(
                "ambient resource authority requires the danger_full_access sandbox backend".into(),
            ));
        }
        if is_process_effect(request)
            && obligations.sandbox_backend == SandboxBoundaryMode::DangerFullAccess.as_backend()
            && obligations.resource_authority != ResourceAuthority::Ambient
        {
            return Err(GatewayError::Safety(
                "danger_full_access process execution requires explicit ambient resource authority"
                    .into(),
            ));
        }
        if obligations.sandbox_backend == "broker"
            && is_process_effect(request)
            && !obligations.allow_sandbox_downgrade
        {
            return Err(GatewayError::Safety(
                "process execution cannot downgrade to the broker without an explicit obligation"
                    .into(),
            ));
        }
        if request.phase == EffectPhase::PreEffect && decision.outcome != DecisionOutcome::Deny {
            let boundary_mode = if obligations.resource_authority == ResourceAuthority::Ambient {
                Some(SandboxBoundaryMode::DangerFullAccess)
            } else if is_process_effect(request) {
                SandboxBoundaryMode::from_backend(&obligations.sandbox_backend)
            } else {
                None
            };
            if let Some(mode) = boundary_mode {
                self.sandbox_boundary_gate
                    .as_ref()
                    .ok_or_else(|| {
                        GatewayError::Safety(format!(
                            "{} execution requires a runtime acknowledgement gate",
                            mode.as_backend()
                        ))
                    })?
                    .validate(request, mode)?;
            }
        }
        if obligations.sandbox_backend == "windows_job"
            && is_process_effect(request)
            && !cfg!(target_os = "windows")
        {
            return Err(GatewayError::Safety(
                "windows_job process execution is available only on Windows".into(),
            ));
        }
        if obligations.sandbox_backend == "windows_job"
            && is_process_effect(request)
            && obligations.timeout_ms < MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS
        {
            return Err(GatewayError::Safety(format!(
                "Windows Job Object process execution requires timeout_ms >= {MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
            )));
        }
        if cfg!(target_os = "windows")
            && obligations.sandbox_backend == "oci"
            && is_process_effect(request)
        {
            return Err(GatewayError::Safety(
                "OCI process execution is disabled on Windows until path mapping passes live acceptance"
                    .into(),
            ));
        }
        if obligations.sandbox_backend == "oci"
            && is_process_effect(request)
            && obligations.timeout_ms < MIN_OCI_EFFECT_TIMEOUT_MS
        {
            return Err(GatewayError::Safety(format!(
                "OCI process execution requires timeout_ms >= {MIN_OCI_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
            )));
        }
        if obligations.sandbox_backend == "oci"
            && is_process_effect(request)
            && !obligations.network_destinations.is_empty()
            && obligations.timeout_ms < MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS
        {
            return Err(GatewayError::Safety(format!(
                "networked OCI process execution requires timeout_ms >= {MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS} so proxy cleanup can be confirmed"
            )));
        }
        let mut environment = BTreeSet::new();
        for name in &obligations.allowed_environment {
            if !valid_environment_name(name) || !environment.insert(name.as_str()) {
                return Err(GatewayError::Safety(
                    "environment obligations must be unique POSIX-style names".into(),
                ));
            }
        }
        for destination in &obligations.network_destinations {
            if destination != "*" && canonical_network_origin(destination)? != *destination {
                return Err(GatewayError::Safety(format!(
                    "network destination must be * or a canonical HTTP(S) origin: {destination}"
                )));
            }
        }
        for grant in &obligations.filesystem {
            if !absolute_policy_root(&grant.root)
                || !matches!(
                    grant.mode.as_str(),
                    "read" | "write" | "metadata" | "execute"
                )
            {
                return Err(GatewayError::Safety(
                    "filesystem obligations require absolute roots and known modes".into(),
                ));
            }
        }
        let mut protected = BTreeSet::new();
        for path in &obligations.protected_filesystem {
            if !absolute_policy_root(path) || !protected.insert(path.as_str()) {
                return Err(GatewayError::Safety(
                    "protected filesystem obligations require unique absolute paths".into(),
                ));
            }
            let canonical = fs::canonicalize(path).map_err(|error| {
                GatewayError::Safety(format!("protected filesystem path is unavailable: {error}"))
            })?;
            let covered_by_write = obligations.filesystem.iter().any(|grant| {
                grant.mode == "write"
                    && fs::canonicalize(&grant.root)
                        .is_ok_and(|root| canonical.starts_with(&root) && canonical != root)
            });
            if !covered_by_write {
                return Err(GatewayError::Safety(
                    "protected filesystem paths must be strict descendants of writable grants"
                        .into(),
                ));
            }
        }
        if decision.outcome == DecisionOutcome::Allow && is_filesystem_action(&request.action) {
            validate_filesystem_containment(request, obligations)?;
        }
        if decision.outcome == DecisionOutcome::Allow
            && request.phase == EffectPhase::PreEffect
            && request.action == "web.search"
            && !obligations.require_post_effect
        {
            return Err(GatewayError::Safety(
                "web.search requires mandatory post-effect authorization".into(),
            ));
        }
        if decision.outcome == DecisionOutcome::Allow
            && request.phase == EffectPhase::PreEffect
            && is_process_effect(request)
        {
            validate_process_obligations(request, obligations)?;
        }
        if decision.outcome == DecisionOutcome::Allow
            && (matches!(
                request.action.as_str(),
                "network.http"
                    | "web.search"
                    | "audit.export.worm.write"
                    | "provider.openai.responses"
                    | "provider.openai.codex"
                    | "provider.openai.chat"
                    | "provider.models"
                    | "registry.pull"
                    | "registry.push"
            ) || is_streamable_http_mcp(request))
        {
            let origin = canonical_network_origin(&request.resource)?;
            if http_transport_authority_match(obligations, &origin)?.is_none() {
                return Err(GatewayError::Safety(format!(
                    "network destination {origin} is not allowed"
                )));
            }
        }
        Ok(())
    }
}

pub(super) fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

pub(super) fn is_process_action(action: &str) -> bool {
    action.starts_with("pack.tool.")
        || action.starts_with("pack.mcp.")
        || matches!(
            action,
            "process.spawn"
                | "shell.run"
                | "git.status"
                | "git.diff"
                | "git.show"
                | "mcp.tools"
                | "mcp.call"
        )
}

fn is_process_effect(request: &EffectRequest) -> bool {
    is_process_action(&request.action) && !is_streamable_http_mcp(request)
}

fn is_streamable_http_mcp(request: &EffectRequest) -> bool {
    matches!(request.action.as_str(), "mcp.tools" | "mcp.call")
        && request.content.get("transport").and_then(Value::as_str) == Some("streamable_http")
}

pub(super) fn is_filesystem_action(action: &str) -> bool {
    action.starts_with("filesystem.")
        || action.starts_with("repo.")
        || matches!(
            action,
            "patch.preview" | "patch.apply" | "patch.reverse" | "trace.export"
        )
}

/// How one configured network grant authorized a canonical origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkDestinationMatch {
    /// Acknowledged ambient authority accepted the canonical HTTP(S) origin.
    Ambient,
    /// The canonical origin was configured exactly.
    Exact,
    /// The public HTTP(S) wildcard matched; DNS must still resolve only public addresses.
    PublicWildcard,
}

/// Canonicalize a credential-free HTTP(S) URL to its origin.
pub fn canonical_network_origin(resource: &str) -> Result<String, GatewayError> {
    let url = Url::parse(resource)
        .map_err(|error| GatewayError::Safety(format!("invalid network URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(GatewayError::Safety(
            "network URLs require HTTP(S), a host, and no embedded credentials".into(),
        ));
    }
    Ok(url.origin().ascii_serialization())
}

/// Match one HTTP(S) resource against exact origins or the public-only wildcard.
pub fn network_destination_match(
    destinations: &[String],
    resource: &str,
) -> Result<Option<NetworkDestinationMatch>, GatewayError> {
    let origin = canonical_network_origin(resource)?;
    if destinations.iter().any(|allowed| allowed == &origin) {
        return Ok(Some(NetworkDestinationMatch::Exact));
    }
    if !destinations.iter().any(|allowed| allowed == "*") {
        return Ok(None);
    }
    let url = Url::parse(&origin)
        .map_err(|error| GatewayError::Safety(format!("invalid network origin: {error}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| GatewayError::Safety("network origin has no host".into()))?;
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost"
        || matches!(
            lower.as_str(),
            "metadata.google.internal"
                | "metadata.goog"
                | "instance-data"
                | "instance-data.ec2.internal"
        )
        || colossus_network::parse_host_ip(host).is_some_and(non_public_network_address)
    {
        return Ok(None);
    }
    Ok(Some(NetworkDestinationMatch::PublicWildcard))
}

/// Match a canonical HTTP(S) resource under a decision's resource authority.
pub fn network_authority_match(
    obligations: &PolicyObligations,
    resource: &str,
) -> Result<Option<NetworkDestinationMatch>, GatewayError> {
    canonical_network_origin(resource)?;
    if obligations.resource_authority == ResourceAuthority::Ambient {
        return Ok(Some(NetworkDestinationMatch::Ambient));
    }
    network_destination_match(&obligations.network_destinations, resource)
}

/// Match one permit-bound HTTP(S) transport, including the plaintext transport gate.
///
/// An exact configured origin does not authorize plaintext HTTP outside loopback. That
/// transport is available only when the execution permit carries acknowledged ambient
/// resource authority. Configuration may be validated before a session acknowledgement
/// exists, so effect adapters must apply this check from the permit they actually receive.
pub fn http_transport_authority_match(
    obligations: &PolicyObligations,
    resource: &str,
) -> Result<Option<NetworkDestinationMatch>, GatewayError> {
    let url = Url::parse(resource)
        .map_err(|error| GatewayError::Safety(format!("invalid network URL: {error}")))?;
    canonical_network_origin(resource)?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || colossus_network::parse_host_ip(host).is_some_and(|address| address.is_loopback())
    });
    if url.scheme() == "http"
        && !loopback
        && obligations.resource_authority != ResourceAuthority::Ambient
    {
        return Err(GatewayError::Safety(
            "non-loopback plaintext HTTP requires ambient resource authority in the execution permit"
                .into(),
        ));
    }
    network_authority_match(obligations, resource)
}

/// Return whether an address is outside public Internet routing.
pub fn non_public_network_address(ip: IpAddr) -> bool {
    colossus_network::non_public_network_address(ip)
}

pub(super) fn validate_process_obligations(
    request: &EffectRequest,
    obligations: &PolicyObligations,
) -> Result<(), GatewayError> {
    let danger_full_access =
        obligations.sandbox_backend == SandboxBoundaryMode::DangerFullAccess.as_backend();
    let executable_allowed = if danger_full_access {
        canonical_effect_path(&request.resource, false)?.is_file()
    } else if obligations.sandbox_backend == "oci" {
        normalized_absolute_path(&request.resource)
            && obligations
                .filesystem
                .iter()
                .any(|grant| grant.mode == "execute" && grant.root == request.resource)
    } else {
        let executable = canonical_effect_path(&request.resource, false)?;
        obligations.filesystem.iter().any(|grant| {
            grant.mode == "execute"
                && fs::canonicalize(&grant.root).is_ok_and(|root| executable == root)
        })
    };
    if !executable_allowed {
        return Err(GatewayError::Safety(format!(
            "executable {} is not explicitly granted",
            request.resource
        )));
    }
    let cwd = request
        .content
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::Safety("process cwd is absent".into()))?;
    let cwd = canonical_effect_path(cwd, false)?;
    if SandboxBoundaryMode::from_backend(&obligations.sandbox_backend).is_none() {
        let cwd_allowed = obligations.filesystem.iter().any(|grant| {
            matches!(grant.mode.as_str(), "read" | "write")
                && fs::canonicalize(&grant.root).is_ok_and(|root| cwd.starts_with(root))
        });
        if !cwd_allowed {
            return Err(GatewayError::Safety(
                "process cwd is outside allowed filesystem roots".into(),
            ));
        }
    }
    let environment = request
        .content
        .get("environment")
        .and_then(Value::as_object)
        .ok_or_else(|| GatewayError::Safety("process environment object is absent".into()))?;
    for name in environment.keys() {
        if !danger_full_access
            && !obligations
                .allowed_environment
                .iter()
                .any(|allowed| allowed == name)
        {
            return Err(GatewayError::Safety(format!(
                "environment variable {name} is not allowed"
            )));
        }
    }
    Ok(())
}

pub(super) fn normalized_absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

pub(super) fn validate_filesystem_containment(
    request: &EffectRequest,
    obligations: &PolicyObligations,
) -> Result<(), GatewayError> {
    let requested_mode = if request.action.contains("write")
        || matches!(request.action.as_str(), "patch.apply" | "patch.reverse")
        || request.action == "trace.export"
    {
        "write"
    } else if request.action.contains("metadata") {
        "metadata"
    } else {
        "read"
    };
    let target = canonical_effect_path(&request.resource, requested_mode == "write")?;
    if obligations.resource_authority == ResourceAuthority::Ambient {
        return Ok(());
    }
    let allowed = obligations.filesystem.iter().any(|grant| {
        let mode_allowed = grant.mode == "write"
            || grant.mode == requested_mode
            || (requested_mode == "metadata" && grant.mode == "read");
        mode_allowed && fs::canonicalize(&grant.root).is_ok_and(|root| target.starts_with(root))
    });
    if !allowed {
        return Err(GatewayError::Safety(format!(
            "{} is outside allowed {requested_mode} roots",
            request.resource
        )));
    }
    Ok(())
}

pub(super) fn canonical_effect_path(
    resource: &str,
    allow_missing_leaf: bool,
) -> Result<std::path::PathBuf, GatewayError> {
    match fs::canonicalize(resource) {
        Ok(path) => Ok(path),
        Err(error) if allow_missing_leaf => {
            let path = Path::new(resource);
            let parent = path.parent().ok_or_else(|| {
                GatewayError::Safety(format!("effect path has no parent: {resource}"))
            })?;
            let name = path.file_name().ok_or_else(|| {
                GatewayError::Safety(format!("effect path has no filename: {resource}"))
            })?;
            fs::canonicalize(parent)
                .map(|parent| parent.join(name))
                .map_err(|_| GatewayError::Safety(error.to_string()))
        }
        Err(error) => Err(GatewayError::Safety(error.to_string())),
    }
}

pub(super) fn absolute_policy_root(root: &str) -> bool {
    Path::new(root).is_absolute()
        || (root.len() >= 3
            && root.as_bytes()[0].is_ascii_alphabetic()
            && root.as_bytes()[1] == b':'
            && matches!(root.as_bytes()[2], b'\\' | b'/'))
}

pub(super) fn is_hard_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    matches!(
        key.as_str(),
        "authorization"
            | "proxy_authorization"
            | "api_key"
            | "apikey"
            | "access_token"
            | "refresh_token"
            | "private_key"
            | "client_secret"
            | "password"
            | "key_material"
            | "hidden_reasoning"
    )
}

/// Effect-content field holding the configured secret HTTP header references.
const CREDENTIAL_HEADERS_FIELD: &str = "credential_headers";

pub(super) fn redact_hard_secrets(value: &mut Value) {
    redact_effect_content(value, true);
}

fn redact_effect_content(value: &mut Value, at_content_root: bool) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if at_content_root && key == CREDENTIAL_HEADERS_FIELD {
                    redact_credential_headers(child);
                } else if is_hard_secret_key(key) && !is_environment_credential_reference(child) {
                    *child = redacted_placeholder(child);
                } else {
                    redact_effect_content(child, false);
                }
            }
        }
        Value::Array(array) => array
            .iter_mut()
            .for_each(|child| redact_effect_content(child, false)),
        _ => {}
    }
}

/// Preserve well-formed structured references only inside the configured
/// `credential_headers` map, so strict downstream input validation still accepts
/// the effect shape without granting a general exemption to hard-secret keys.
fn redact_credential_headers(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        redact_effect_content(value, false);
        return;
    };
    for child in object.values_mut() {
        if !is_environment_credential_header_reference(child) {
            *child = redacted_placeholder(child);
        }
    }
}

fn redacted_placeholder(value: &Value) -> Value {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    json!({
        "redacted": true,
        "sha256": sha256_hex(&bytes),
        "size": bytes.len()
    })
}

fn is_environment_credential_header_reference(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() > 2
        || !object.contains_key("reference")
        || object
            .keys()
            .any(|key| !matches!(key.as_str(), "reference" | "scheme"))
        || !object
            .get("reference")
            .is_some_and(is_environment_credential_reference)
    {
        return false;
    }
    object.get("scheme").is_none_or(|scheme| {
        scheme.is_null()
            || scheme.as_str().is_some_and(|scheme| {
                !scheme.is_empty()
                    && scheme.len() <= 64
                    && scheme
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
    })
}

pub(super) fn is_environment_credential_reference(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.strip_prefix("env:").is_some_and(|name| {
            let mut bytes = name.bytes();
            bytes
                .next()
                .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
                && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        })
    })
}

pub(super) fn disclosure_summary(request: &EffectRequest) -> Value {
    let fields = request
        .content
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let encoded = serde_json::to_vec(&request.content).unwrap_or_default();
    json!({
        "request_id": request.request_id,
        "phase": request.phase,
        "action": request.action,
        "resource": request.resource,
        "content_fields": fields,
        "content_size": encoded.len(),
        "content_hash": sha256_hex(&encoded),
        "credential_references": request.credential_references,
        "capabilities": request.capabilities,
    })
}
