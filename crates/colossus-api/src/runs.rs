use crate::{
    ApiError, ApiErrorReason, ApiResult, CallerContext, IdempotencyKey,
    validation::{
        MAX_IDENTIFIER_BYTES, MAX_INPUT_BYTES, MAX_INPUT_PARTS, MAX_PAGE_SIZE, MAX_ROLE_BYTES,
        bounded_text, token,
    },
};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use url::Url;

pub use colossus_contracts::RunBranchContextMode;

const MAX_PUBLIC_APPROVAL_ORIGIN_BYTES: usize = 512;
const MAX_RUN_TITLE_CHARACTERS: usize = 80;
const UNTITLED_RUN: &str = "Untitled work";

/// Authenticated server capability for exact-revision Plan continuation.
pub const PLAN_CONTINUATION_CAPABILITY: &str = "plans.continue";
/// Authenticated server capability for caller-scoped canonical session activity.
pub const SESSION_ACTIVITY_CAPABILITY: &str = "sessions.activity";

use std::{collections::BTreeMap, pin::Pin};

/// Persistent public run lifecycle state.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Accepted but not yet executing.
    Queued,
    /// Actively executing.
    Running,
    /// Waiting for a bound prompt or approval response.
    Waiting,
    /// Cooperative cancellation has been requested.
    Cancelling,
    /// Completed with a released result.
    Completed,
    /// Failed with a known terminal outcome.
    Failed,
    /// Cooperatively cancelled with a known terminal outcome.
    Cancelled,
    /// Interrupted by process loss before an external effect was uncertain.
    Interrupted,
    /// An external effect may have occurred and must not be retried automatically.
    OutcomeUnknown,
}

impl RunStatus {
    /// Return whether no later updates may be appended.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Failed
                | Self::Cancelled
                | Self::Interrupted
                | Self::OutcomeUnknown
        )
    }

    pub(super) fn permits(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Queued,
                Self::Running
                    | Self::Cancelling
                    | Self::Cancelled
                    | Self::Failed
                    | Self::Interrupted
            ) | (
                Self::Running,
                Self::Waiting
                    | Self::Cancelling
                    | Self::Completed
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Interrupted
                    | Self::OutcomeUnknown
            ) | (
                Self::Waiting,
                Self::Running
                    | Self::Cancelling
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Interrupted
                    | Self::OutcomeUnknown
            ) | (
                Self::Cancelling,
                Self::Completed
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Interrupted
                    | Self::OutcomeUnknown
            )
        )
    }
}

/// Requested application run mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Ordinary bounded model/tool execution.
    #[default]
    Execute,
    /// Block implementation and external mutation; local task/plan records remain available.
    Plan,
    /// Run the dedicated durable evidence-and-citation research service.
    Research,
}

/// Requested research breadth for a public Research run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchDepth {
    /// Fast, narrow evidence pass.
    Quick,
    /// Balanced default investigation.
    #[default]
    Standard,
    /// Broadest configured investigation.
    Deep,
}

/// Explicit evidence lane enabled for a public Research run.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchSourceKind {
    /// Selected Workspace repository evidence.
    Repo,
    /// Configured web-search evidence.
    Web,
    /// Configured MCP research evidence.
    Mcp,
}

/// Released canonical Plan lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// The exact revision remains editable and cannot execute yet.
    Draft,
    /// The exact revision was explicitly approved.
    Approved,
    /// The approved revision was consumed by direct or Goal execution.
    Executed,
    /// The revision was intentionally closed without execution.
    Discarded,
}

/// Requested execution strategy for one exact Plan revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanExecutionStrategy {
    /// Consume the approved Plan in one normal policy-bound run.
    Direct,
    /// Consume the approved Plan into bounded Goal Mode.
    Goal {
        /// Maximum autonomous Goal iterations.
        max_iterations: u16,
    },
}

/// Trusted continuation of a Plan produced by a caller-owned source run.
///
/// The source run, rather than a caller-supplied Plan identifier, is the authority
/// anchor. The runtime resolves its Plan and session and then applies the exact
/// optimistic revision supplied here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanRunAction {
    /// Refine one exact draft revision through structurally constrained Plan Mode.
    Revise {
        /// Caller-owned completed run that released the Plan reference.
        source_run_id: String,
        /// Exact Plan revision visible to the caller.
        expected_revision: u64,
    },
    /// Approve and consume one exact draft revision.
    Execute {
        /// Caller-owned completed run that released the Plan reference.
        source_run_id: String,
        /// Exact Plan revision visible to the caller.
        expected_revision: u64,
        /// Direct execution or bounded Goal Mode.
        strategy: PlanExecutionStrategy,
    },
}

impl PlanRunAction {
    /// Caller-owned run used to resolve the Plan and its session.
    pub fn source_run_id(&self) -> &str {
        match self {
            Self::Revise { source_run_id, .. } | Self::Execute { source_run_id, .. } => {
                source_run_id
            }
        }
    }

