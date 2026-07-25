use super::*;

/// Provider-neutral assistant tool call preserved between turns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolCall {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered tool name.
    pub name: String,
    /// Validated object arguments.
    pub arguments: Value,
}

/// One strict function tool exposed to a provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolDefinition {
    /// Stable tool name.
    pub name: String,
    /// Bounded human-readable description.
    pub description: String,
    /// JSON Schema for object arguments.
    pub input_schema: Value,
}

/// Application tool specification and effect identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// Stable model-visible name.
    pub name: String,
    /// Bounded model-visible description.
    pub description: String,
    /// Strict JSON Schema for object arguments.
    pub input_schema: Value,
    /// Effect action when the tool is effectful; absent for pure computation.
    pub effect_action: Option<String>,
    /// Requested capability identity when effectful.
    pub capability: Option<String>,
    /// Tool-specific normalized output ceiling.
    pub max_output_bytes: u64,
}

/// One validated model-requested tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered tool name.
    pub name: String,
    /// Strict object arguments.
    pub arguments: Value,
}

/// One bounded tool result suitable for provider continuation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered tool name.
    pub name: String,
    /// Bounded UTF-8 or JSON output.
    pub output: String,
    /// Conventional zero-success exit code.
    pub exit_code: i32,
}

/// Stable phase of one application-level agent run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// Preparing durable context and the next model request.
    Preparing,
    /// Waiting for a policy-authorized provider turn.
    WaitingForModel,
    /// Releasing visible assistant output.
    Responding,
    /// The operator requested a cooperative stop and the current effect is settling.
    Cancelling,
    /// The run reached a durable operator-cancelled terminal state.
    Cancelled,
    /// The run reached a durable successful terminal state.
    Completed,
}

/// Safe ordered event released by the application-level agent runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunEvent {
    /// One normalized provider event released through post-effect policy.
    Provider {
        /// Safe provider event; never a raw provider frame.
        event: ProviderEvent,
    },
    /// Durable run phase orientation.
    Phase {
        /// Current run phase.
        phase: RunPhase,
        /// One-based model turn when applicable.
        turn: Option<u16>,
        /// Optional safe current action.
        action: Option<String>,
        /// Wall time since the run began.
        elapsed_seconds: f64,
    },
    /// Validated tool call immediately before execution.
    ToolStarted {
        /// One-based model turn.
        turn: u16,
        /// Strict released tool call.
        call: ToolCall,
        /// Wall time since the run began.
        elapsed_seconds: f64,
    },
    /// Policy-released tool result after durable completion recording.
    ToolCompleted {
        /// One-based model turn.
        turn: u16,
        /// Bounded result suitable for provider continuation.
        result: ToolResult,
        /// Wall time spent executing this tool call.
        duration_seconds: f64,
        /// Wall time since the run began.
        elapsed_seconds: f64,
    },
    /// A validated model-requested tool was not executed because cancellation won first.
    ToolCancelled {
        /// One-based model turn.
        turn: u16,
        /// Strict released tool call that was skipped.
        call: ToolCall,
        /// Wall time since the run began.
        elapsed_seconds: f64,
    },
    /// Durable recoverable or terminal run error.
    Error {
        /// Stable error category.
        code: String,
        /// Bounded user-safe message.
        message: String,
        /// Whether the failure is safe for bounded recovery or an explicit caller retry.
        recoverable: bool,
        /// HTTP response status when the failure came from an upstream response.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_status: Option<u16>,
        /// Bounded provider retry lower bound when supplied.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<u64>,
        /// One-based model turn when applicable.
        turn: Option<u16>,
        /// Wall time since the run began.
        elapsed_seconds: f64,
    },
}

/// Correlated application-level run event delivered to embedded and worker clients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEventEnvelope {
    /// Envelope schema version.
    pub schema_version: u16,
    /// Stable UUIDv7 run identifier.
    pub run_id: String,
    /// Durable session containing the run.
    pub session_id: String,
    /// Ordered safe event.
    pub event: RunEvent,
}

