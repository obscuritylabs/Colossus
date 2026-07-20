use colossus_api::IdempotencyKey;

/// Requested public execution mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    /// Permit policy-authorized effects.
    Execute,
    /// Block implementation and external mutation; local planning records may be created.
    Plan,
}

/// Durable public run lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    /// Durable but not yet executing.
    Queued,
    /// Actively executing.
    Running,
    /// Waiting for a caller interaction.
    Waiting,
    /// Cooperative cancellation was requested.
    Cancelling,
    /// Released output is terminal.
    Completed,
    /// A known failure is terminal.
    Failed,
    /// Cancellation is terminal.
    Cancelled,
    /// The runtime stopped before completion.
    Interrupted,
    /// An effect may have occurred without trustworthy terminal evidence.
    OutcomeUnknown,
}

/// Whether an effectful outcome is known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeCertainty {
    /// Durable evidence establishes the outcome.
    Known,
    /// The outcome may be external and must not be retried automatically.
    Unknown,
}

/// One v1alpha1 run input part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputContentPart {
    /// Visible user text.
    Text(String),
}

/// Request to create one durable run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateRunRequest {
    /// Ordered initial visible content.
    pub input: Vec<InputContentPart>,
    /// Existing canonical session, or absent to allocate a durable run-owned identity.
    ///
    /// Cancellation before execution starts can leave only that run-owned identity.
    pub session_id: Option<String>,
    /// Logical role; an empty string selects the server default.
    pub role: String,
    /// Requested execution mode.
    pub mode: RunMode,
    /// Declarative skill identities; these do not grant capabilities.
    pub selected_skills: Vec<String>,
    /// Model-turn ceiling; zero selects the configured default.
    pub max_turns: u32,
    /// Required caller-scoped idempotency key.
    pub idempotency_key: IdempotencyKey,
}

/// Bounded released terminal output.
#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    /// Complete visible assistant output.
    pub output: String,
    /// Credential-free provider profile.
    pub profile: String,
    /// Provider model identifier.
    pub model: String,
    /// Finite non-negative elapsed wall time.
    pub elapsed_seconds: f64,
}

/// Bounded user-safe terminal failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunFailure {
    /// Stable machine-readable reason.
    pub reason: String,
    /// Safe released message.
    pub message: String,
    /// External outcome certainty.
    pub outcome_certainty: OutcomeCertainty,
}

/// Durable cancellation evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCancellation {
    /// Turn at which cancellation became terminal; zero means before turn one.
    pub turn: u32,
    /// Bounded cancellation summary.
    pub message: String,
}

/// Exactly one terminal run payload.
#[derive(Clone, Debug, PartialEq)]
pub enum RunTerminal {
    /// Successful released result.
    Result(RunResult),
    /// Known or outcome-unknown failure.
    Failure(RunFailure),
    /// Cooperative cancellation evidence.
    Cancellation(RunCancellation),
}

/// Durable bounded run summary.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// Stable run identifier.
    pub run_id: String,
    /// Durable session identity associated with the run.
    pub session_id: String,
    /// Selected logical role.
    pub role: String,
    /// Requested execution mode.
    pub mode: RunMode,
    /// Current lifecycle state.
    pub status: RunStatus,
    /// UTC RFC3339 allocation time.
    pub created_at: String,
    /// UTC RFC3339 latest update time.
    pub updated_at: String,
    /// UTC RFC3339 execution start time.
    pub started_at: Option<String>,
    /// UTC RFC3339 terminal time.
    pub finished_at: Option<String>,
    /// Highest durable released feed sequence.
    pub last_sequence: u64,
    /// Number of pending caller-visible interactions.
    pub pending_interaction_count: u32,
    /// Exactly one terminal payload when terminal.
    pub terminal: Option<RunTerminal>,
    /// Opaque optimistic-concurrency token.
    pub etag: String,
    /// Reserved skill identities; always empty for public v1alpha1 runs.
    pub selected_skills: Vec<String>,
}

/// Response from durable run creation.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateRunResponse {
    /// Allocated durable run.
    pub run: Run,
}

/// Request for one run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetRunRequest {
    /// Exact stable run identifier.
    pub run_id: String,
}

/// Response containing one run and its currently pending interactions.
#[derive(Clone, Debug, PartialEq)]
pub struct GetRunResponse {
    /// Current bounded run summary.
    pub run: Run,
    /// Caller-visible unanswered interactions.
    pub pending_interactions: Vec<Interaction>,
}

/// Bounded stable page request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageRequest {
    /// Requested page size; zero selects the server default.
    pub page_size: u32,
    /// Opaque server-issued continuation token.
    pub page_token: String,
}