    /// Exact optimistic Plan revision visible when the action was chosen.
    pub fn expected_revision(&self) -> u64 {
        match self {
            Self::Revise {
                expected_revision, ..
            }
            | Self::Execute {
                expected_revision, ..
            } => *expected_revision,
        }
    }
}

/// Released terminal run result.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RunResult {
    /// Complete released assistant output.
    pub output: String,
    /// Canonical plan written by a completed Plan Mode run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Exact canonical Plan revision paired with `plan_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_revision: Option<u64>,
    /// Released lifecycle state paired with `plan_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_status: Option<PlanStatus>,
    /// Durable Goal created by a Plan handoff, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    /// Deprecated compatibility alias populated with the model profile.
    pub profile: String,
    /// Resolved model profile.
    pub model_profile: String,
    /// Resolved provider connection profile.
    pub provider_profile: String,
    /// Resolved model identifier.
    pub model: String,
    /// Elapsed wall time in fractional seconds.
    pub elapsed_seconds: f64,
}

#[derive(Default)]
struct LegacyRunResultProfile(Option<String>);

impl<'de> Deserialize<'de> for LegacyRunResultProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunResultFields {
    output: String,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    plan_revision: Option<u64>,
    #[serde(default)]
    plan_status: Option<PlanStatus>,
    #[serde(default)]
    goal_id: Option<String>,
    profile: String,
    #[serde(default)]
    model_profile: LegacyRunResultProfile,
    #[serde(default)]
    provider_profile: LegacyRunResultProfile,
    model: String,
    elapsed_seconds: f64,
}

impl<'de> Deserialize<'de> for RunResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let fields = RunResultFields::deserialize(deserializer)?;
        Ok(Self {
            output: fields.output,
            plan_id: fields.plan_id,
            plan_revision: fields.plan_revision,
            plan_status: fields.plan_status,
            goal_id: fields.goal_id,
            model_profile: fields
                .model_profile
                .0
                .unwrap_or_else(|| fields.profile.clone()),
            provider_profile: fields
                .provider_profile
                .0
                .unwrap_or_else(|| fields.profile.clone()),
            profile: fields.profile,
            model: fields.model,
            elapsed_seconds: fields.elapsed_seconds,
        })
    }
}

/// Released terminal failure without private adapter detail.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFailure {
    /// Stable machine-readable failure code.
    pub code: String,
    /// Bounded user-safe message.
    pub message: String,
    /// Whether durable evidence proves the external outcome.
    pub outcome: crate::OutcomeCertainty,
    /// Whether an explicit caller retry is known to be safe.
    #[serde(default)]
    pub recoverable: bool,
    /// Released upstream HTTP response status, when one was received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Provider-supplied retry lower bound in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// Durable terminal cancellation evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCancellation {
    /// Model turn at which cancellation became terminal; zero means before turn one.
    pub turn: u32,
    /// Bounded released cancellation summary.
    pub message: String,
    /// Canonical plan written before a cancelled Plan Mode run stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Exact canonical Plan revision paired with `plan_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_revision: Option<u64>,
    /// Released lifecycle state paired with `plan_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_status: Option<PlanStatus>,
    /// Durable Goal created by a Plan handoff, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
}

/// Released lifecycle state for one bounded tool activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolActivityState {
    /// A strict call was validated.
    Requested,
    /// Policy requires an approval before the call may start.
    WaitingApproval,
    /// Permit-bound execution began.
    Started,
    /// Released output reached a known successful terminal state.
    Completed,
    /// The call was settled without starting execution.
    Cancelled,
    /// The call reached a known failed terminal state.
    Failed,
    /// An effect started without trustworthy terminal evidence.
    OutcomeUnknown,
}

/// Bounded released tool lifecycle metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolActivity {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered public tool name.
    pub tool_name: String,
    /// Released lifecycle state.
    pub state: ToolActivityState,
    /// Bounded summary without raw arguments or quarantined output.
    pub summary: String,
    /// Optional bounded validated input released once execution starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Optional bounded preview of successful output released by post-effect policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// Normalized provider token accounting.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    /// Input tokens, including cached input when provider-reported.
    pub input_tokens: u64,
    /// Output tokens, including reasoning tokens when provider-reported.
    pub output_tokens: u64,
    /// Provider-reported total.
    pub total_tokens: u64,
    /// Cached input tokens when available.
    pub cached_input_tokens: Option<u64>,
    /// Hidden reasoning-token count when available; never reasoning content.
    pub reasoning_tokens: Option<u64>,
}

/// Bounded non-terminal public notice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunNotice {
    /// Stable machine-readable notice reason.
    pub reason: String,
    /// Bounded released message.
    pub message: String,
}

/// Public conversation-message provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleasedMessageRole {
    /// Human or application input.
    User,
    /// Released model output.
    Assistant,
    /// Released explicitly authorized tool output.
    Tool,
    /// A bounded public system notice, never hidden instructions.
    System,
}

/// Public artifact purpose associated with released message content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleasedArtifactPurpose {
    /// Content supplied to an agent run.
    RunInput,
    /// Released output from an agent run.
    RunOutput,
    /// Workflow definition or input.
    Workflow,
    /// Pack or extension archive.
    Extension,
    /// Exported Colossus archive.
    Archive,
}