/// Bounded model-authored question presented only by an interactive interface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserPromptRequest {
    /// User-visible question.
    pub question: String,
    /// Optional bounded suggested answers.
    pub choices: Vec<String>,
    /// Whether an answer outside the choices is accepted.
    pub allow_free_form: bool,
}

/// User answer returned to the model as an ordinary bounded tool result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserPromptResponse {
    /// Exact bounded answer supplied by the user.
    pub answer: String,
    /// Selected zero-based choice index, when a suggestion was selected.
    pub selected_index: Option<usize>,
}

/// Provider-neutral request for one model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    /// Trusted instructions supplied separately from conversation messages.
    pub instructions: String,
    /// Ordered conversation messages.
    pub messages: Vec<ModelMessage>,
    /// Strict tools available for this turn.
    pub tools: Vec<ModelToolDefinition>,
    /// Optional caller ceiling which may only narrow the configured model maximum.
    pub max_output_tokens: Option<u64>,
}

/// Explicit model capabilities used to shape a request before provider execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCapabilities {
    /// Whether this model may receive tools and continue structured tool history.
    pub tool_calls: bool,
    /// Whether this model should use the provider's streaming transport.
    pub streaming: bool,
}

/// Declared and effective token limits for one configured model profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelLimits {
    /// Total provider context window.
    pub context_window_tokens: u64,
    /// Maximum configured output allocation.
    pub max_output_tokens: u64,
    /// Conservative reserve held outside both input and output allocations.
    pub safety_margin_tokens: u64,
    /// Effective provider-visible input budget after output and safety reservations.
    pub input_budget_tokens: u64,
}

/// Provider-neutral token accounting for one model turn.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUsage {
    /// Tokens supplied to the model, including cached tokens when reported.
    pub input_tokens: u64,
    /// Tokens generated by the model, including reasoning tokens when reported.
    pub output_tokens: u64,
    /// Provider-reported total tokens.
    pub total_tokens: u64,
    /// Input tokens served from a provider cache when available.
    pub cached_input_tokens: Option<u64>,
    /// Hidden reasoning tokens billed inside output tokens when available.
    pub reasoning_tokens: Option<u64>,
}

/// Provider-neutral safe event produced by one model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderEvent {
    /// Visible text delta. Non-streaming providers emit one complete delta.
    ModelDelta {
        /// Visible text.
        text: String,
    },
    /// Provider-declared safe reasoning summary, never hidden reasoning.
    ReasoningSummary {
        /// Bounded safe summary.
        summary: String,
    },
    /// Strict validated tool call request.
    ToolCallRequested {
        /// Provider call identifier.
        call_id: String,
        /// Registered tool name.
        name: String,
        /// Parsed JSON object arguments.
        arguments: Value,
    },
    /// Final visible assistant text.
    FinalOutput {
        /// Complete visible output.
        text: String,
    },
    /// Final bounded token accounting for the turn.
    Usage {
        /// Normalized provider accounting.
        usage: ProviderUsage,
    },
}

/// Ordered item released from a streaming provider effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderStreamItem {
    /// One safe normalized provider event.
    Event {
        /// Event released through per-chunk post-effect policy.
        event: ProviderEvent,
    },
    /// Terminal metadata proving the provider stream completed normally.
    Completed {
        /// Deprecated compatibility alias populated with the model profile.
        profile: String,
        /// Configured model profile.
        model_profile: String,
        /// Configured provider connection profile.
        provider_profile: String,
        /// Provider adapter kind.
        provider: String,
        /// Model identifier used by the request.
        model: String,
        /// Provider response identifier when supplied.
        response_id: Option<String>,
    },
}

/// Normalized result of one provider turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTurn {
    /// Deprecated compatibility alias populated with the model profile.
    pub profile: String,
    /// Configured model profile.
    pub model_profile: String,
    /// Configured provider connection profile.
    pub provider_profile: String,
    /// Provider adapter kind.
    pub provider: String,
    /// Model identifier used by the request.
    pub model: String,
    /// Provider response identifier when supplied.
    pub response_id: Option<String>,
    /// Ordered safe normalized events.
    pub events: Vec<ProviderEvent>,
}