/// Stable page continuation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageResponse {
    /// Opaque next-page token, empty when the page is terminal.
    pub next_page_token: String,
}

/// Stable run listing request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListRunsRequest {
    /// Optional exact session filter.
    pub session_id: Option<String>,
    /// Empty includes every status.
    pub statuses: Vec<RunStatus>,
    /// Optional bounded page request.
    pub page: Option<PageRequest>,
}

/// Stable page of run summaries.
#[derive(Clone, Debug, PartialEq)]
pub struct ListRunsResponse {
    /// Deterministically ordered run summaries.
    pub runs: Vec<Run>,
    /// Page continuation.
    pub page: Option<PageResponse>,
}

/// Replay-and-tail request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchRunRequest {
    /// Exact run identifier.
    pub run_id: String,
    /// Exclusive durable replay cursor.
    pub after_sequence: u64,
}

/// Idempotent cooperative cancellation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelRunRequest {
    /// Exact run identifier.
    pub run_id: String,
    /// Required caller-scoped idempotency key.
    pub idempotency_key: IdempotencyKey,
}

/// Cancellation response.
#[derive(Clone, Debug, PartialEq)]
pub struct CancelRunResponse {
    /// Current durable run summary.
    pub run: Run,
}

/// Caller-visible interaction class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionKind {
    /// Ordinary user input.
    UserPrompt,
    /// Explicit effect approval decision.
    Approval,
}

/// Durable interaction lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionStatus {
    /// Awaiting one response.
    Pending,
    /// A response consumed this interaction.
    Answered,
    /// The response window elapsed.
    Expired,
    /// Run cancellation closed the interaction.
    Cancelled,
}

/// Exact suggested prompt choice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptChoice {
    /// Opaque server-issued identifier that must be echoed unchanged.
    pub choice_id: String,
    /// Exact displayed label that must be echoed unchanged.
    pub label: String,
}

/// Released ordinary prompt content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserPromptInteraction {
    /// User-visible question.
    pub question: String,
    /// Optional exact suggestions.
    pub choices: Vec<PromptChoice>,
    /// Whether alternative bounded text is permitted.
    pub allow_free_form: bool,
}

/// Non-authoritative approval risk metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalRisk {
    /// Bounded reversible impact.
    Low,
    /// Meaningful local or external impact.
    Medium,
    /// Sensitive, destructive, or hard-to-reverse impact.
    High,
}

/// Released approval content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalInteraction {
    /// Released reason approval is required.
    pub reason: String,
    /// Canonical action.
    pub action: String,
    /// Canonical public resource.
    /// Sanitized display origin or opaque resource class, never a raw effect target.
    pub resource: String,
    /// Optional non-authoritative risk metadata.
    pub risk: Option<ApprovalRisk>,
    /// Randomized one-use approval binding that an answer must echo.
    pub request_hash: String,
}

/// Exactly one released interaction representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionContent {
    /// Ordinary user prompt.
    UserPrompt(UserPromptInteraction),
    /// Effect approval.
    Approval(ApprovalInteraction),
}

/// Durable caller-bound prompt or approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Interaction {
    /// Opaque one-use identifier.
    pub interaction_id: String,
    /// Owning run identifier.
    pub run_id: String,
    /// Interaction class.
    pub kind: InteractionKind,
    /// Durable lifecycle.
    pub status: InteractionStatus,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// UTC RFC3339 expiry.
    pub expires_at: String,
    /// Whether this authenticated caller may currently respond.
    pub respondable_by_caller: bool,
    /// Opaque response concurrency token; empty when not respondable.
    pub etag: String,
    /// Released prompt or approval.
    pub content: InteractionContent,
}

/// Prompt answer that preserves opaque choice integrity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptAnswer {
    /// Select and echo one exact displayed choice.
    Choice(PromptChoice),
    /// Alternative bounded text when free form is permitted.
    FreeForm(String),
}

/// One-use interaction answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionAnswer {
    /// Ordinary prompt answer.
    Prompt(PromptAnswer),
    /// Approval answer bound to the displayed randomized one-use value.
    Approval {
        /// Whether the request was approved.
        approved: bool,
        /// Exact randomized approval binding displayed to the user.
        request_hash: String,
    },
}

/// Request that consumes one interaction exactly once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RespondInteractionRequest {
    /// Owning run.
    pub run_id: String,
    /// Exact one-use interaction identifier.
    pub interaction_id: String,
    /// Exact pending-interaction concurrency token.
    pub etag: String,
    /// Required caller-scoped idempotency key.
    pub idempotency_key: IdempotencyKey,
    /// Kind-matched response.
    pub response: InteractionAnswer,
}