/// Public release state for an opaque artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleasedArtifactState {
    /// A staged upload is incomplete.
    Uploading,
    /// Bytes remain private pending validation and policy.
    Quarantined,
    /// Bytes are released to authorized callers.
    Available,
    /// Verification or policy denied release.
    Rejected,
    /// A staged upload can no longer be completed.
    Expired,
}

/// Safe metadata for an opaque stored artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedArtifactReference {
    /// Stable opaque artifact identifier.
    pub artifact_id: String,
    /// Bounded display name, never a server-local path.
    pub file_name: String,
    /// Normalized declared or detected media type.
    pub media_type: String,
    /// Verified byte length.
    pub size_bytes: u64,
    /// Lowercase SHA-256 digest.
    pub sha256: String,
    /// Validated intended use.
    pub purpose: ReleasedArtifactPurpose,
    /// Current public release state.
    pub state: ReleasedArtifactState,
    /// UTC RFC3339 creation time.
    pub created_at: String,
}

/// One released public message part.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReleasedContentPart {
    /// Visible released text.
    Text {
        /// Complete part text.
        text: String,
    },
    /// Opaque artifact metadata.
    Artifact {
        /// Authorized artifact reference.
        artifact: ReleasedArtifactReference,
    },
}

/// One newly durable released session message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedSessionMessage {
    /// Owning session identifier.
    pub session_id: String,
    /// Run that produced or consumed the message.
    pub run_id: String,
    /// One-based session message sequence.
    pub sequence: u64,
    /// Message provenance.
    pub role: ReleasedMessageRole,
    /// Ordered released content.
    pub content: Vec<ReleasedContentPart>,
    /// UTC RFC3339 creation time.
    pub created_at: String,
}

/// One prompt or approval class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    /// Ordinary bounded user input.
    Prompt,
    /// Explicit effect approval decision.
    Approval,
}

/// Non-authoritative user-visible approval risk metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRisk {
    /// Bounded reversible impact.
    Low,
    /// Meaningful local or external impact.
    Medium,
    /// Sensitive, destructive, or hard-to-reverse impact.
    High,
}

/// Durable interaction lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStatus {
    /// Awaiting one response from the bound application.
    Pending,
    /// A bound application supplied a valid one-use response.
    Responded,
    /// The interaction expired without a response.
    Expired,
    /// Run cancellation closed the interaction without a response.
    Cancelled,
}

/// One application response to a prompt or approval.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InteractionResponse {
    /// Bounded user input.
    Prompt {
        /// Exact released answer.
        answer: String,
        /// Selected zero-based choice, if applicable.
        selected_index: Option<u32>,
    },
    /// Approval or denial bound to the opaque one-use value shown to the user.
    Approval {
        /// Whether the user approved the exact request.
        approved: bool,
        /// Exact randomized approval binding presented by Colossus.
        request_hash: String,
    },
}

/// Durable prompt or approval visible to an application.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interaction {
    /// Stable interaction identifier.
    pub id: String,
    /// Interaction class.
    pub kind: InteractionKind,
    /// Durable lifecycle status.
    pub status: InteractionStatus,
    /// Authenticated application identity allowed to respond.
    pub application_id: String,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// Bounded user-visible prompt.
    pub prompt: String,
    /// Optional bounded suggested answers.
    pub choices: Vec<String>,
    /// Whether a prompt accepts an answer outside the supplied choices.
    pub allow_free_form: bool,
    /// Randomized one-use approval binding the user must see and echo, when applicable.
    pub request_hash: Option<String>,
    /// Fixed public action category for approvals; never a raw internal action name.
    pub action: Option<String>,
    /// Sanitized display origin or opaque resource class for approvals.
    pub resource: Option<String>,
    /// Non-authoritative released risk metadata for approvals.
    pub risk: Option<ApprovalRisk>,
    /// UTC RFC3339 expiry.
    pub expires_at: String,
    /// One-use response after resolution.
    pub response: Option<InteractionResponse>,
    /// UTC RFC3339 response time after resolution.
    pub responded_at: Option<String>,
}

/// Validate the complete approval display that may cross the public application boundary.
///
/// The action is an exact fixed category. The resource is either that category's fixed
/// opaque label or a canonical HTTP(S) origin with no user information, path, query, or
/// fragment. Keeping this check in the transport-neutral contract lets durable replay
/// and every transport fail closed if an older or malformed record contains private
/// effect detail.
pub fn validate_public_approval_display(action: &str, resource: &str) -> ApiResult<()> {
    let expected_opaque_resource = match action {
        "workspace.modify" => "workspace resource",
        "process.execute" => "configured executable",
        "model.invoke" => "configured model provider",
        "network.access" => "configured network destination",
        "integration.invoke" => "configured integration",
        "colossus.record" => "Colossus record",
        "protected.effect" => "protected resource",
        _ => {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "interaction.action",
                "approval action must use a fixed public category",
            ));
        }
    };
    if resource == expected_opaque_resource || is_canonical_public_origin(resource) {
        return Ok(());
    }
    Err(ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "interaction.resource",
        "approval resource must use its fixed public category or a canonical HTTP(S) origin",
    ))
}