/// Resolved model route metadata without credentials.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRoute {
    /// Requested logical role.
    pub role: String,
    /// Deprecated compatibility alias populated with the model profile.
    pub profile: String,
    /// Resolved model profile.
    pub model_profile: String,
    /// Resolved provider connection profile.
    pub provider_profile: String,
    /// Provider adapter kind.
    pub provider: String,
    /// Configured model identifier.
    pub model: String,
    /// Declared and effective token limits.
    pub limits: ModelLimits,
    /// Explicit request-shaping capabilities.
    pub capabilities: ModelCapabilities,
}

/// Compatibility name retained for callers compiled against the provider-centric route API.
pub type ProviderRoute = ModelRoute;

/// Provider-neutral bounded web-search request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    /// Search query supplied to the configured provider.
    pub query: String,
    /// Maximum normalized results, constrained to 1 through 20.
    pub limit: usize,
}

/// One normalized ranked web-search result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    /// One-based provider result rank after normalization.
    pub rank: usize,
    /// Bounded human-readable result title.
    pub title: String,
    /// Credential-free HTTP(S) result URL.
    pub url: String,
    /// Bounded untrusted provider snippet.
    pub snippet: String,
    /// Optional provider-reported source or engine label.
    pub source: Option<String>,
}

/// Provider-neutral normalized response released after policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResponse {
    /// Original validated query.
    pub query: String,
    /// Number of normalized results.
    pub count: usize,
    /// Ranked bounded results.
    pub results: Vec<SearchResult>,
}

/// Resolved search role metadata without credentials.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRoute {
    /// Requested logical role such as `agent` or `research`.
    pub role: String,
    /// Resolved profile name.
    pub profile: String,
    /// Search adapter kind.
    pub provider: String,
}

/// Safe configured search-profile summary for operator diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchProfileSummary {
    /// Stable profile name.
    pub profile: String,
    /// Search adapter kind.
    pub provider: String,
    /// Credential-free configured endpoint.
    pub endpoint: String,
    /// Credential reference without its value.
    pub credential_reference: Option<String>,
    /// Per-request transport timeout.
    pub timeout_ms: u64,
}

/// One model visible through a provider catalog endpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderModelInfo {
    /// Provider model identifier.
    pub id: String,
    /// Provider object/type label when supplied.
    pub object: Option<String>,
    /// Owning organization when supplied.
    pub owned_by: Option<String>,
}

/// One bounded provider readiness check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderReadinessCheck {
    /// Stable check name.
    pub name: String,
    /// `pass`, `fail`, `not_checked`, or `not_applicable`.
    pub status: String,
    /// Bounded detail without credentials or response bodies.
    pub detail: String,
}

/// Provider profile readiness and capability report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderReadiness {
    /// Profile name.
    pub profile: String,
    /// Adapter kind.
    pub provider: String,
    /// Whether every required check passed.
    pub ready: bool,
    /// Whether the provider supports strict tool calls.
    pub tool_calls: bool,
    /// Whether the provider supports transport streaming.
    pub streaming: bool,
    /// Ordered bounded checks.
    pub checks: Vec<ProviderReadinessCheck>,
}

/// Result of one application-level model run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunResult {
    /// Stable UUIDv7 run identifier.
    pub run_id: String,
    /// Durable session containing this run's conversation messages.
    pub session_id: Option<String>,
    /// Selected model role.
    pub role: String,
    /// Deprecated compatibility alias populated with the model profile.
    pub profile: String,
    /// Resolved model profile.
    pub model_profile: String,
    /// Resolved provider connection profile.
    pub provider_profile: String,
    /// Model used for the turn.
    pub model: String,
    /// Complete visible assistant output.
    pub output: String,
    /// Number of events durably recorded on the run stream, including preparation.
    pub event_count: u64,
    /// Elapsed wall time in fractional seconds.
    pub elapsed_seconds: f64,
}

/// Durable result of a cooperatively cancelled application run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunCancellation {
    /// Stable UUIDv7 run identifier.
    pub run_id: String,
    /// Durable session containing any completed work before cancellation.
    pub session_id: String,
    /// One-based turn at which cancellation became terminal.
    pub turn: u16,
    /// Number of events durably recorded on the run stream.
    pub event_count: u64,
    /// Elapsed wall time in fractional seconds.
    pub elapsed_seconds: f64,
}

