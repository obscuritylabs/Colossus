//! Non-bypassable effect gateway, built-in policy, and OPA adapter.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, ApprovalProof, DecisionOutcome, EffectPhase, EffectRequest,
    EventClassification, NewEvent, PolicyDecision, PolicyObligations, QuarantinedEffectResult,
    RiskLevel, RiskRecommendation, RiskStatus,
};
use colossus_ports::{
    ApprovalProvider, EventJournal, PolicyDecisionPoint, PolicyError, RiskEvaluator, StoreError,
};
use hmac::{Hmac, Mac};
use reqwest::{Certificate, Client, Identity, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{
        Arc, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const DEFAULT_POLICY_INPUT_LIMIT: usize = 1024 * 1024;
const PERMIT_LIFETIME_MS: i128 = 30_000;

/// Minimum timeout that leaves the OCI helper enough time to confirm container cleanup.
pub const MIN_OCI_EFFECT_TIMEOUT_MS: u64 = 5_000;
/// Minimum timeout for OCI jobs that must also create and remove proxy networks.
pub const MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS: u64 = 10_000;
/// Minimum timeout that leaves Windows enough time to confirm Job Object cleanup.
pub const MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS: u64 = 10_000;

type HmacSha256 = Hmac<Sha256>;

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, GatewayError> {
    serde_json::to_vec(value).map_err(|error| GatewayError::Contract(error.to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn now_unix_ms() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

fn approval_proof(
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
struct PermitClaims<'a> {
    request_hash: &'a str,
    decision_id: &'a str,
    obligations_hash: &'a str,
    actor_id: &'a str,
    nonce: &'a str,
    expires_at_unix_ms: i128,
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
    request_hash: String,
    decision_id: String,
    obligations_hash: String,
    actor_id: String,
    nonce: String,
    expires_at_unix_ms: i128,
    authentication_tag: Vec<u8>,
    obligations: PolicyObligations,
    consumed: AtomicBool,
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

/// Hard safety checks policy is never allowed to override.
pub struct SafetyKernel {
    known_capabilities: BTreeSet<String>,
    policy_input_limit: usize,
}

impl SafetyKernel {
    /// Construct a kernel with signed/known capability identities.
    pub fn new(known_capabilities: impl IntoIterator<Item = String>) -> Self {
        Self {
            known_capabilities: known_capabilities.into_iter().collect(),
            policy_input_limit: DEFAULT_POLICY_INPUT_LIMIT,
        }
    }

    /// Override the disclosure cap for bounded tests or stricter deployments.
    pub fn with_policy_input_limit(mut self, bytes: usize) -> Self {
        self.policy_input_limit = bytes;
        self
    }

    fn prepare(&self, request: &EffectRequest) -> Result<EffectRequest, GatewayError> {
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
        if size > self.policy_input_limit {
            return Err(GatewayError::Policy(PolicyError::InputTooLarge {
                limit: self.policy_input_limit,
            }));
        }
        Ok(prepared)
    }

    fn validate_decision(
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
            "broker" | "native" | "oci" | "windows_job"
        ) {
            return Err(GatewayError::Safety(format!(
                "unknown sandbox backend {}",
                obligations.sandbox_backend
            )));
        }
        if obligations.sandbox_backend == "broker"
            && is_process_action(&request.action)
            && !obligations.allow_sandbox_downgrade
        {
            return Err(GatewayError::Safety(
                "process execution cannot downgrade to the broker without an explicit obligation"
                    .into(),
            ));
        }
        if obligations.sandbox_backend == "windows_job"
            && is_process_action(&request.action)
            && !cfg!(target_os = "windows")
        {
            return Err(GatewayError::Safety(
                "windows_job process execution is available only on Windows".into(),
            ));
        }
        if obligations.sandbox_backend == "windows_job"
            && is_process_action(&request.action)
            && obligations.timeout_ms < MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS
        {
            return Err(GatewayError::Safety(format!(
                "Windows Job Object process execution requires timeout_ms >= {MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
            )));
        }
        if cfg!(target_os = "windows")
            && obligations.sandbox_backend == "oci"
            && is_process_action(&request.action)
        {
            return Err(GatewayError::Safety(
                "OCI process execution is disabled on Windows until path mapping passes live acceptance"
                    .into(),
            ));
        }
        if obligations.sandbox_backend == "oci"
            && is_process_action(&request.action)
            && obligations.timeout_ms < MIN_OCI_EFFECT_TIMEOUT_MS
        {
            return Err(GatewayError::Safety(format!(
                "OCI process execution requires timeout_ms >= {MIN_OCI_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
            )));
        }
        if obligations.sandbox_backend == "oci"
            && is_process_action(&request.action)
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
            if canonical_network_origin(destination)? != *destination {
                return Err(GatewayError::Safety(format!(
                    "network destination must be a canonical HTTP(S) origin: {destination}"
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
            && is_process_action(&request.action)
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
                    | "provider.openai.chat"
                    | "provider.models"
                    | "registry.pull"
                    | "registry.push"
            ))
        {
            let origin = canonical_network_origin(&request.resource)?;
            if !obligations
                .network_destinations
                .iter()
                .any(|allowed| allowed == &origin)
            {
                return Err(GatewayError::Safety(format!(
                    "network destination {origin} is not allowed"
                )));
            }
        }
        Ok(())
    }
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn is_process_action(action: &str) -> bool {
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

fn is_filesystem_action(action: &str) -> bool {
    action.starts_with("filesystem.")
        || action.starts_with("repo.")
        || matches!(
            action,
            "patch.preview" | "patch.apply" | "patch.reverse" | "trace.export"
        )
}

fn canonical_network_origin(resource: &str) -> Result<String, GatewayError> {
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

fn validate_process_obligations(
    request: &EffectRequest,
    obligations: &PolicyObligations,
) -> Result<(), GatewayError> {
    let executable_allowed = if obligations.sandbox_backend == "oci" {
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
    let cwd_allowed = obligations.filesystem.iter().any(|grant| {
        matches!(grant.mode.as_str(), "read" | "write")
            && fs::canonicalize(&grant.root).is_ok_and(|root| cwd.starts_with(root))
    });
    if !cwd_allowed {
        return Err(GatewayError::Safety(
            "process cwd is outside allowed filesystem roots".into(),
        ));
    }
    let environment = request
        .content
        .get("environment")
        .and_then(Value::as_object)
        .ok_or_else(|| GatewayError::Safety("process environment object is absent".into()))?;
    for name in environment.keys() {
        if !obligations
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

fn normalized_absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn validate_filesystem_containment(
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

fn canonical_effect_path(
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

fn absolute_policy_root(root: &str) -> bool {
    Path::new(root).is_absolute()
        || (root.len() >= 3
            && root.as_bytes()[0].is_ascii_alphabetic()
            && root.as_bytes()[1] == b':'
            && matches!(root.as_bytes()[2], b'\\' | b'/'))
}

fn is_hard_secret_key(key: &str) -> bool {
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

fn redact_hard_secrets(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if is_hard_secret_key(key) && !is_environment_credential_reference(child) {
                    let bytes = serde_json::to_vec(child).unwrap_or_default();
                    *child = json!({
                        "redacted": true,
                        "sha256": sha256_hex(&bytes),
                        "size": bytes.len()
                    });
                } else {
                    redact_hard_secrets(child);
                }
            }
        }
        Value::Array(array) => array.iter_mut().for_each(redact_hard_secrets),
        _ => {}
    }
}

fn is_environment_credential_reference(value: &Value) -> bool {
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

fn disclosure_summary(request: &EffectRequest) -> Value {
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

/// Single policy-enforcement point for all external or sensitive effects.
pub struct EffectGateway {
    journal: Arc<dyn EventJournal>,
    policy: Arc<dyn PolicyDecisionPoint>,
    approvals: Arc<dyn ApprovalProvider>,
    risk_evaluator: RwLock<Option<Weak<dyn RiskEvaluator>>>,
    kernel: SafetyKernel,
    permit_key: [u8; 32],
}

struct StreamBridge<'a> {
    gateway: &'a EffectGateway,
    executor: &'a dyn StreamingEffectExecutor,
    observer: tokio::sync::Mutex<&'a mut dyn ReleasedEffectObserver>,
}

struct GatewayStreamSink<'a> {
    gateway: &'a EffectGateway,
    request: &'a EffectRequest,
    obligations: PolicyObligations,
    observer: &'a mut dyn ReleasedEffectObserver,
    sequence: u64,
    total_bytes: usize,
    last: Option<QuarantinedEffectResult>,
    failure: Option<StreamSinkFailure>,
}

enum StreamSinkFailure {
    Failed(String),
    Unknown(String),
    Denied(String),
}

impl StreamSinkFailure {
    fn execution_error(&self) -> ExecutionError {
        match self {
            Self::Failed(message) => ExecutionError::Failed(message.clone()),
            Self::Unknown(message) => ExecutionError::OutcomeUnknown(message.clone()),
            Self::Denied(message) => ExecutionError::ReleaseDenied(message.clone()),
        }
    }
}

#[async_trait]
impl QuarantinedEffectObserver for GatewayStreamSink<'_> {
    async fn observe(&mut self, result: QuarantinedEffectResult) -> Result<(), ExecutionError> {
        if let Some(failure) = &self.failure {
            return Err(failure.execution_error());
        }
        if !result.effect_succeeded {
            let failure =
                StreamSinkFailure::Failed("streaming adapter reported chunk failure".into());
            let error = failure.execution_error();
            self.failure = Some(failure);
            return Err(error);
        }
        let limit = match usize::try_from(self.obligations.max_output_bytes) {
            Ok(limit) => limit,
            Err(error) => {
                let failure = StreamSinkFailure::Failed(error.to_string());
                let error = failure.execution_error();
                self.failure = Some(failure);
                return Err(error);
            }
        };
        self.total_bytes = self.total_bytes.saturating_add(result.bytes.len());
        if self.total_bytes > limit {
            let failure = StreamSinkFailure::Unknown(
                "streamed provider output exceeds the cumulative permitted bound".into(),
            );
            let error = failure.execution_error();
            self.failure = Some(failure);
            return Err(error);
        }
        self.sequence = self.sequence.saturating_add(1);
        let released = match self
            .gateway
            .release_stream_chunk(self.request, &self.obligations, self.sequence, &result)
            .await
        {
            Ok(released) => released,
            Err(GatewayError::Denied(message)) => {
                let failure = StreamSinkFailure::Denied(message);
                let error = failure.execution_error();
                self.failure = Some(failure);
                return Err(error);
            }
            Err(error) => {
                let failure = StreamSinkFailure::Unknown(format!(
                    "stream release failed after execution began: {error}"
                ));
                let error = failure.execution_error();
                self.failure = Some(failure);
                return Err(error);
            }
        };
        if let Err(error) = self.observer.observe(released).await {
            let failure = match error {
                ExecutionError::ReleaseDenied(message) => StreamSinkFailure::Denied(message),
                ExecutionError::Failed(message)
                | ExecutionError::OutcomeUnknown(message)
                | ExecutionError::Recoverable { message, .. } => StreamSinkFailure::Unknown(
                    format!("released stream observation failed: {message}"),
                ),
            };
            let error = failure.execution_error();
            self.failure = Some(failure);
            return Err(error);
        }
        self.last = Some(result);
        Ok(())
    }
}

#[async_trait]
impl EffectExecutor for StreamBridge<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let obligations = permit.obligations().clone();
        let mut observer = self.observer.lock().await;
        let mut sink = GatewayStreamSink {
            gateway: self.gateway,
            request,
            obligations,
            observer: &mut **observer,
            sequence: 0,
            total_bytes: 0,
            last: None,
            failure: None,
        };
        let terminal = self
            .executor
            .execute_stream(request, permit, &mut sink)
            .await?;
        if let Some(failure) = &sink.failure {
            return Err(failure.execution_error());
        }
        if sink.sequence == 0 || sink.last.as_ref() != Some(&terminal) {
            return Err(ExecutionError::Failed(
                "streaming adapter terminal result did not match its last released chunk".into(),
            ));
        }
        Ok(terminal)
    }
}

impl EffectGateway {
    /// Compose trusted journal, policy, approval, and permit services.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        policy: Arc<dyn PolicyDecisionPoint>,
        approvals: Arc<dyn ApprovalProvider>,
        kernel: SafetyKernel,
        permit_key: [u8; 32],
    ) -> Self {
        Self {
            journal,
            policy,
            approvals,
            risk_evaluator: RwLock::new(None),
            kernel,
            permit_key,
        }
    }

    /// Bind the policy-gated model evaluator after provider composition is complete.
    pub fn bind_risk_evaluator(
        &self,
        evaluator: Weak<dyn RiskEvaluator>,
    ) -> Result<(), GatewayError> {
        *self
            .risk_evaluator
            .write()
            .map_err(|_| GatewayError::Contract("risk evaluator lock is poisoned".into()))? =
            Some(evaluator);
        Ok(())
    }

    async fn review_risk(
        &self,
        request: &mut EffectRequest,
        decision: &PolicyDecision,
    ) -> Result<bool, GatewayError> {
        if request.action != "shell.run" || !self.approvals.risk_auto_enabled() {
            return Ok(false);
        }
        self.event(
            request,
            "risk.review.requested.v1",
            EventClassification::Policy,
            json!({
                "decision_id": decision.decision_id,
                "policy_revision": decision.policy_revision,
            }),
        )?;
        let evaluator = self
            .risk_evaluator
            .read()
            .map_err(|_| GatewayError::Contract("risk evaluator lock is poisoned".into()))?
            .as_ref()
            .and_then(Weak::upgrade);
        let Some(evaluator) = evaluator else {
            request.risk.status = RiskStatus::Unavailable;
            request.risk.level = None;
            request.risk.reason = Some("risk evaluator is not available".into());
            self.event(
                request,
                "risk.review.unavailable.v1",
                EventClassification::Policy,
                json!({"reason": "risk evaluator is not available"}),
            )?;
            return Ok(false);
        };
        match evaluator.evaluate(request, decision).await {
            Ok(assessment) => {
                let reason = assessment.reason.trim();
                if reason.is_empty() || reason.chars().count() > 1_000 {
                    request.risk.status = RiskStatus::Unavailable;
                    request.risk.level = None;
                    request.risk.reason = Some("risk evaluator returned an invalid reason".into());
                    self.event(
                        request,
                        "risk.review.unavailable.v1",
                        EventClassification::Policy,
                        json!({"reason": "risk evaluator returned an invalid reason"}),
                    )?;
                    return Ok(false);
                }
                request.risk.status = RiskStatus::Available;
                request.risk.level = Some(
                    match assessment.risk_level {
                        RiskLevel::Low => "low",
                        RiskLevel::Medium => "medium",
                        RiskLevel::High => "high",
                    }
                    .into(),
                );
                request.risk.reason = Some(reason.into());
                self.event(
                    request,
                    "risk.review.completed.v1",
                    EventClassification::Policy,
                    json!({
                        "decision_id": decision.decision_id,
                        "risk_level": assessment.risk_level,
                        "recommended_decision": assessment.recommended_decision,
                        "reason": reason,
                    }),
                )?;
                Ok(assessment.risk_level == RiskLevel::Low
                    && assessment.recommended_decision == RiskRecommendation::Allow)
            }
            Err(error) => {
                let message = error.to_string();
                let bounded = message.chars().take(1_000).collect::<String>();
                request.risk.status = RiskStatus::Unavailable;
                request.risk.level = None;
                request.risk.reason = Some(bounded.clone());
                self.event(
                    request,
                    "risk.review.unavailable.v1",
                    EventClassification::Policy,
                    json!({"reason": bounded}),
                )?;
                Ok(false)
            }
        }
    }

    fn event(
        &self,
        request: &EffectRequest,
        event_type: &str,
        classification: EventClassification,
        payload: Value,
    ) -> Result<(), GatewayError> {
        let stream_id = format!("effect:{}", request.request_id);
        let version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| GatewayError::Contract(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: version,
            classification,
            event_type: event_type.into(),
            actor: request.actor.clone(),
            context: request.context.clone(),
            payload,
        })?;
        Ok(())
    }

    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, GatewayError> {
        let decision = match self.policy.decide(request).await {
            Ok(decision) => decision,
            Err(error) => {
                self.event(
                    request,
                    "policy.error.v1",
                    EventClassification::Policy,
                    json!({"error_kind": "unavailable_or_invalid", "message": error.to_string()}),
                )?;
                self.event(
                    request,
                    "effect.denied.v1",
                    EventClassification::Effect,
                    json!({"reason": "policy failure; fail closed"}),
                )?;
                return Err(error.into());
            }
        };
        if let Err(error) = self.kernel.validate_decision(request, &decision) {
            self.event(
                request,
                "policy.error.v1",
                EventClassification::Policy,
                json!({"error_kind": "invalid_decision", "message": error.to_string()}),
            )?;
            self.event(
                request,
                "effect.denied.v1",
                EventClassification::Effect,
                json!({"reason": "invalid policy decision; fail closed"}),
            )?;
            return Err(error);
        }
        self.event(
            request,
            "policy.decided.v1",
            EventClassification::Policy,
            json!({
                "decision_id": decision.decision_id,
                "policy_revision": decision.policy_revision,
                "outcome": decision.outcome,
                "reason": decision.reason,
                "audit_labels": decision.obligations.audit_labels,
            }),
        )?;
        Ok(decision)
    }

    fn mint_permit(
        &self,
        request: &EffectRequest,
        request_hash: String,
        decision: &PolicyDecision,
    ) -> Result<ExecutionPermit, GatewayError> {
        let obligations_hash = sha256_hex(&canonical_bytes(&decision.obligations)?);
        let nonce = Uuid::now_v7().to_string();
        let expires_at_unix_ms = now_unix_ms() + PERMIT_LIFETIME_MS;
        let claims = PermitClaims {
            request_hash: &request_hash,
            decision_id: &decision.decision_id,
            obligations_hash: &obligations_hash,
            actor_id: &request.actor.id,
            nonce: &nonce,
            expires_at_unix_ms,
        };
        let mut mac = HmacSha256::new_from_slice(&self.permit_key)
            .map_err(|error| GatewayError::Contract(error.to_string()))?;
        mac.update(&canonical_bytes(&claims)?);
        Ok(ExecutionPermit {
            request_hash,
            decision_id: decision.decision_id.clone(),
            obligations_hash,
            actor_id: request.actor.id.clone(),
            nonce,
            expires_at_unix_ms,
            authentication_tag: mac.finalize().into_bytes().to_vec(),
            obligations: decision.obligations.clone(),
            consumed: AtomicBool::new(false),
        })
    }

    fn authenticate_and_consume(
        &self,
        permit: &ExecutionPermit,
        request: &EffectRequest,
        decision: &PolicyDecision,
    ) -> Result<(), GatewayError> {
        let request_hash = sha256_hex(&canonical_bytes(request)?);
        let obligations_hash = sha256_hex(&canonical_bytes(&decision.obligations)?);
        if permit.request_hash != request_hash
            || permit.decision_id != decision.decision_id
            || permit.obligations_hash != obligations_hash
            || permit.actor_id != request.actor.id
            || permit.expires_at_unix_ms < now_unix_ms()
        {
            return Err(GatewayError::Safety(
                "permit does not match request, decision, actor, obligations, or expiry".into(),
            ));
        }
        let claims = PermitClaims {
            request_hash: &permit.request_hash,
            decision_id: &permit.decision_id,
            obligations_hash: &permit.obligations_hash,
            actor_id: &permit.actor_id,
            nonce: &permit.nonce,
            expires_at_unix_ms: permit.expires_at_unix_ms,
        };
        let mut mac = HmacSha256::new_from_slice(&self.permit_key)
            .map_err(|error| GatewayError::Contract(error.to_string()))?;
        mac.update(&canonical_bytes(&claims)?);
        mac.verify_slice(&permit.authentication_tag)
            .map_err(|_| GatewayError::Safety("permit authentication failed".into()))?;
        permit
            .consumed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| GatewayError::Safety("permit has already been consumed".into()))?;
        Ok(())
    }

    /// Authorize, execute into quarantine, optionally authorize output, and release.
    pub async fn execute(
        &self,
        request: EffectRequest,
        executor: &dyn EffectExecutor,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        self.execute_internal(request, executor, false).await
    }

    /// Authorize one streaming effect and release only gateway-approved normalized chunks.
    pub async fn execute_stream(
        &self,
        request: EffectRequest,
        executor: &dyn StreamingEffectExecutor,
        observer: &mut dyn ReleasedEffectObserver,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        let bridge = StreamBridge {
            gateway: self,
            executor,
            observer: tokio::sync::Mutex::new(observer),
        };
        self.execute_internal(request, &bridge, true).await
    }

    async fn execute_internal(
        &self,
        request: EffectRequest,
        executor: &dyn EffectExecutor,
        chunks_already_released: bool,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        if self.journal.is_recovery_mode() {
            return Err(GatewayError::Journal(StoreError::RecoveryMode));
        }
        if request.schema_version != 1
            || request.request_id.is_empty()
            || request.phase != EffectPhase::PreEffect
        {
            return Err(GatewayError::Safety(
                "unsupported schema version, empty request id, or caller-supplied post-effect phase"
                    .into(),
            ));
        }
        self.event(
            &request,
            "effect.requested.v1",
            EventClassification::Effect,
            disclosure_summary(&request),
        )?;
        let mut request = match self.kernel.prepare(&request) {
            Ok(request) => request,
            Err(error) => {
                self.event(
                    &request,
                    "effect.denied.v1",
                    EventClassification::Effect,
                    json!({"reason": error.to_string(), "source": "safety_kernel"}),
                )?;
                return Err(error);
            }
        };
        let mut decision = self.decide(&request).await?;
        if decision.outcome == DecisionOutcome::RequireApproval {
            let risk_auto_approved = self.review_risk(&mut request, &decision).await?;
            let request_hash = sha256_hex(&canonical_bytes(&request)?);
            let approval = if risk_auto_approved {
                Ok(Some(approval_proof(
                    &request_hash,
                    "risk-evaluator:auto-low-risk",
                )?))
            } else {
                self.approvals
                    .request_approval(&request, &request_hash, &decision)
                    .await
            };
            let proof = match approval {
                Ok(Some(proof)) => proof,
                Ok(None) => {
                    self.event(
                        &request,
                        "approval.denied.v1",
                        EventClassification::Approval,
                        json!({"decision_id": decision.decision_id, "reason": "operator declined"}),
                    )?;
                    self.event(
                        &request,
                        "effect.denied.v1",
                        EventClassification::Effect,
                        json!({"decision_id": decision.decision_id, "reason": "operator declined"}),
                    )?;
                    return Err(GatewayError::Approval("operator declined".into()));
                }
                Err(error) => {
                    self.event(
                        &request,
                        "approval.error.v1",
                        EventClassification::Approval,
                        json!({"decision_id": decision.decision_id, "message": error.to_string()}),
                    )?;
                    self.event(
                        &request,
                        "effect.denied.v1",
                        EventClassification::Effect,
                        json!({"decision_id": decision.decision_id, "reason": "approval provider failed"}),
                    )?;
                    return Err(GatewayError::Policy(error));
                }
            };
            if proof.request_hash != request_hash {
                return Err(GatewayError::Approval(
                    "approval proof is bound to a different request".into(),
                ));
            }
            self.event(
                &request,
                "approval.granted.v1",
                EventClassification::Approval,
                json!({
                    "approval_id": proof.approval_id,
                    "approved_by": proof.approved_by,
                    "request_hash": proof.request_hash,
                }),
            )?;
            request.approval = Some(proof);
            decision = self.decide(&request).await?;
        }
        if decision.outcome != DecisionOutcome::Allow {
            self.event(
                &request,
                "effect.denied.v1",
                EventClassification::Effect,
                json!({"decision_id": decision.decision_id, "reason": decision.reason}),
            )?;
            return Err(GatewayError::Denied(decision.reason));
        }
        let request_hash = sha256_hex(&canonical_bytes(&request)?);
        let permit = self.mint_permit(&request, request_hash, &decision)?;
        self.authenticate_and_consume(&permit, &request, &decision)?;
        self.event(
            &request,
            "effect.started.v1",
            EventClassification::Effect,
            json!({
                "decision_id": decision.decision_id,
                "permit_nonce": permit.nonce,
                "permit_expires_at_unix_ms": permit.expires_at_unix_ms,
            }),
        )?;

        let result = match tokio::time::timeout(
            Duration::from_millis(decision.obligations.timeout_ms),
            executor.execute(&request, permit),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(ExecutionError::Failed(message))) => {
                self.event(
                    &request,
                    "effect.failed.v1",
                    EventClassification::Effect,
                    json!({"message": message}),
                )?;
                return Err(GatewayError::Execution(message));
            }
            Ok(Err(ExecutionError::Recoverable { code, message })) => {
                self.event(
                    &request,
                    "effect.failed.v1",
                    EventClassification::Effect,
                    json!({"code": code, "message": message, "recoverable": true}),
                )?;
                return Err(GatewayError::RecoverableExecution { code, message });
            }
            Ok(Err(ExecutionError::OutcomeUnknown(message))) => {
                self.event(
                    &request,
                    "effect.outcome_unknown.v1",
                    EventClassification::Effect,
                    json!({"message": message}),
                )?;
                return Err(GatewayError::OutcomeUnknown(message));
            }
            Ok(Err(ExecutionError::ReleaseDenied(message))) => {
                return Err(GatewayError::Denied(message));
            }
            Err(_) => {
                let message = "adapter timed out after execution began".to_owned();
                self.event(
                    &request,
                    "effect.outcome_unknown.v1",
                    EventClassification::Effect,
                    json!({"message": message}),
                )?;
                return Err(GatewayError::OutcomeUnknown(message));
            }
        };
        if result.bytes.len()
            > usize::try_from(decision.obligations.max_output_bytes).unwrap_or(usize::MAX)
        {
            self.event(
                &request,
                "effect.failed.v1",
                EventClassification::Effect,
                json!({"message": "quarantined output exceeded policy limit"}),
            )?;
            return Err(GatewayError::Execution(
                "quarantined output exceeded policy limit".into(),
            ));
        }
        if !result.effect_succeeded {
            self.event(
                &request,
                "effect.failed.v1",
                EventClassification::Effect,
                json!({"message": "adapter reported effect failure"}),
            )?;
            return Err(GatewayError::Execution(
                "adapter reported effect failure".into(),
            ));
        }

        if decision.obligations.require_post_effect && !chunks_already_released {
            let mut post_request = request.clone();
            post_request.request_id = format!("{}:post", request.request_id);
            post_request.phase = EffectPhase::PostEffect;
            post_request.approval = None;
            post_request.content = json!({
                "media_type": result.media_type,
                "size": result.bytes.len(),
                "content_base64": BASE64.encode(&result.bytes),
            });
            let post_request = self.kernel.prepare(&post_request)?;
            self.event(
                &post_request,
                "effect.release_requested.v1",
                EventClassification::Effect,
                disclosure_summary(&post_request),
            )?;
            let post_decision = self.decide(&post_request).await?;
            if post_decision.outcome != DecisionOutcome::Allow {
                self.event(
                    &post_request,
                    "effect.release_denied.v1",
                    EventClassification::Effect,
                    json!({
                        "decision_id": post_decision.decision_id,
                        "reason": post_decision.reason,
                        "content_hash": sha256_hex(&result.bytes),
                        "size": result.bytes.len(),
                    }),
                )?;
                return Err(GatewayError::Denied(format!(
                    "post-effect release denied: {}",
                    post_decision.reason
                )));
            }
        }

        self.event(
            &request,
            "effect.completed.v1",
            EventClassification::Effect,
            json!({
                "decision_id": decision.decision_id,
                "content_hash": sha256_hex(&result.bytes),
                "size": result.bytes.len(),
            }),
        )?;
        Ok(ReleasedEffectResult {
            media_type: result.media_type,
            bytes: result.bytes,
        })
    }

    async fn release_stream_chunk(
        &self,
        request: &EffectRequest,
        obligations: &PolicyObligations,
        sequence: u64,
        result: &QuarantinedEffectResult,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        if obligations.require_post_effect {
            let mut post_request = request.clone();
            post_request.request_id = format!("{}:post:chunk:{sequence}", request.request_id);
            post_request.phase = EffectPhase::PostEffect;
            post_request.approval = None;
            post_request.content = json!({
                "media_type": result.media_type,
                "size": result.bytes.len(),
                "sequence": sequence,
                "content_base64": BASE64.encode(&result.bytes),
            });
            let post_request = self.kernel.prepare(&post_request)?;
            self.event(
                &post_request,
                "effect.release_requested.v1",
                EventClassification::Effect,
                disclosure_summary(&post_request),
            )?;
            let post_decision = self.decide(&post_request).await?;
            if post_decision.outcome != DecisionOutcome::Allow {
                self.event(
                    &post_request,
                    "effect.release_denied.v1",
                    EventClassification::Effect,
                    json!({
                        "decision_id": post_decision.decision_id,
                        "reason": post_decision.reason,
                        "content_hash": sha256_hex(&result.bytes),
                        "size": result.bytes.len(),
                        "sequence": sequence,
                    }),
                )?;
                return Err(GatewayError::Denied(format!(
                    "stream chunk post-effect release denied: {}",
                    post_decision.reason
                )));
            }
        }
        self.event(
            request,
            "effect.chunk_released.v1",
            EventClassification::Effect,
            json!({
                "content_hash": sha256_hex(&result.bytes),
                "size": result.bytes.len(),
                "sequence": sequence,
                "media_type": result.media_type,
            }),
        )?;
        Ok(ReleasedEffectResult {
            media_type: result.media_type.clone(),
            bytes: result.bytes.clone(),
        })
    }
}

fn default_obligations() -> PolicyObligations {
    PolicyObligations {
        sandbox_backend: "broker".into(),
        sandbox_profile: "offline-default".into(),
        filesystem: Vec::new(),
        network_destinations: Vec::new(),
        allowed_environment: Vec::new(),
        allow_sandbox_downgrade: false,
        timeout_ms: 30_000,
        max_output_bytes: 1024 * 1024,
        max_processes: 1,
        max_memory_bytes: 256 * 1024 * 1024,
        max_concurrency: 1,
        required_redactions: Vec::new(),
        require_post_effect: false,
        audit_labels: BTreeMap::new(),
        retention: "standard".into(),
    }
}

/// Offline policy with an explicit action outcome map and deny-by-default behavior.
pub struct BuiltInPolicy {
    revision: String,
    actions: BTreeMap<String, DecisionOutcome>,
    obligations: PolicyObligations,
    action_obligations: BTreeMap<String, PolicyObligations>,
}

impl BuiltInPolicy {
    /// Secure offline defaults: only the deterministic echo provider is allowed.
    pub fn offline_default() -> Self {
        Self {
            revision: "builtin/offline-v1".into(),
            actions: BTreeMap::from([("provider.echo".into(), DecisionOutcome::Allow)]),
            obligations: default_obligations(),
            action_obligations: BTreeMap::new(),
        }
    }

    /// Add or replace an exact action decision.
    pub fn with_action(mut self, action: impl Into<String>, outcome: DecisionOutcome) -> Self {
        self.actions.insert(action.into(), outcome);
        self
    }

    /// Require a post-effect release decision for allowed actions.
    pub fn with_post_effect(mut self, required: bool) -> Self {
        self.obligations.require_post_effect = required;
        self
    }

    /// Add one canonical read-only filesystem root to built-in obligations.
    pub fn with_filesystem_read_root(mut self, root: impl Into<String>) -> Self {
        self = self.with_filesystem_root(root, "read");
        self
    }

    /// Add one canonical filesystem root and known access mode.
    pub fn with_filesystem_root(
        mut self,
        root: impl Into<String>,
        mode: impl Into<String>,
    ) -> Self {
        self.obligations
            .filesystem
            .push(colossus_contracts::FilesystemGrant {
                root: root.into(),
                mode: mode.into(),
            });
        self
    }

    /// Select the sandbox backend/profile and explicit downgrade behavior.
    pub fn with_sandbox(
        mut self,
        backend: impl Into<String>,
        profile: impl Into<String>,
        allow_downgrade: bool,
    ) -> Self {
        self.obligations.sandbox_backend = backend.into();
        self.obligations.sandbox_profile = profile.into();
        self.obligations.allow_sandbox_downgrade = allow_downgrade;
        self
    }

    /// Allow one exact environment variable name inside sandboxed processes.
    pub fn with_environment(mut self, name: impl Into<String>) -> Self {
        self.obligations.allowed_environment.push(name.into());
        self
    }

    /// Allow one canonical HTTP(S) origin for brokered network requests.
    pub fn with_network_destination(mut self, origin: impl Into<String>) -> Self {
        self.obligations.network_destinations.push(origin.into());
        self
    }

    /// Apply bounded process, memory, output, timeout, and concurrency ceilings.
    pub fn with_limits(
        mut self,
        timeout_ms: u64,
        max_output_bytes: u64,
        max_processes: u32,
        max_memory_bytes: u64,
        max_concurrency: u32,
    ) -> Self {
        self.obligations.timeout_ms = timeout_ms;
        self.obligations.max_output_bytes = max_output_bytes;
        self.obligations.max_processes = max_processes;
        self.obligations.max_memory_bytes = max_memory_bytes;
        self.obligations.max_concurrency = max_concurrency;
        self
    }

    /// Restrict one exact action to its own filesystem, environment, and network grants.
    pub fn with_action_restrictions(
        mut self,
        action: impl Into<String>,
        filesystem: Vec<colossus_contracts::FilesystemGrant>,
        allowed_environment: Vec<String>,
        network_destinations: Vec<String>,
    ) -> Self {
        let mut obligations = self.obligations.clone();
        obligations.filesystem = filesystem;
        obligations.allowed_environment = allowed_environment;
        obligations.network_destinations = network_destinations;
        self.action_obligations.insert(action.into(), obligations);
        self
    }
}

#[async_trait]
impl PolicyDecisionPoint for BuiltInPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut outcome = self
            .actions
            .get(&request.action)
            .copied()
            .unwrap_or_else(|| {
                if request.action.starts_with("openapi.")
                    || request.action.starts_with("github.")
                    || request.action.starts_with("searxng.")
                    || request.action.starts_with("opensearch.")
                    || request.action == "web.search"
                    || request.action == "mcp.call"
                {
                    DecisionOutcome::RequireApproval
                } else {
                    DecisionOutcome::Deny
                }
            });
        if outcome == DecisionOutcome::RequireApproval
            && (request.approval.is_some() || request.phase == EffectPhase::PostEffect)
        {
            outcome = DecisionOutcome::Allow;
        }
        let mut obligations = self
            .action_obligations
            .get(&request.action)
            .cloned()
            .unwrap_or_else(|| self.obligations.clone());
        if request.action.starts_with("filesystem.")
            || is_process_action(&request.action)
            || matches!(
                request.action.as_str(),
                "provider.openai.responses" | "provider.openai.chat" | "provider.models"
            )
            || request.action.starts_with("task.")
            || request.action.starts_with("decision.")
            || request.action.starts_with("plan.")
            || request.action.starts_with("goal.")
            || request.action.starts_with("subagent.")
            || request.action.starts_with("memory.")
            || request.action.starts_with("skill.")
            || request.action.starts_with("research.")
            || request.action.starts_with("integration.")
            || request.action.starts_with("openapi.")
            || request.action.starts_with("github.")
            || request.action.starts_with("searxng.")
            || request.action.starts_with("opensearch.")
            || request.action.starts_with("mcp.")
            || request.action.starts_with("pack.")
            || request.action.starts_with("bundle.")
            || request.action.starts_with("collection.")
            || request.action.starts_with("registry.")
            || matches!(
                request.action.as_str(),
                "network.http" | "web.search" | "audit.export.worm.write"
            )
        {
            obligations.require_post_effect = true;
        }
        Ok(PolicyDecision {
            decision_id: Uuid::now_v7().to_string(),
            policy_revision: self.revision.clone(),
            outcome,
            reason: match outcome {
                DecisionOutcome::Allow => "allowed by explicit built-in rule",
                DecisionOutcome::Deny => "denied by built-in default",
                DecisionOutcome::RequireApproval => "explicit operator approval required",
            }
            .into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        Ok(json!({
            "ready": true,
            "kind": "built_in",
            "revision": self.revision,
            "default": "deny"
        }))
    }
}

/// Approval provider that always denies; safe for non-interactive runs.
pub struct DenyApproval;

#[async_trait]
impl ApprovalProvider for DenyApproval {
    async fn request_approval(
        &self,
        _request: &EffectRequest,
        _request_hash: &str,
        _decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        Ok(None)
    }
}

/// Approval provider used by trusted application APIs after explicit operator action.
pub struct AllowApproval {
    /// Stable approving operator identifier.
    pub approved_by: String,
}

#[async_trait]
impl ApprovalProvider for AllowApproval {
    async fn request_approval(
        &self,
        _request: &EffectRequest,
        request_hash: &str,
        _decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        Ok(Some(approval_proof(
            request_hash,
            self.approved_by.clone(),
        )?))
    }
}

/// Strict OPA REST/mTLS configuration.
pub struct OpaConfig {
    /// OPA base URL, without the fixed decision path.
    pub base_url: String,
    /// Fixed data decision path, such as `colossus/effect`.
    pub decision_path: String,
    /// PEM trust anchor. Required for remote OPA.
    pub ca_pem: Option<Vec<u8>>,
    /// PEM client certificate plus private key. Required for remote OPA.
    pub identity_pem: Option<Vec<u8>>,
    /// Explicit acknowledgement that full logical content is sent.
    pub full_content_disclosure_acknowledged: bool,
    /// Operator assertion that OPA decision logs are disabled or safely masked.
    pub decision_log_masking_verified: bool,
    /// Transport timeout.
    pub timeout: Duration,
}

/// OPA policy decision point. Colossus still enforces every returned obligation.
pub struct OpaPolicy {
    client: Client,
    decision_url: Url,
    ready_url: Url,
    decision_log_masking_verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpaDecisionResponse {
    result: PolicyDecision,
}

impl OpaPolicy {
    /// Validate disclosure/TLS invariants and construct the OPA client.
    pub fn new(config: OpaConfig) -> Result<Self, PolicyError> {
        if !config.full_content_disclosure_acknowledged {
            return Err(PolicyError::InvalidDecision(
                "full-content OPA disclosure acknowledgement is required".into(),
            ));
        }
        if config.decision_path.is_empty()
            || config.decision_path.starts_with('/')
            || config.decision_path.contains("..")
        {
            return Err(PolicyError::InvalidDecision(
                "OPA decision path must be fixed, relative, and non-traversing".into(),
            ));
        }
        let base = Url::parse(&config.base_url)
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
        let local = base
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if !local && base.scheme() != "https" {
            return Err(PolicyError::InvalidDecision(
                "remote OPA requires HTTPS".into(),
            ));
        }
        if !local && (config.ca_pem.is_none() || config.identity_pem.is_none()) {
            return Err(PolicyError::InvalidDecision(
                "remote OPA requires pinned CA trust and mTLS identity".into(),
            ));
        }
        let mut builder = Client::builder().timeout(config.timeout);
        if let Some(ca_pem) = config.ca_pem {
            let certificate = Certificate::from_pem(&ca_pem)
                .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
            builder = builder
                .tls_built_in_root_certs(false)
                .add_root_certificate(certificate);
        }
        if let Some(identity_pem) = config.identity_pem {
            let identity = Identity::from_pem(&identity_pem)
                .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
            builder = builder.identity(identity);
        }
        let client = builder
            .build()
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let decision_url = base
            .join(&format!("v1/data/{}", config.decision_path))
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
        let ready_url = base
            .join("health?bundles=true&plugins=true")
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))?;
        Ok(Self {
            client,
            decision_url,
            ready_url,
            decision_log_masking_verified: config.decision_log_masking_verified,
        })
    }
}

#[async_trait]
impl PolicyDecisionPoint for OpaPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let response = self
            .client
            .post(self.decision_url.clone())
            .json(&json!({"input": request}))
            .send()
            .await
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(PolicyError::Unavailable(format!(
                "OPA decision endpoint returned {}",
                response.status()
            )));
        }
        response
            .json::<OpaDecisionResponse>()
            .await
            .map(|response| response.result)
            .map_err(|error| PolicyError::InvalidDecision(error.to_string()))
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        let response = self
            .client
            .get(self.ready_url.clone())
            .send()
            .await
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(PolicyError::Unavailable(format!(
                "OPA readiness returned {}",
                response.status()
            )));
        }
        Ok(json!({
            "ready": true,
            "kind": "opa",
            "decision_url": self.decision_url.as_str(),
            "decision_log_masking_verified": self.decision_log_masking_verified,
            "warning": if self.decision_log_masking_verified {
                Value::Null
            } else {
                Value::String("OPA decision-log masking could not be verified".into())
            }
        }))
    }
}

/// Build a minimal effect request for trusted callers without losing provenance.
pub fn effect_request(
    actor: Actor,
    action: impl Into<String>,
    resource: impl Into<String>,
    content: Value,
) -> EffectRequest {
    EffectRequest {
        schema_version: 1,
        request_id: Uuid::now_v7().to_string(),
        actor,
        action: action.into(),
        resource: resource.into(),
        capabilities: Vec::new(),
        risk: colossus_contracts::RiskInput {
            status: colossus_contracts::RiskStatus::Unavailable,
            level: None,
            reason: None,
        },
        content,
        credential_references: Vec::new(),
        context: colossus_contracts::ExecutionContext {
            correlation_id: Uuid::now_v7().to_string(),
            ..colossus_contracts::ExecutionContext::default()
        },
        idempotency_id: None,
        phase: EffectPhase::PreEffect,
        approval: None,
    }
}

/// Trusted system actor used by kernel services and offline smoke adapters.
pub fn system_actor(id: impl Into<String>) -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: id.into(),
    }
}

#[cfg(test)]
mod tests;