fn is_canonical_public_origin(resource: &str) -> bool {
    if resource.is_empty()
        || resource.len() > MAX_PUBLIC_APPROVAL_ORIGIN_BYTES
        || !resource.is_ascii()
    {
        return false;
    }
    let Ok(url) = Url::parse(resource) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let origin = url.origin().ascii_serialization();
    origin != "null" && origin == resource
}

/// Current public projection of one durable agent run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// Stable run identifier.
    pub id: String,
    /// Durable session identity associated with the run.
    pub session_id: String,
    /// Bounded deterministic display title derived from the opening request.
    pub title: String,
    /// Current lifecycle state.
    pub status: RunStatus,
    /// Requested execution mode.
    pub mode: RunMode,
    /// Resolved logical model role.
    pub role: String,
    /// Reserved skill identities; always empty for public v1alpha1 runs.
    pub skill_ids: Vec<String>,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// UTC RFC3339 last-update time.
    pub updated_at: String,
    /// UTC RFC3339 execution start time.
    pub started_at: Option<String>,
    /// UTC RFC3339 terminal time.
    pub finished_at: Option<String>,
    /// Last durable update sequence.
    pub last_sequence: u64,
    /// Released terminal result, if completed.
    pub result: Option<RunResult>,
    /// Released terminal failure, if failed or uncertain.
    pub failure: Option<RunFailure>,
    /// Durable terminal cancellation evidence, if cancelled.
    pub cancellation: Option<RunCancellation>,
    /// Current unresolved prompt or approval.
    pub pending_interaction: Option<Interaction>,
    /// Opaque optimistic-concurrency token.
    pub etag: String,
    /// Whether the containing thread is hidden from normal listings.
    #[serde(default)]
    pub archived: bool,
}

/// One durable, replayable UI update.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunUpdate {
    /// Stable run identifier.
    pub run_id: String,
    /// Monotonic one-based sequence within the run.
    pub sequence: u64,
    /// UTC RFC3339 durable event time.
    pub occurred_at: String,
    /// Released update content.
    pub kind: RunUpdateKind,
}

/// Released run-feed update content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunUpdateKind {
    /// Durable lifecycle transition.
    State {
        /// New run state.
        status: RunStatus,
    },
    /// Policy-released incremental assistant text.
    OutputDelta {
        /// Complete visible delta.
        text: String,
    },
    /// Provider-declared safe reasoning metadata, never hidden reasoning.
    ReasoningSummary {
        /// Bounded released summary.
        summary: String,
    },
    /// Bounded released tool lifecycle metadata.
    ToolActivity {
        /// Current released activity.
        activity: ToolActivity,
    },
    /// Normalized provider token accounting.
    Usage {
        /// Current accounting item.
        usage: TokenUsage,
    },
    /// Prompt or approval lifecycle update.
    Interaction {
        /// Complete current interaction state.
        interaction: Interaction,
    },
    /// Newly durable released conversation message.
    Message {
        /// Complete released message.
        message: ReleasedSessionMessage,
    },
    /// Bounded non-terminal informational or warning notice.
    Notice {
        /// Released notice.
        notice: RunNotice,
    },
    /// Released successful terminal result.
    Result {
        /// Complete terminal result.
        result: RunResult,
    },
    /// Released known or uncertain terminal failure.
    Failure {
        /// Terminal run status.
        status: RunStatus,
        /// Safe failure detail.
        failure: RunFailure,
    },
    /// Released cooperative cancellation evidence.
    Cancellation {
        /// Durable cancellation detail.
        cancellation: RunCancellation,
    },
}

/// Forward-compatible public run input part.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentPart {
    /// Bounded visible text. This is the only accepted v1alpha1 input kind.
    Text {
        /// Visible text.
        text: String,
    },
    /// Opaque caller-owned released artifact supplied as run input.
    Artifact {
        /// Exact authorized artifact identifier.
        artifact_id: String,
    },
}

/// Canonical session prefix used to start a separate child conversation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBranch {
    /// Caller-owned run whose session supplies the canonical context.
    pub source_run_id: String,
    /// Exact number of canonical messages to copy, or zero when the context mode
    /// resolves the boundary through the source run.
    pub source_message_count: u64,
    /// Provider-transcript or visible-conversation projection for the copied prefix.
    #[serde(default)]
    pub context_mode: RunBranchContextMode,
}

/// Request to create and start one agent run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRunRequest {
    /// Ordered initial visible content.
    pub input: Vec<ContentPart>,
    /// Existing session, or absent to create a session.
    pub session_id: Option<String>,
    /// Optional caller-asserted end-user correlation identifier, never authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_user_id: Option<String>,
    /// Logical role; absent selects the configured default role.
    pub role: Option<String>,
    /// Requested execution mode.
    #[serde(default)]
    pub mode: RunMode,
    /// Research breadth; present only for Research runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_depth: Option<ResearchDepth>,
    /// Explicit evidence lanes; non-empty only for Research runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub research_sources: Vec<ResearchSourceKind>,
    /// Reserved for a future public skill ceiling; v1alpha1 requires this to be empty.
    #[serde(default)]
    pub skill_ids: Vec<String>,
    /// Exact caller-owned Plan continuation, when this run revises or executes a Plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_action: Option<PlanRunAction>,
    /// Optional point-in-time canonical context for a separate child session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<RunBranch>,
    /// Model-turn ceiling; zero selects the configured default.
    pub max_turns: u32,
    /// Required key for atomic create replay.
    pub idempotency_key: IdempotencyKey,
}

