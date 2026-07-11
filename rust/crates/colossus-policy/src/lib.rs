//! Non-bypassable effect gateway, built-in policy, and OPA adapter.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, ApprovalProof, DecisionOutcome, EffectPhase, EffectRequest,
    EventClassification, NewEvent, PolicyDecision, PolicyObligations, QuarantinedEffectResult,
};
use colossus_ports::{
    ApprovalProvider, EventJournal, PolicyDecisionPoint, PolicyError, StoreError,
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
        Arc,
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
            && request.action == "process.spawn"
            && !obligations.allow_sandbox_downgrade
        {
            return Err(GatewayError::Safety(
                "process execution cannot downgrade to the broker without an explicit obligation"
                    .into(),
            ));
        }
        if obligations.sandbox_backend == "windows_job" && request.action == "process.spawn" {
            return Err(GatewayError::Safety(
                "windows_job process execution is reserved and currently fail-closed".into(),
            ));
        }
        if cfg!(target_os = "windows")
            && obligations.sandbox_backend == "oci"
            && request.action == "process.spawn"
        {
            return Err(GatewayError::Safety(
                "OCI process execution is disabled on Windows until path mapping passes live acceptance"
                    .into(),
            ));
        }
        if obligations.sandbox_backend == "oci"
            && request.action == "process.spawn"
            && obligations.timeout_ms < MIN_OCI_EFFECT_TIMEOUT_MS
        {
            return Err(GatewayError::Safety(format!(
                "OCI process execution requires timeout_ms >= {MIN_OCI_EFFECT_TIMEOUT_MS} so cleanup can be confirmed"
            )));
        }
        if obligations.sandbox_backend == "oci"
            && request.action == "process.spawn"
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
        if decision.outcome == DecisionOutcome::Allow && request.action.starts_with("filesystem.") {
            validate_filesystem_containment(request, obligations)?;
        }
        if decision.outcome == DecisionOutcome::Allow && request.action == "process.spawn" {
            validate_process_obligations(request, obligations)?;
        }
        if decision.outcome == DecisionOutcome::Allow
            && (request.action == "network.http"
                || matches!(
                    request.action.as_str(),
                    "provider.openai.responses" | "provider.openai.chat" | "provider.models"
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
    let requested_mode = if request.action.contains("write") || request.action.contains("patch") {
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
                if is_hard_secret_key(key) {
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
    kernel: SafetyKernel,
    permit_key: [u8; 32],
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
            kernel,
            permit_key,
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
        if self.journal.is_recovery_mode() {
            return Err(GatewayError::Journal(StoreError::RecoveryMode));
        }
        if request.schema_version != 1 || request.request_id.is_empty() {
            return Err(GatewayError::Safety(
                "unsupported schema version or empty request id".into(),
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
        let request_hash = sha256_hex(&canonical_bytes(&request)?);
        let mut decision = self.decide(&request).await?;
        if decision.outcome == DecisionOutcome::RequireApproval {
            let proof = self
                .approvals
                .request_approval(&request, &request_hash, &decision)
                .await?
                .ok_or_else(|| GatewayError::Approval("operator declined".into()))?;
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

        if decision.obligations.require_post_effect {
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
}

impl BuiltInPolicy {
    /// Secure offline defaults: only the deterministic echo provider is allowed.
    pub fn offline_default() -> Self {
        Self {
            revision: "builtin/offline-v1".into(),
            actions: BTreeMap::from([("provider.echo".into(), DecisionOutcome::Allow)]),
            obligations: default_obligations(),
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
}

#[async_trait]
impl PolicyDecisionPoint for BuiltInPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut outcome = self
            .actions
            .get(&request.action)
            .copied()
            .unwrap_or(DecisionOutcome::Deny);
        if outcome == DecisionOutcome::RequireApproval && request.approval.is_some() {
            outcome = DecisionOutcome::Allow;
        }
        let mut obligations = self.obligations.clone();
        if (request.action.starts_with("filesystem.") && request.action != "filesystem.write")
            || request.action == "network.http"
            || matches!(
                request.action.as_str(),
                "memory.read" | "memory.list" | "memory.search"
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
        let approved_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        Ok(Some(ApprovalProof {
            approval_id: Uuid::now_v7().to_string(),
            request_hash: request_hash.into(),
            approved_by: self.approved_by.clone(),
            approved_at,
        }))
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
mod tests {
    use super::{
        AllowApproval, BuiltInPolicy, EffectExecutor, EffectGateway, ExecutionError,
        ExecutionPermit, GatewayError, SafetyKernel, effect_request, system_actor,
    };
    use async_trait::async_trait;
    use colossus_contracts::{DecisionOutcome, QuarantinedEffectResult};
    use colossus_ports::{EventJournal, PolicyDecisionPoint};
    use colossus_testkit::InMemoryEventJournal;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    struct CountingExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl EffectExecutor for CountingExecutor {
        async fn execute(
            &self,
            _request: &colossus_contracts::EffectRequest,
            _permit: ExecutionPermit,
        ) -> Result<QuarantinedEffectResult, ExecutionError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(QuarantinedEffectResult {
                media_type: "text/plain".into(),
                bytes: b"ok".to_vec(),
                effect_succeeded: true,
            })
        }
    }

    #[tokio::test]
    async fn deny_never_reaches_adapter() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(BuiltInPolicy::offline_default()),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new([]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let error = gateway
            .execute(
                effect_request(
                    system_actor("test"),
                    "filesystem.write",
                    "/tmp/x",
                    serde_json::json!({"content":"x"}),
                ),
                &executor,
            )
            .await
            .expect_err("deny");
        assert!(matches!(error, GatewayError::Denied(_)));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
        let names = journal
            .read_global(1, 20)
            .expect("events")
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(names.contains(&"effect.denied.v1".into()));
    }

    #[tokio::test]
    async fn memory_disclosures_always_require_post_effect_release() {
        let policy = BuiltInPolicy::offline_default()
            .with_action("memory.search", DecisionOutcome::Allow)
            .with_post_effect(false);
        let decision = policy
            .decide(&effect_request(
                system_actor("memory-test"),
                "memory.search",
                "session:one",
                serde_json::json!({"query": "rust"}),
            ))
            .await
            .expect("decision");
        assert_eq!(decision.outcome, DecisionOutcome::Allow);
        assert!(decision.obligations.require_post_effect);
    }

    #[tokio::test]
    async fn process_environment_and_executable_obligations_fail_closed() {
        let directory = tempfile::tempdir().expect("directory");
        let executable = std::env::current_exe()
            .expect("executable")
            .canonicalize()
            .expect("canonical executable");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action("process.spawn", DecisionOutcome::Allow)
            .with_sandbox("native", "test", false)
            .with_filesystem_root(executable.display().to_string(), "execute")
            .with_filesystem_read_root(directory.path().display().to_string());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new(["process.spawn".into()]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let mut request = effect_request(
            system_actor("test"),
            "process.spawn",
            executable.display().to_string(),
            serde_json::json!({
                "cwd": directory.path(),
                "args": [],
                "environment": {"SECRET": "not allowed"},
                "stdin_base64": null,
            }),
        );
        request.capabilities = vec!["process.spawn".into()];
        let error = gateway
            .execute(request, &executor)
            .await
            .expect_err("environment denied");
        assert!(matches!(error, GatewayError::Safety(_)));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn oci_process_timeout_must_reserve_confirmed_cleanup_time() {
        let directory = tempfile::tempdir().expect("directory");
        let executable = std::env::current_exe()
            .expect("executable")
            .canonicalize()
            .expect("canonical executable");
        let policy = BuiltInPolicy::offline_default()
            .with_action("process.spawn", DecisionOutcome::Allow)
            .with_sandbox("oci", "test", false)
            .with_limits(1_000, 1024, 2, 64 * 1024 * 1024, 1)
            .with_filesystem_root(executable.display().to_string(), "execute")
            .with_filesystem_read_root(directory.path().display().to_string());
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new(["process.spawn".into()]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let mut request = effect_request(
            system_actor("test"),
            "process.spawn",
            executable.display().to_string(),
            serde_json::json!({
                "cwd": directory.path(),
                "args": [],
                "environment": {},
                "stdin_base64": null,
            }),
        );
        request.capabilities = vec!["process.spawn".into()];
        assert!(matches!(
            gateway.execute(request, &executor).await,
            Err(GatewayError::Safety(_))
        ));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn networked_oci_process_reserves_proxy_and_container_cleanup_time() {
        let directory = tempfile::tempdir().expect("directory");
        let executable = std::env::current_exe()
            .expect("executable")
            .canonicalize()
            .expect("canonical executable");
        let policy = BuiltInPolicy::offline_default()
            .with_action("process.spawn", DecisionOutcome::Allow)
            .with_sandbox("oci", "test", false)
            .with_limits(9_999, 1024, 2, 64 * 1024 * 1024, 1)
            .with_filesystem_root(executable.display().to_string(), "execute")
            .with_filesystem_read_root(directory.path().display().to_string())
            .with_network_destination("https://example.com");
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new(["process.spawn".into()]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let mut request = effect_request(
            system_actor("test"),
            "process.spawn",
            executable.display().to_string(),
            serde_json::json!({
                "cwd": directory.path(),
                "args": [],
                "environment": {},
                "stdin_base64": null,
            }),
        );
        request.capabilities = vec!["process.spawn".into()];
        assert!(matches!(
            gateway.execute(request, &executor).await,
            Err(GatewayError::Safety(_))
        ));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn oci_executable_identity_is_an_exact_normalized_image_path() {
        let directory = tempfile::tempdir().expect("directory");
        let policy = BuiltInPolicy::offline_default()
            .with_action("process.spawn", DecisionOutcome::Allow)
            .with_sandbox("oci", "test", false)
            .with_limits(10_000, 1024, 2, 64 * 1024 * 1024, 1)
            .with_filesystem_root("/image/bin/tool", "execute")
            .with_filesystem_read_root(directory.path().display().to_string());
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new(["process.spawn".into()]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let mut request = effect_request(
            system_actor("test"),
            "process.spawn",
            "/image/bin/tool",
            serde_json::json!({
                "cwd": directory.path(),
                "args": [],
                "environment": {},
                "stdin_base64": null,
            }),
        );
        request.capabilities = vec!["process.spawn".into()];
        gateway
            .execute(request, &executor)
            .await
            .expect("exact image path");
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);

        let mut request = effect_request(
            system_actor("test"),
            "process.spawn",
            "/image/../image/bin/tool",
            serde_json::json!({
                "cwd": directory.path(),
                "args": [],
                "environment": {},
                "stdin_base64": null,
            }),
        );
        request.capabilities = vec!["process.spawn".into()];
        assert!(matches!(
            gateway.execute(request, &executor).await,
            Err(GatewayError::Safety(_))
        ));
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn network_origin_not_in_obligations_never_reaches_adapter() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy =
            BuiltInPolicy::offline_default().with_action("network.http", DecisionOutcome::Allow);
        let gateway = EffectGateway::new(
            journal,
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new(["network.http".into()]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let mut request = effect_request(
            system_actor("test"),
            "network.http",
            "https://example.com/path",
            serde_json::json!({"method": "GET", "headers": {}}),
        );
        request.capabilities = vec!["network.http".into()];
        assert!(matches!(
            gateway.execute(request, &executor).await,
            Err(GatewayError::Safety(_))
        ));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn approval_is_reevaluated_before_execution() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action("filesystem.write", DecisionOutcome::RequireApproval)
            .with_filesystem_root("/tmp", "write");
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new([]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let result = gateway
            .execute(
                effect_request(
                    system_actor("test"),
                    "filesystem.write",
                    "/tmp/x",
                    serde_json::json!({"content":"x"}),
                ),
                &executor,
            )
            .await
            .expect("allow after proof");
        assert_eq!(result.bytes, b"ok");
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
        let names = journal
            .read_global(1, 20)
            .expect("events")
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(names.contains(&"approval.granted.v1".into()));
        assert_eq!(
            names
                .iter()
                .filter(|name| name.as_str() == "policy.decided.v1")
                .count(),
            2
        );
    }

    struct PostDenyPolicy;

    #[async_trait]
    impl colossus_ports::PolicyDecisionPoint for PostDenyPolicy {
        async fn decide(
            &self,
            request: &colossus_contracts::EffectRequest,
        ) -> Result<colossus_contracts::PolicyDecision, colossus_ports::PolicyError> {
            let mut obligations = super::default_obligations();
            obligations.require_post_effect = true;
            Ok(colossus_contracts::PolicyDecision {
                decision_id: uuid::Uuid::now_v7().to_string(),
                policy_revision: "test".into(),
                outcome: if request.phase == colossus_contracts::EffectPhase::PostEffect {
                    DecisionOutcome::Deny
                } else {
                    DecisionOutcome::Allow
                },
                reason: "test".into(),
                obligations,
            })
        }

        async fn doctor(&self) -> Result<serde_json::Value, colossus_ports::PolicyError> {
            Ok(serde_json::json!({"ready":true}))
        }
    }

    #[tokio::test]
    async fn denied_post_effect_content_is_not_released() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(PostDenyPolicy),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new([]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let error = gateway
            .execute(
                effect_request(
                    system_actor("test"),
                    "provider.remote",
                    "https://example.test",
                    serde_json::json!({"prompt":"x"}),
                ),
                &executor,
            )
            .await
            .expect_err("post deny");
        assert!(matches!(error, GatewayError::Denied(_)));
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
    }

    struct RecordingPolicy {
        request: Arc<Mutex<Option<colossus_contracts::EffectRequest>>>,
    }

    #[async_trait]
    impl colossus_ports::PolicyDecisionPoint for RecordingPolicy {
        async fn decide(
            &self,
            request: &colossus_contracts::EffectRequest,
        ) -> Result<colossus_contracts::PolicyDecision, colossus_ports::PolicyError> {
            *self.request.lock().expect("recording policy lock") = Some(request.clone());
            Ok(colossus_contracts::PolicyDecision {
                decision_id: uuid::Uuid::now_v7().to_string(),
                policy_revision: "recording-v1".into(),
                outcome: DecisionOutcome::Allow,
                reason: "test allow".into(),
                obligations: super::default_obligations(),
            })
        }

        async fn doctor(&self) -> Result<serde_json::Value, colossus_ports::PolicyError> {
            Ok(serde_json::json!({"ready":true}))
        }
    }

    #[tokio::test]
    async fn hard_secrets_are_hashed_before_policy_disclosure() {
        let seen = Arc::new(Mutex::new(None));
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(RecordingPolicy {
                request: Arc::clone(&seen),
            }),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new([]),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        gateway
            .execute(
                effect_request(
                    system_actor("test"),
                    "provider.remote",
                    "provider:test",
                    serde_json::json!({
                        "message": "safe",
                        "api_key": "must-not-leak",
                        "headers": {"authorization": "Bearer secret"}
                    }),
                ),
                &executor,
            )
            .await
            .expect("allowed");
        let request = seen
            .lock()
            .expect("seen lock")
            .clone()
            .expect("policy request");
        assert_eq!(request.content["message"], "safe");
        assert_eq!(request.content["api_key"]["redacted"], true);
        assert_eq!(
            request.content["headers"]["authorization"]["redacted"],
            true
        );
        assert!(
            !serde_json::to_string(&request)
                .expect("request json")
                .contains("must-not-leak")
        );
    }

    #[tokio::test]
    async fn oversized_request_is_audited_and_fails_closed() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(BuiltInPolicy::offline_default()),
            Arc::new(AllowApproval {
                approved_by: "user".into(),
            }),
            SafetyKernel::new([]).with_policy_input_limit(256),
            [9_u8; 32],
        );
        let executor = CountingExecutor {
            calls: AtomicUsize::new(0),
        };
        let error = gateway
            .execute(
                effect_request(
                    system_actor("test"),
                    "provider.echo",
                    "provider:echo",
                    serde_json::json!({"message": "x".repeat(1024)}),
                ),
                &executor,
            )
            .await
            .expect_err("oversized deny");
        assert!(matches!(error, GatewayError::Policy(_)));
        assert_eq!(executor.calls.load(Ordering::Acquire), 0);
        let names = journal
            .read_global(1, 10)
            .expect("audit events")
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["effect.requested.v1", "effect.denied.v1"]);
    }

    fn one_shot_opa(response: serde_json::Value) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("OPA test listener");
        let address = listener.local_addr().expect("OPA test address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("OPA request");
            let mut request = [0_u8; 16 * 1024];
            let read = stream.read(&mut request).expect("read OPA request");
            assert!(String::from_utf8_lossy(&request[..read]).contains("/v1/data/colossus/effect"));
            let body = serde_json::to_vec(&response).expect("OPA response JSON");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("OPA response headers");
            stream.write_all(&body).expect("OPA response body");
        });
        (format!("http://{address}/"), handle)
    }

    fn local_opa_config(base_url: String) -> super::OpaConfig {
        super::OpaConfig {
            base_url,
            decision_path: "colossus/effect".into(),
            ca_pem: None,
            identity_pem: None,
            full_content_disclosure_acknowledged: true,
            decision_log_masking_verified: false,
            timeout: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn opa_adapter_accepts_strict_decisions_and_rejects_invalid_responses() {
        let decision = colossus_contracts::PolicyDecision {
            decision_id: "opa-decision".into(),
            policy_revision: "bundle-42".into(),
            outcome: DecisionOutcome::Allow,
            reason: "test".into(),
            obligations: super::default_obligations(),
        };
        let (url, server) = one_shot_opa(serde_json::json!({"result": decision}));
        let policy = super::OpaPolicy::new(local_opa_config(url)).expect("OPA policy");
        let result = colossus_ports::PolicyDecisionPoint::decide(
            &policy,
            &effect_request(
                system_actor("test"),
                "provider.echo",
                "provider:echo",
                serde_json::json!({"message":"ok"}),
            ),
        )
        .await
        .expect("strict OPA decision");
        server.join().expect("OPA server");
        assert_eq!(result.policy_revision, "bundle-42");

        let (url, server) = one_shot_opa(serde_json::json!({
            "result": {"decision_id":"missing-everything-else"}
        }));
        let policy = super::OpaPolicy::new(local_opa_config(url)).expect("OPA policy");
        let error = colossus_ports::PolicyDecisionPoint::decide(
            &policy,
            &effect_request(
                system_actor("test"),
                "provider.echo",
                "provider:echo",
                serde_json::json!({"message":"ok"}),
            ),
        )
        .await
        .expect_err("invalid response");
        server.join().expect("OPA server");
        assert!(matches!(
            error,
            colossus_ports::PolicyError::InvalidDecision(_)
        ));
    }

    #[test]
    fn remote_opa_requires_disclosure_https_pinned_trust_and_mtls() {
        let mut config = local_opa_config("https://opa.example.test/".into());
        config.full_content_disclosure_acknowledged = false;
        assert!(matches!(
            super::OpaPolicy::new(config),
            Err(colossus_ports::PolicyError::InvalidDecision(_))
        ));

        let config = local_opa_config("http://opa.example.test/".into());
        assert!(matches!(
            super::OpaPolicy::new(config),
            Err(colossus_ports::PolicyError::InvalidDecision(_))
        ));

        let config = local_opa_config("https://opa.example.test/".into());
        assert!(matches!(
            super::OpaPolicy::new(config),
            Err(colossus_ports::PolicyError::InvalidDecision(_))
        ));
    }
}