/// Terminal outcome of a run that supports cooperative cancellation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentRunOutcome {
    /// The run produced a normal released assistant result.
    Completed {
        /// Existing stable run result contract.
        result: AgentRunResult,
    },
    /// The operator cancelled before another external effect began.
    Cancelled {
        /// Durable cancellation evidence.
        result: AgentRunCancellation,
    },
}

/// Complete, versioned request sent to a policy decision point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRequest {
    /// Contract schema version.
    pub schema_version: u16,
    /// Stable request identifier.
    pub request_id: String,
    /// Actor and provenance.
    pub actor: Actor,
    /// Canonical action name.
    pub action: String,
    /// Canonical resource name.
    pub resource: String,
    /// Requested capability identities.
    pub capabilities: Vec<String>,
    /// Non-authoritative declared risk.
    pub risk: RiskInput,
    /// Complete proposed logical request or quarantined result content.
    pub content: Value,
    /// Credential references with values removed.
    pub credential_references: Vec<CredentialReference>,
    /// Correlation and workflow context.
    pub context: ExecutionContext,
    /// Optional idempotency key.
    pub idempotency_id: Option<String>,
    /// Pre-execution or post-result phase.
    pub phase: EffectPhase,
    /// Approval proof supplied only during policy re-evaluation.
    pub approval: Option<ApprovalProof>,
}

/// Evidence that a specific actor approved a specific request hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalProof {
    /// Approval identifier.
    pub approval_id: String,
    /// Request hash the user saw.
    pub request_hash: String,
    /// Approving actor identifier.
    pub approved_by: String,
    /// UTC RFC3339 approval time.
    pub approved_at: String,
}

/// Non-overridable and policy-provided execution obligations.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyObligations {
    /// Required sandbox backend.
    pub sandbox_backend: String,
    /// Required sandbox profile.
    pub sandbox_profile: String,
    /// Canonical filesystem roots and access modes.
    pub filesystem: Vec<FilesystemGrant>,
    /// Canonical paths that writable process sandboxes must keep inaccessible.
    #[serde(default)]
    pub protected_filesystem: Vec<String>,
    /// Allowed network destination patterns.
    pub network_destinations: Vec<String>,
    /// Exact environment variable names visible to a sandboxed process.
    pub allowed_environment: Vec<String>,
    /// Whether an unavailable isolation backend may downgrade to the broker.
    pub allow_sandbox_downgrade: bool,
    /// Maximum wall-clock time in milliseconds.
    pub timeout_ms: u64,
    /// Maximum released output bytes.
    pub max_output_bytes: u64,
    /// Maximum child process count.
    pub max_processes: u32,
    /// Maximum memory bytes.
    pub max_memory_bytes: u64,
    /// Maximum effect concurrency for the actor/run.
    pub max_concurrency: u32,
    /// Fields or patterns that must be redacted.
    pub required_redactions: Vec<String>,
    /// Whether output must pass a post-effect decision.
    pub require_post_effect: bool,
    /// Labels written to audit events.
    pub audit_labels: BTreeMap<String, String>,
    /// Required retention category.
    pub retention: String,
}

/// A canonical filesystem grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemGrant {
    /// Canonical absolute root.
    pub root: String,
    /// `read`, `write`, `metadata`, or `execute`.
    pub mode: String,
}

/// Strict policy decision returned by built-in or OPA policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDecision {
    /// Stable decision identifier.
    pub decision_id: String,
    /// Policy or bundle revision.
    pub policy_revision: String,
    /// Strict decision outcome.
    pub outcome: DecisionOutcome,
    /// Human-readable bounded reason.
    pub reason: String,
    /// Complete recognized obligations.
    pub obligations: PolicyObligations,
}

/// Bounded adapter output held until optional post-effect authorization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuarantinedEffectResult {
    /// Media type of the captured bytes.
    pub media_type: String,
    /// Captured bytes, not yet released to the requester.
    pub bytes: Vec<u8>,
    /// Whether the adapter believes the external effect succeeded.
    pub effect_succeeded: bool,
}