/// Encrypted durable coordinator input captured when a run is accepted.
///
/// This record is trusted runtime input, not part of the public [`Run`] projection.
/// It includes the original caller ceiling so recovery cannot silently widen authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunExecutionRequest {
    /// Validated public create request, including its encrypted text and idempotency key.
    pub request: CreateRunRequest,
    /// Stable authenticated application owner.
    pub application_id: String,
    /// Original hosting class, retained so recovery cannot change trust placement.
    pub application_kind: crate::ApplicationKind,
    /// Exact API scopes present when the run was accepted.
    pub scopes: Vec<crate::ApiScope>,
    /// Exact allowed role ceiling present when the run was accepted.
    pub allowed_roles: Vec<String>,
    /// Exact allowed tool ceiling present when the run was accepted.
    ///
    /// Empty means deny all.
    pub allowed_tools: Vec<String>,
    /// Normalized W3C parent context captured at acceptance, excluding baggage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<colossus_contracts::RemoteTraceContext>,
}

impl RunExecutionRequest {
    pub(super) fn capture(caller: &CallerContext, request: &CreateRunRequest) -> Self {
        Self {
            request: request.clone(),
            application_id: caller.principal().application_id().into(),
            application_kind: caller.principal().kind(),
            scopes: caller.principal().scopes().cloned().collect(),
            allowed_roles: caller
                .principal()
                .allowed_roles()
                .map(str::to_owned)
                .collect(),
            allowed_tools: caller
                .principal()
                .allowed_tools()
                .map(str::to_owned)
                .collect(),
            trace_context: caller.remote_trace_context().cloned(),
        }
    }
}

impl CreateRunRequest {
    pub(crate) fn display_title(&self) -> String {
        let mut title = String::new();
        let mut title_characters = 0;
        let mut needs_space = false;
        let mut truncated = false;

        'parts: for part in &self.input {
            let ContentPart::Text { text } = part else {
                continue;
            };
            for character in text.chars() {
                let is_unsafe_formatting = matches!(
                    character,
                    '\u{061c}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                );
                if character.is_whitespace() || character.is_control() || is_unsafe_formatting {
                    needs_space = !title.is_empty();
                    continue;
                }
                let additional_characters = usize::from(needs_space) + 1;
                if title_characters + additional_characters > MAX_RUN_TITLE_CHARACTERS {
                    truncated = true;
                    break 'parts;
                }
                if needs_space {
                    title.push(' ');
                    needs_space = false;
                }
                title.push(character);
                title_characters += additional_characters;
            }
        }

        if truncated && title_characters == MAX_RUN_TITLE_CHARACTERS {
            title.pop();
        }
        let title = title.trim();
        if title.is_empty() {
            return UNTITLED_RUN.into();
        }
        if truncated {
            format!("{}…", title.trim_end())
        } else {
            title.into()
        }
    }