/// Response after consuming one interaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RespondInteractionResponse {
    /// Terminal interaction with an empty response etag.
    pub interaction: Interaction,
}

/// Released tool activity state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolActivityState {
    /// Strict call validated.
    Requested,
    /// Waiting for policy approval.
    WaitingApproval,
    /// Permit-bound execution began.
    Started,
    /// Known successful terminal state.
    Completed,
    /// Known failed terminal state.
    Failed,
    /// Effect may have occurred.
    OutcomeUnknown,
}

/// Bounded tool lifecycle metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolActivity {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered public tool name.
    pub tool_name: String,
    /// Released state.
    pub state: ToolActivityState,
    /// Bounded summary without arguments or quarantined output.
    pub summary: String,
}

/// Normalized provider accounting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenUsage {
    /// Input token count.
    pub input_tokens: u64,
    /// Output token count.
    pub output_tokens: u64,
    /// Provider total.
    pub total_tokens: u64,
    /// Cached input count when available.
    pub cached_input_tokens: Option<u64>,
    /// Reasoning token count when available; never reasoning content.
    pub reasoning_tokens: Option<u64>,
}

/// Released message role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    /// Human or application input.
    User,
    /// Released assistant output.
    Assistant,
    /// Released authorized tool result.
    Tool,
    /// Bounded public system notice.
    System,
}

/// Public artifact purpose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactPurpose {
    /// Agent run input.
    RunInput,
    /// Agent run output.
    RunOutput,
    /// Workflow content.
    Workflow,
    /// Extension content.
    Extension,
    /// Export archive.
    Archive,
}

/// Public artifact state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactState {
    /// Staged upload incomplete.
    Uploading,
    /// Private pending release.
    Quarantined,
    /// Released to this caller.
    Available,
    /// Rejected by verification or policy.
    Rejected,
    /// Staged upload expired.
    Expired,
}

/// Safe metadata for an opaque artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReference {
    /// Stable opaque identifier.
    pub artifact_id: String,
    /// Display name, never a server path.
    pub file_name: String,
    /// Normalized media type.
    pub media_type: String,
    /// Verified length.
    pub size_bytes: u64,
    /// Lowercase SHA-256.
    pub sha256: String,
    /// Validated purpose.
    pub purpose: ArtifactPurpose,
    /// Public release state.
    pub state: ArtifactState,
    /// UTC RFC3339 creation time.
    pub created_at: String,
}

/// Released message content part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageContentPart {
    /// Visible text.
    Text(String),
    /// Authorized opaque artifact.
    Artifact(ArtifactReference),
}

/// Newly durable released session message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMessage {
    /// Owning session.
    pub session_id: String,
    /// Associated run.
    pub run_id: String,
    /// One-based session message sequence.
    pub sequence: u64,
    /// Message provenance.
    pub role: MessageRole,
    /// Ordered released content.
    pub content: Vec<MessageContentPart>,
    /// UTC RFC3339 creation time.
    pub created_at: String,
}

/// One durable replayable run update.
#[derive(Clone, Debug, PartialEq)]
pub struct RunUpdate {
    /// Owning run identifier.
    pub run_id: String,
    /// Monotonic one-based replay sequence.
    pub sequence: u64,
    /// UTC RFC3339 durable creation time.
    pub created_at: String,
    /// Exactly one released update.
    pub update: RunUpdateKind,
}

/// Released public run-feed update.
#[derive(Clone, Debug, PartialEq)]
pub enum RunUpdateKind {
    /// Exact historical state transition.
    State(RunStatus),
    /// Incremental visible assistant text.
    OutputDelta(String),
    /// Provider-declared safe reasoning summary.
    ReasoningSummary(String),
    /// Bounded tool activity.
    ToolActivity(ToolActivity),
    /// Provider token accounting.
    Usage(TokenUsage),
    /// Prompt or approval lifecycle update.
    Interaction(Interaction),
    /// Newly durable released message.
    Message(SessionMessage),
    /// Bounded non-terminal notice.
    Notice {
        /// Stable machine-readable reason.
        reason: String,
        /// Bounded released message.
        message: String,
    },
    /// Successful terminal result.
    Result(RunResult),
    /// Failed terminal transition.
    Failure {
        /// Exact terminal state.
        status: RunStatus,
        /// Released failure.
        failure: RunFailure,
    },
    /// Terminal cancellation evidence.
    Cancellation(RunCancellation),
}

/// Transport-neutral stream of replayed and live public updates.
pub type RunUpdateStream = std::pin::Pin<
    Box<dyn futures::Stream<Item = Result<RunUpdate, colossus_api::ApiError>> + Send + 'static>,
>;