    /// Validate bounded public request fields.
    pub fn validate(&self) -> ApiResult<()> {
        if self.input.is_empty() {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "input",
                "input must contain at least one content part",
            ));
        }
        if self.input.len() > MAX_INPUT_PARTS {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "input",
                format!("input must contain at most {MAX_INPUT_PARTS} content parts"),
            ));
        }
        let mut total_input_bytes = 0_usize;
        for part in &self.input {
            match part {
                ContentPart::Text { text } => {
                    bounded_text(text, "input.text", MAX_INPUT_BYTES, false)?;
                    total_input_bytes = total_input_bytes.saturating_add(text.len());
                }
                ContentPart::Artifact { artifact_id } => {
                    crate::artifacts::validate_artifact_id(artifact_id, "input.artifact_id")?;
                }
            }
        }
        if total_input_bytes > MAX_INPUT_BYTES {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "input",
                format!("combined input must be at most {MAX_INPUT_BYTES} bytes"),
            ));
        }
        if let Some(session_id) = &self.session_id {
            token(session_id, "session_id", MAX_IDENTIFIER_BYTES)?;
        }
        if let Some(end_user_id) = &self.end_user_id {
            token(end_user_id, "end_user_id", MAX_IDENTIFIER_BYTES)?;
        }
        if let Some(role) = &self.role {
            token(role, "role", MAX_ROLE_BYTES)?;
        }
        if !self.skill_ids.is_empty() {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "skill_ids",
                "public application runs do not support skill activation",
            ));
        }
        match self.mode {
            RunMode::Research => {
                if self.research_depth.is_none() {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "research_depth",
                        "Research mode requires an explicit depth",
                    ));
                }
                if self.research_sources.is_empty() {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "research_sources",
                        "Research mode requires at least one evidence lane",
                    ));
                }
                let unique = self
                    .research_sources
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>();
                if unique.len() != self.research_sources.len() {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "research_sources",
                        "Research evidence lanes must be unique",
                    ));
                }
                if self.plan_action.is_some() || self.branch.is_some() {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "mode",
                        "Research mode does not support Plan continuation or Aside branching",
                    ));
                }
            }
            RunMode::Execute | RunMode::Plan => {
                if self.research_depth.is_some() || !self.research_sources.is_empty() {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "research_depth",
                        "Research options are accepted only in Research mode",
                    ));
                }
            }
        }
        if let Some(action) = &self.plan_action {
            token(
                action.source_run_id(),
                "plan_action.source_run_id",
                MAX_IDENTIFIER_BYTES,
            )?;
            if action.expected_revision() == 0 {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "plan_action.expected_revision",
                    "expected Plan revision must be greater than zero",
                ));
            }
            if self.session_id.is_none() {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "session_id",
                    "Plan continuation requires the source run session",
                ));
            }
            match action {
                PlanRunAction::Revise { .. } if self.mode != RunMode::Plan => {
                    return Err(ApiError::invalid(
                        ApiErrorReason::InvalidArgument,
                        "mode",
                        "Plan revision requires Plan Mode",
                    ));
                }
                PlanRunAction::Execute { strategy, .. } => {
                    if self.mode != RunMode::Execute {
                        return Err(ApiError::invalid(
                            ApiErrorReason::InvalidArgument,
                            "mode",
                            "Plan execution requires Execute Mode",
                        ));
                    }
                    if let PlanExecutionStrategy::Goal { max_iterations } = strategy
                        && !(1..=50).contains(max_iterations)
                    {
                        return Err(ApiError::invalid(
                            ApiErrorReason::InvalidArgument,
                            "plan_action.strategy.max_iterations",
                            "Goal iterations must be in 1..=50",
                        ));
                    }
                }
                PlanRunAction::Revise { .. } => {}
            }
        }
        if let Some(branch) = &self.branch {
            token(
                &branch.source_run_id,
                "branch.source_run_id",
                MAX_IDENTIFIER_BYTES,
            )?;
            if branch.source_message_count > 512 {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "branch.source_message_count",
                    "branch context may contain at most 512 canonical messages",
                ));
            }
            if branch.context_mode == RunBranchContextMode::SourceRunConversation
                && branch.source_message_count != 0
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "branch.source_message_count",
                    "source-run conversation boundaries are resolved canonically and require a zero message count",
                ));
            }
            if self.session_id.is_some() || self.plan_action.is_some() {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "branch",
                    "branch creation cannot continue an existing session or Plan",
                ));
            }
        }
        if self.max_turns > 100 {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "max_turns",
                "max_turns must be at most 100",
            ));
        }
        Ok(())
    }
}

/// Validated server-created run identity paired with a create request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewRun {
    id: String,
    session_id: String,
    role: String,
}

impl NewRun {
    /// Bind server-created identifiers and the resolved role to a validated request.
    pub fn from_request(
        id: impl Into<String>,
        session_id: impl Into<String>,
        resolved_role: impl Into<String>,
        request: &CreateRunRequest,
    ) -> ApiResult<Self> {
        request.validate()?;
        let id = id.into();
        let session_id = session_id.into();
        let role = resolved_role.into();
        token(&id, "run_id", MAX_IDENTIFIER_BYTES)?;
        token(&session_id, "session_id", MAX_IDENTIFIER_BYTES)?;
        token(&role, "role", MAX_ROLE_BYTES)?;
        if request
            .session_id
            .as_ref()
            .is_some_and(|requested| requested != &session_id)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "session_id",
                "resolved session does not match the requested session",
            ));
        }
        if request
            .role
            .as_ref()
            .is_some_and(|requested| requested != &role)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "role",
                "resolved role does not match the requested role",
            ));
        }
        Ok(Self {
            id,
            session_id,
            role,
        })
    }

    /// Server-created run identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Resolved durable session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Resolved logical role.
    pub fn role(&self) -> &str {
        &self.role
    }
}

/// Value returned from an idempotent operation.
#[derive(Clone, Debug, PartialEq)]
pub struct Idempotent<T> {
    /// Durable value created by the first request.
    pub value: T,
    /// Whether this response replayed that first value.
    pub replayed: bool,
}

/// Create-run response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRunResponse {
    /// Durable run accepted before execution begins.
    pub run: Run,
}

/// Get-run request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRunRequest {
    /// Stable run identifier.
    pub run_id: String,
}

/// Stable filtered run listing request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRunsRequest {
    /// Optional session filter.
    pub session_id: Option<String>,
    /// Optional lifecycle filters; empty includes every status.
    #[serde(default)]
    pub statuses: Vec<RunStatus>,
    /// Bounded page size.
    pub page_size: u32,
    /// Opaque continuation token.
    pub page_token: Option<String>,
    /// Include runs whose containing thread is archived.
    #[serde(default)]
    pub include_archived: bool,
}

impl ListRunsRequest {
    /// Return the default or clamped nonzero page size.
    pub fn bounded_page_size(&self) -> usize {
        if self.page_size == 0 {
            return MAX_PAGE_SIZE;
        }
        usize::try_from(self.page_size)
            .unwrap_or(MAX_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE)
    }
}

/// One stable run page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRunsResponse {
    /// Runs in deterministic newest-first order.
    pub runs: Vec<Run>,
    /// Opaque token for the next page.
    pub next_page_token: Option<String>,
}

/// Timeline lane used by a curated session activity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionActivityLane {
    /// User, assistant, and model-turn activity.
    Agent,
    /// Tool and effect lifecycle activity.
    Tools,
    /// Runtime, context, policy, and usage activity.
    System,
}

/// Human-readable activity kind used by the event table.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionActivityKind {
    /// Released human or application input.
    User,
    /// Released model activity.
    Assistant,
    /// Policy-released tool activity.
    Tool,
    /// Trusted runtime activity.
    System,
}

/// Released lifecycle state for a curated activity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionActivityStatus {
    /// Accepted but not started.
    Requested,
    /// Actively progressing.
    Running,
    /// Waiting on user input or approval.
    Waiting,
    /// Reached a known successful terminal state.
    Completed,
    /// Reached a known failed terminal state.
    Failed,
    /// Settled without completing.
    Cancelled,
    /// An external effect may have occurred.
    OutcomeUnknown,
}

/// One bounded released inspector value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionActivityContent {
    /// Rendering hint: `text` or `json`.
    pub format: String,
    /// Bounded policy-released content.
    pub value: String,
}

/// One curated logical activity for a caller-owned session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionActivity {
    /// Stable logical identifier used for live merging.
    pub activity_id: String,
    /// Owning public run when available.
    pub run_id: Option<String>,
    /// One-based model turn when available.
    pub turn: Option<u32>,
    /// Timeline lane.
    pub lane: SessionActivityLane,
    /// Display kind.
    pub kind: SessionActivityKind,
    /// Bounded title.
    pub title: String,
    /// Bounded released summary.
    pub summary: String,
    /// Coarse actor label without an internal identity.
    pub actor: String,
    /// Released lifecycle state when applicable.
    pub status: Option<SessionActivityStatus>,
    /// UTC start or occurrence time.
    pub started_at: String,
    /// UTC completion time when trustworthy.
    pub completed_at: Option<String>,
    /// Millisecond duration only for paired canonical boundaries.
    pub duration_ms: Option<u64>,
    /// Policy-released input.
    pub input: Option<SessionActivityContent>,
    /// Policy-released result.
    pub result: Option<SessionActivityContent>,
    /// Small allowlisted metadata values.
    pub attributes: BTreeMap<String, String>,
    /// Canonical event types contributing to the logical record.
    pub source_event_types: Vec<String>,
    /// First contributing global sequence.
    pub first_sequence: u64,
    /// Latest contributing global sequence.
    pub last_sequence: u64,
}

/// Caller-scoped session activity query addressed through an owned run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSessionActivityRequest {
    /// Any caller-owned run in the requested session.
    pub source_run_id: String,
    /// Case-insensitive search over released activity fields.
    pub query: String,
    /// Optional lane filters; empty includes every lane.
    #[serde(default)]
    pub lanes: Vec<SessionActivityLane>,
    /// Optional kind filters; empty includes every kind.
    #[serde(default)]
    pub kinds: Vec<SessionActivityKind>,
    /// Optional status filters; empty includes every status.
    #[serde(default)]
    pub statuses: Vec<SessionActivityStatus>,
    /// Bounded page size; zero selects the default.
    pub page_size: u32,
    /// Opaque continuation token bound to this exact query.
    pub page_token: Option<String>,
}

impl ListSessionActivityRequest {
    /// Validate bounded public request fields.
    pub fn validate(&self) -> ApiResult<()> {
        token(&self.source_run_id, "source_run_id", MAX_IDENTIFIER_BYTES)?;
        bounded_text(&self.query, "query", 256, true)?;
        if self
            .page_token
            .as_ref()
            .is_some_and(|value| value.len() > 4_096)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "page_token",
                "page_token must be at most 4096 bytes",
            ));
        }
        Ok(())
    }

    /// Return the default or clamped page size.
    pub fn bounded_page_size(&self) -> usize {
        if self.page_size == 0 {
            100
        } else {
            usize::try_from(self.page_size).unwrap_or(100).clamp(1, 100)
        }
    }
}

/// One eventually consistent newest-first activity page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSessionActivityResponse {
    /// Curated activities matching the request.
    pub activities: Vec<SessionActivity>,
    /// Opaque token for the next matching page.
    pub next_page_token: Option<String>,
    /// Current authoritative journal head when the page was read.
    pub head_sequence: u64,
    /// Latest global sequence applied to the activity projection.
    pub projected_through_sequence: u64,
    /// Whether the disposable projection reached the observed journal head.
    pub caught_up: bool,
}

/// Replay-and-tail request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchRunRequest {
    /// Stable run identifier.
    pub run_id: String,
    /// Last sequence already delivered; replay begins exclusively after it.
    pub after_sequence: u64,
}

/// Cooperative cancellation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelRunRequest {
    /// Stable run identifier.
    pub run_id: String,
    /// Required key making cancellation retries safe.
    pub idempotency_key: IdempotencyKey,
}

/// Archive one terminal thread, addressed through a caller-owned run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveThreadRequest {
    /// Any stable run identifier belonging to the thread.
    pub run_id: String,
    /// Required key making archive retries safe.
    pub idempotency_key: IdempotencyKey,
}

/// Restore one archived thread, addressed through a caller-owned run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreThreadRequest {
    /// Any stable run identifier belonging to the thread.
    pub run_id: String,
    /// Required key making restore retries safe.
    pub idempotency_key: IdempotencyKey,
}

/// Current archive lifecycle for one durable thread.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadLifecycle {
    /// Durable session identity shared by the thread's runs.
    pub session_id: String,
    /// Whether the thread is hidden from normal listings.
    pub archived: bool,
}

/// One-use interaction response request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondInteractionRequest {
    /// Stable run identifier.
    pub run_id: String,
    /// Stable interaction identifier.
    pub interaction_id: String,
    /// Opaque pending-interaction concurrency token.
    pub etag: String,
    /// Required key making response retries safe.
    pub idempotency_key: IdempotencyKey,
    /// Prompt answer or exact-binding approval response.
    pub response: InteractionResponse,
}

/// Transport-neutral stream of replayed and live run updates.
pub type RunUpdateStream = Pin<Box<dyn Stream<Item = ApiResult<RunUpdate>> + Send + 'static>>;

/// Runtime-owned execution hook used by the public application facade.
///
/// Implementations must preserve the supplied authenticated caller through policy,
/// approval, permit, and journal evidence. Scheduling success means only that the
/// durable queued run was accepted; it does not imply an effectful outcome.
#[async_trait]
pub trait RunExecutor: Send + Sync {
    /// Schedule execution for an already durable queued run.
    async fn start(
        &self,
        caller: CallerContext,
        run: Run,
        request: CreateRunRequest,
    ) -> ApiResult<()>;

    /// Request cooperative cancellation at the next safe boundary.
    async fn cancel(&self, caller: &CallerContext, run: &Run) -> ApiResult<()>;

    /// Deliver one already validated and durably consumed interaction response.
    async fn deliver_interaction_response(
        &self,
        caller: &CallerContext,
        run_id: &str,
        interaction: &Interaction,
    ) -> ApiResult<()>;
}

/// Public run application service implemented by embedded and remote backends.
#[async_trait]
pub trait AgentRunApi: Send + Sync {
    /// Atomically accept one idempotent run before execution begins.
    async fn create_run(
        &self,
        caller: &CallerContext,
        request: CreateRunRequest,
    ) -> ApiResult<CreateRunResponse>;

    /// Return one current run projection.
    async fn get_run(&self, caller: &CallerContext, request: GetRunRequest) -> ApiResult<Run>;

    /// Return one bounded stable run page.
    async fn list_runs(
        &self,
        caller: &CallerContext,
        request: ListRunsRequest,
    ) -> ApiResult<ListRunsResponse>;

    /// Return one caller-scoped page of policy-released canonical session activity.
    async fn list_session_activity(
        &self,
        _caller: &CallerContext,
        _request: ListSessionActivityRequest,
    ) -> ApiResult<ListSessionActivityResponse> {
        Err(ApiError::failed_precondition(
            ApiErrorReason::InvalidRunTransition,
            "the connected backend does not support session activity",
        ))
    }

    /// Replay updates after an exclusive cursor and then tail live updates.
    async fn watch_run(
        &self,
        caller: &CallerContext,
        request: WatchRunRequest,
    ) -> ApiResult<RunUpdateStream>;

    /// Request idempotent cooperative cancellation.
    async fn cancel_run(&self, caller: &CallerContext, request: CancelRunRequest)
    -> ApiResult<Run>;

    /// Hide a thread after every run in it has reached a terminal state.
    async fn archive_thread(
        &self,
        _caller: &CallerContext,
        _request: ArchiveThreadRequest,
    ) -> ApiResult<ThreadLifecycle> {
        Err(ApiError::failed_precondition(
            ApiErrorReason::InvalidRunTransition,
            "the connected backend does not support thread archiving",
        ))
    }

    /// Return an archived thread to normal listings.
    async fn restore_thread(
        &self,
        _caller: &CallerContext,
        _request: RestoreThreadRequest,
    ) -> ApiResult<ThreadLifecycle> {
        Err(ApiError::failed_precondition(
            ApiErrorReason::InvalidRunTransition,
            "the connected backend does not support thread restoration",
        ))
    }

    /// Resolve one caller-bound prompt or approval exactly once.
    async fn respond_interaction(
        &self,
        caller: &CallerContext,
        request: RespondInteractionRequest,
    ) -> ApiResult<Interaction>;
}
