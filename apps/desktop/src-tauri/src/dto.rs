use colossus_sdk::{
    ApiError, ApiErrorCode, ApprovalInteraction, ApprovalRisk, ArtifactPurpose, ArtifactReference,
    ArtifactState, CancelRunRequest, CreateRunRequest, FieldViolation, GetRunRequest,
    IdempotencyKey, InputContentPart, Interaction, InteractionAnswer, InteractionContent,
    InteractionKind, InteractionStatus, ListRunsRequest, MessageContentPart, MessageRole,
    OutcomeCertainty, PageRequest, PromptAnswer, PromptChoice, RespondInteractionRequest, Run,
    RunCancellation, RunFailure, RunMode, RunResult, RunStatus, RunTerminal, RunUpdate,
    RunUpdateKind, SdkError, SessionMessage, TokenUsage, ToolActivity, ToolActivityState,
    WatchRunRequest,
};
use serde::{Deserialize, Serialize};

const MAX_TEXT_BYTES: usize = 65_536;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_OPAQUE_BYTES: usize = 512;
const PAGE_SIZE: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionStateDto {
    Connected,
    Disconnected,
    NotConfigured,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionStatusDto {
    pub(crate) state: ConnectionStateDto,
    pub(crate) message: String,
}

impl ConnectionStatusDto {
    pub(crate) fn connected() -> Self {
        Self {
            state: ConnectionStateDto::Connected,
            message: "Connected to Colossus.".into(),
        }
    }

    pub(crate) fn disconnected() -> Self {
        Self {
            state: ConnectionStateDto::Disconnected,
            message: "Colossus is not connected.".into(),
        }
    }

    pub(crate) fn not_configured() -> Self {
        Self {
            state: ConnectionStateDto::NotConfigured,
            message: "Desktop enrollment is not configured.".into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldViolationDto {
    pub(crate) field: String,
    pub(crate) description: String,
}

impl From<FieldViolation> for FieldViolationDto {
    fn from(value: FieldViolation) -> Self {
        Self {
            field: value.field,
            description: value.description,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandErrorDto {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
    pub(crate) outcome_unknown: bool,
    pub(crate) violations: Vec<FieldViolationDto>,
}

impl CommandErrorDto {
    pub(crate) fn not_configured() -> Self {
        Self::local(
            "not_configured",
            "Desktop enrollment is not configured.",
            false,
            false,
        )
    }

    pub(crate) fn disconnected() -> Self {
        Self::local(
            "disconnected",
            "Connect to Colossus before using this action.",
            true,
            false,
        )
    }

    pub(crate) fn stream_delivery() -> Self {
        Self::local(
            "stream_delivery_failed",
            "The desktop update channel closed.",
            false,
            false,
        )
    }

    pub(crate) fn busy(message: &str) -> Self {
        Self::local("busy", message, true, false)
    }

    pub(crate) fn invalid(field: &str, description: &str) -> Self {
        Self {
            code: "invalid_argument".into(),
            message: "The request is invalid.".into(),
            retryable: false,
            outcome_unknown: false,
            violations: vec![FieldViolationDto {
                field: field.into(),
                description: description.into(),
            }],
        }
    }

    fn local(code: &str, message: &str, retryable: bool, outcome_unknown: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            outcome_unknown,
            violations: Vec::new(),
        }
    }

    pub(crate) fn from_api(error: ApiError) -> Self {
        Self {
            code: api_error_code(error.code).into(),
            message: error.message,
            retryable: error.retryable,
            outcome_unknown: error.code == ApiErrorCode::OutcomeUnknown,
            violations: error.violations.into_iter().map(Into::into).collect(),
        }
    }

    pub(crate) fn from_sdk(error: SdkError) -> Self {
        match error {
            SdkError::InvalidConfiguration(_) | SdkError::PathNotAbsolute(_) => {
                Self::not_configured()
            }
            SdkError::Unavailable | SdkError::Busy => Self::local(
                "unavailable",
                "The Colossus daemon is unavailable.",
                true,
                false,
            ),
            SdkError::Authentication => Self::local(
                "unauthenticated",
                "Colossus desktop authentication failed.",
                false,
                false,
            ),
            SdkError::IdentityMismatch => Self::local(
                "identity_mismatch",
                "The Colossus daemon identity could not be verified.",
                false,
                false,
            ),
            SdkError::VersionMismatch => Self::local(
                "version_mismatch",
                "This desktop build is incompatible with the Colossus API.",
                false,
                false,
            ),
            SdkError::OutcomeUnknown => Self::local(
                "outcome_unknown",
                "The operation outcome is unknown. Do not retry automatically.",
                false,
                true,
            ),
            SdkError::Api(error) => Self::from_api(error),
            SdkError::Transport => Self::local(
                "transport",
                "The secure Colossus connection failed.",
                true,
                false,
            ),
            SdkError::CloseFailed => Self::local(
                "close_failed",
                "The previous Colossus connection did not close cleanly.",
                false,
                false,
            ),
            SdkError::LaunchFailed | SdkError::SidecarFailed | SdkError::EmbeddedOpenFailed => {
                Self::local(
                    "unavailable",
                    "The Colossus runtime is unavailable.",
                    false,
                    false,
                )
            }
            _ => Self::local(
                "internal",
                "The native Colossus bridge failed safely.",
                false,
                false,
            ),
        }
    }
}

fn api_error_code(code: ApiErrorCode) -> &'static str {
    match code {
        ApiErrorCode::InvalidArgument => "invalid_argument",
        ApiErrorCode::Unauthenticated => "unauthenticated",
        ApiErrorCode::PermissionDenied => "permission_denied",
        ApiErrorCode::NotFound => "not_found",
        ApiErrorCode::AlreadyExists => "already_exists",
        ApiErrorCode::Conflict => "conflict",
        ApiErrorCode::FailedPrecondition => "failed_precondition",
        ApiErrorCode::ResourceExhausted => "resource_exhausted",
        ApiErrorCode::Cancelled => "cancelled",
        ApiErrorCode::Unavailable => "unavailable",
        ApiErrorCode::Internal => "internal",
        ApiErrorCode::OutcomeUnknown => "outcome_unknown",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunModeDto {
    Execute,
    Plan,
}

impl From<RunMode> for RunModeDto {
    fn from(value: RunMode) -> Self {
        match value {
            RunMode::Execute => Self::Execute,
            RunMode::Plan => Self::Plan,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunStatusDto {
    Queued,
    Running,
    Waiting,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    OutcomeUnknown,
}

impl From<RunStatus> for RunStatusDto {
    fn from(value: RunStatus) -> Self {
        match value {
            RunStatus::Queued => Self::Queued,
            RunStatus::Running => Self::Running,
            RunStatus::Waiting => Self::Waiting,
            RunStatus::Cancelling => Self::Cancelling,
            RunStatus::Completed => Self::Completed,
            RunStatus::Failed => Self::Failed,
            RunStatus::Cancelled => Self::Cancelled,
            RunStatus::Interrupted => Self::Interrupted,
            RunStatus::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunResultDto {
    pub(crate) output: String,
    pub(crate) profile: String,
    pub(crate) model: String,
    pub(crate) elapsed_seconds: f64,
}

impl From<RunResult> for RunResultDto {
    fn from(value: RunResult) -> Self {
        Self {
            output: value.output,
            profile: value.profile,
            model: value.model,
            elapsed_seconds: value.elapsed_seconds,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunFailureDto {
    pub(crate) reason: String,
    pub(crate) message: String,
    pub(crate) outcome_certainty: OutcomeCertaintyDto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutcomeCertaintyDto {
    Known,
    Unknown,
}

impl From<OutcomeCertainty> for OutcomeCertaintyDto {
    fn from(value: OutcomeCertainty) -> Self {
        match value {
            OutcomeCertainty::Known => Self::Known,
            OutcomeCertainty::Unknown => Self::Unknown,
        }
    }
}

impl From<RunFailure> for RunFailureDto {
    fn from(value: RunFailure) -> Self {
        Self {
            reason: value.reason,
            message: value.message,
            outcome_certainty: value.outcome_certainty.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunCancellationDto {
    pub(crate) turn: u32,
    pub(crate) message: String,
}

impl From<RunCancellation> for RunCancellationDto {
    fn from(value: RunCancellation) -> Self {
        Self {
            turn: value.turn,
            message: value.message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RunTerminalDto {
    Result { result: RunResultDto },
    Failure { failure: RunFailureDto },
    Cancellation { cancellation: RunCancellationDto },
}

impl From<RunTerminal> for RunTerminalDto {
    fn from(value: RunTerminal) -> Self {
        match value {
            RunTerminal::Result(result) => Self::Result {
                result: result.into(),
            },
            RunTerminal::Failure(failure) => Self::Failure {
                failure: failure.into(),
            },
            RunTerminal::Cancellation(cancellation) => Self::Cancellation {
                cancellation: cancellation.into(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunDto {
    pub(crate) run_id: String,
    pub(crate) session_id: String,
    pub(crate) role: String,
    pub(crate) mode: RunModeDto,
    pub(crate) status: RunStatusDto,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) started_at: Option<String>,
    pub(crate) finished_at: Option<String>,
    pub(crate) last_sequence: u64,
    pub(crate) pending_interaction_count: u32,
    pub(crate) terminal: Option<RunTerminalDto>,
    pub(crate) etag: String,
    pub(crate) selected_skills: Vec<String>,
}

impl From<Run> for RunDto {
    fn from(value: Run) -> Self {
        Self {
            run_id: value.run_id,
            session_id: value.session_id,
            role: value.role,
            mode: value.mode.into(),
            status: value.status.into(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
            last_sequence: value.last_sequence,
            pending_interaction_count: value.pending_interaction_count,
            terminal: value.terminal.map(Into::into),
            etag: value.etag,
            selected_skills: value.selected_skills,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InteractionKindDto {
    UserPrompt,
    Approval,
}

impl From<InteractionKind> for InteractionKindDto {
    fn from(value: InteractionKind) -> Self {
        match value {
            InteractionKind::UserPrompt => Self::UserPrompt,
            InteractionKind::Approval => Self::Approval,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InteractionStatusDto {
    Pending,
    Answered,
    Expired,
    Cancelled,
}

impl From<InteractionStatus> for InteractionStatusDto {
    fn from(value: InteractionStatus) -> Self {
        match value {
            InteractionStatus::Pending => Self::Pending,
            InteractionStatus::Answered => Self::Answered,
            InteractionStatus::Expired => Self::Expired,
            InteractionStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptChoiceDto {
    pub(crate) choice_id: String,
    pub(crate) label: String,
}

impl From<PromptChoice> for PromptChoiceDto {
    fn from(value: PromptChoice) -> Self {
        Self {
            choice_id: value.choice_id,
            label: value.label,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalRiskDto {
    Low,
    Medium,
    High,
}

impl From<ApprovalRisk> for ApprovalRiskDto {
    fn from(value: ApprovalRisk) -> Self {
        match value {
            ApprovalRisk::Low => Self::Low,
            ApprovalRisk::Medium => Self::Medium,
            ApprovalRisk::High => Self::High,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum InteractionContentDto {
    UserPrompt {
        question: String,
        choices: Vec<PromptChoiceDto>,
        #[serde(rename = "allowFreeForm")]
        allow_free_form: bool,
    },
    Approval {
        reason: String,
        action: String,
        resource: String,
        risk: Option<ApprovalRiskDto>,
        #[serde(rename = "requestHash")]
        request_hash: String,
    },
}

impl From<InteractionContent> for InteractionContentDto {
    fn from(value: InteractionContent) -> Self {
        match value {
            InteractionContent::UserPrompt(prompt) => Self::UserPrompt {
                question: prompt.question,
                choices: prompt.choices.into_iter().map(Into::into).collect(),
                allow_free_form: prompt.allow_free_form,
            },
            InteractionContent::Approval(approval) => Self::from_approval(approval),
        }
    }
}

impl InteractionContentDto {
    fn from_approval(approval: ApprovalInteraction) -> Self {
        Self::Approval {
            reason: approval.reason,
            action: approval.action,
            resource: approval.resource,
            risk: approval.risk.map(Into::into),
            request_hash: approval.request_hash,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InteractionDto {
    pub(crate) interaction_id: String,
    pub(crate) run_id: String,
    pub(crate) kind: InteractionKindDto,
    pub(crate) status: InteractionStatusDto,
    pub(crate) created_at: String,
    pub(crate) expires_at: String,
    pub(crate) respondable_by_caller: bool,
    pub(crate) etag: String,
    pub(crate) content: InteractionContentDto,
}

impl From<Interaction> for InteractionDto {
    fn from(value: Interaction) -> Self {
        Self {
            interaction_id: value.interaction_id,
            run_id: value.run_id,
            kind: value.kind.into(),
            status: value.status.into(),
            created_at: value.created_at,
            expires_at: value.expires_at,
            respondable_by_caller: value.respondable_by_caller,
            etag: value.etag,
            content: value.content.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolActivityStateDto {
    Requested,
    WaitingApproval,
    Started,
    Completed,
    Failed,
    OutcomeUnknown,
}

impl From<ToolActivityState> for ToolActivityStateDto {
    fn from(value: ToolActivityState) -> Self {
        match value {
            ToolActivityState::Requested => Self::Requested,
            ToolActivityState::WaitingApproval => Self::WaitingApproval,
            ToolActivityState::Started => Self::Started,
            ToolActivityState::Completed => Self::Completed,
            ToolActivityState::Failed => Self::Failed,
            ToolActivityState::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolActivityDto {
    pub(crate) call_id: String,
    pub(crate) tool_name: String,
    pub(crate) state: ToolActivityStateDto,
    pub(crate) summary: String,
}

impl From<ToolActivity> for ToolActivityDto {
    fn from(value: ToolActivity) -> Self {
        Self {
            call_id: value.call_id,
            tool_name: value.tool_name,
            state: value.state.into(),
            summary: value.summary,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
pub(crate) struct TokenUsageDto {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: Option<u64>,
    pub(crate) reasoning_tokens: Option<u64>,
}

impl From<TokenUsage> for TokenUsageDto {
    fn from(value: TokenUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            total_tokens: value.total_tokens,
            cached_input_tokens: value.cached_input_tokens,
            reasoning_tokens: value.reasoning_tokens,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MessageRoleDto {
    User,
    Assistant,
    Tool,
    System,
}

impl From<MessageRole> for MessageRoleDto {
    fn from(value: MessageRole) -> Self {
        match value {
            MessageRole::User => Self::User,
            MessageRole::Assistant => Self::Assistant,
            MessageRole::Tool => Self::Tool,
            MessageRole::System => Self::System,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactPurposeDto {
    RunInput,
    RunOutput,
    Workflow,
    Extension,
    Archive,
}

impl From<ArtifactPurpose> for ArtifactPurposeDto {
    fn from(value: ArtifactPurpose) -> Self {
        match value {
            ArtifactPurpose::RunInput => Self::RunInput,
            ArtifactPurpose::RunOutput => Self::RunOutput,
            ArtifactPurpose::Workflow => Self::Workflow,
            ArtifactPurpose::Extension => Self::Extension,
            ArtifactPurpose::Archive => Self::Archive,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactStateDto {
    Uploading,
    Quarantined,
    Available,
    Rejected,
    Expired,
}

impl From<ArtifactState> for ArtifactStateDto {
    fn from(value: ArtifactState) -> Self {
        match value {
            ArtifactState::Uploading => Self::Uploading,
            ArtifactState::Quarantined => Self::Quarantined,
            ArtifactState::Available => Self::Available,
            ArtifactState::Rejected => Self::Rejected,
            ArtifactState::Expired => Self::Expired,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactReferenceDto {
    pub(crate) artifact_id: String,
    pub(crate) file_name: String,
    pub(crate) media_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) purpose: ArtifactPurposeDto,
    pub(crate) state: ArtifactStateDto,
    pub(crate) created_at: String,
}

impl From<ArtifactReference> for ArtifactReferenceDto {
    fn from(value: ArtifactReference) -> Self {
        Self {
            artifact_id: value.artifact_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
            purpose: value.purpose.into(),
            state: value.state.into(),
            created_at: value.created_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum MessageContentPartDto {
    Text { text: String },
    Artifact { artifact: ArtifactReferenceDto },
}

impl From<MessageContentPart> for MessageContentPartDto {
    fn from(value: MessageContentPart) -> Self {
        match value {
            MessageContentPart::Text(text) => Self::Text { text },
            MessageContentPart::Artifact(artifact) => Self::Artifact {
                artifact: artifact.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionMessageDto {
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) sequence: u64,
    pub(crate) role: MessageRoleDto,
    pub(crate) content: Vec<MessageContentPartDto>,
    pub(crate) created_at: String,
}

impl From<SessionMessage> for SessionMessageDto {
    fn from(value: SessionMessage) -> Self {
        Self {
            session_id: value.session_id,
            run_id: value.run_id,
            sequence: value.sequence,
            role: value.role.into(),
            content: value.content.into_iter().map(Into::into).collect(),
            created_at: value.created_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RunUpdateKindDto {
    State {
        status: RunStatusDto,
    },
    OutputDelta {
        delta: String,
    },
    ReasoningSummary {
        summary: String,
    },
    ToolActivity {
        activity: ToolActivityDto,
    },
    Usage {
        usage: TokenUsageDto,
    },
    Interaction {
        interaction: InteractionDto,
    },
    Message {
        message: SessionMessageDto,
    },
    Notice {
        reason: String,
        message: String,
    },
    Result {
        result: RunResultDto,
    },
    Failure {
        status: RunStatusDto,
        failure: RunFailureDto,
    },
    Cancellation {
        cancellation: RunCancellationDto,
    },
}

impl From<RunUpdateKind> for RunUpdateKindDto {
    fn from(value: RunUpdateKind) -> Self {
        match value {
            RunUpdateKind::State(status) => Self::State {
                status: status.into(),
            },
            RunUpdateKind::OutputDelta(delta) => Self::OutputDelta { delta },
            RunUpdateKind::ReasoningSummary(summary) => Self::ReasoningSummary { summary },
            RunUpdateKind::ToolActivity(activity) => Self::ToolActivity {
                activity: activity.into(),
            },
            RunUpdateKind::Usage(usage) => Self::Usage {
                usage: usage.into(),
            },
            RunUpdateKind::Interaction(interaction) => Self::Interaction {
                interaction: interaction.into(),
            },
            RunUpdateKind::Message(message) => Self::Message {
                message: message.into(),
            },
            RunUpdateKind::Notice { reason, message } => Self::Notice { reason, message },
            RunUpdateKind::Result(result) => Self::Result {
                result: result.into(),
            },
            RunUpdateKind::Failure { status, failure } => Self::Failure {
                status: status.into(),
                failure: failure.into(),
            },
            RunUpdateKind::Cancellation(cancellation) => Self::Cancellation {
                cancellation: cancellation.into(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunUpdateDto {
    pub(crate) run_id: String,
    pub(crate) sequence: u64,
    pub(crate) created_at: String,
    pub(crate) update: RunUpdateKindDto,
}

impl From<RunUpdate> for RunUpdateDto {
    fn from(value: RunUpdate) -> Self {
        Self {
            run_id: value.run_id,
            sequence: value.sequence,
            created_at: value.created_at,
            update: value.update.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WatchEventDto {
    Update {
        update: Box<RunUpdateDto>,
    },
    Complete {
        #[serde(rename = "runId")]
        run_id: String,
    },
    Error {
        error: CommandErrorDto,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetRunDto {
    pub(crate) run: RunDto,
    pub(crate) pending_interactions: Vec<InteractionDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListRunsDto {
    pub(crate) runs: Vec<RunDto>,
    pub(crate) next_page_token: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunModeInput {
    Execute,
    Plan,
}

impl From<RunModeInput> for RunMode {
    fn from(value: RunModeInput) -> Self {
        match value {
            RunModeInput::Execute => Self::Execute,
            RunModeInput::Plan => Self::Plan,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateRunInput {
    prompt: String,
    session_id: Option<String>,
    role: String,
    mode: RunModeInput,
    max_turns: u32,
    idempotency_key: String,
}

impl CreateRunInput {
    pub(crate) fn into_sdk(self) -> Result<CreateRunRequest, CommandErrorDto> {
        validate_text(&self.prompt, "prompt", false)?;
        if let Some(session_id) = self.session_id.as_deref() {
            validate_identifier(session_id, "sessionId")?;
        }
        if !self.role.is_empty() {
            validate_identifier(&self.role, "role")?;
        }
        if self.max_turns > 100 {
            return Err(CommandErrorDto::invalid(
                "maxTurns",
                "maxTurns must be at most 100.",
            ));
        }
        let idempotency_key =
            IdempotencyKey::new(self.idempotency_key).map_err(CommandErrorDto::from_api)?;
        Ok(CreateRunRequest {
            input: vec![InputContentPart::Text(self.prompt)],
            session_id: self.session_id,
            role: self.role,
            mode: self.mode.into(),
            selected_skills: Vec::new(),
            max_turns: self.max_turns,
            idempotency_key,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GetRunInput {
    run_id: String,
}

impl GetRunInput {
    pub(crate) fn into_sdk(self) -> Result<GetRunRequest, CommandErrorDto> {
        validate_identifier(&self.run_id, "runId")?;
        Ok(GetRunRequest {
            run_id: self.run_id,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListRunsInput {
    session_id: Option<String>,
    #[serde(default)]
    page_token: String,
}

impl ListRunsInput {
    pub(crate) fn into_sdk(self) -> Result<ListRunsRequest, CommandErrorDto> {
        if let Some(session_id) = self.session_id.as_deref() {
            validate_identifier(session_id, "sessionId")?;
        }
        validate_optional_opaque(&self.page_token, "pageToken")?;
        Ok(ListRunsRequest {
            session_id: self.session_id,
            statuses: Vec::new(),
            page: Some(PageRequest {
                page_size: PAGE_SIZE,
                page_token: self.page_token,
            }),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WatchRunInput {
    run_id: String,
    #[serde(default)]
    after_sequence: u64,
}

impl WatchRunInput {
    pub(crate) fn into_sdk(self) -> Result<WatchRunRequest, CommandErrorDto> {
        validate_identifier(&self.run_id, "runId")?;
        Ok(WatchRunRequest {
            run_id: self.run_id,
            after_sequence: self.after_sequence,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CancelRunInput {
    run_id: String,
    idempotency_key: String,
}

impl CancelRunInput {
    pub(crate) fn into_sdk(self) -> Result<CancelRunRequest, CommandErrorDto> {
        validate_identifier(&self.run_id, "runId")?;
        let idempotency_key =
            IdempotencyKey::new(self.idempotency_key).map_err(CommandErrorDto::from_api)?;
        Ok(CancelRunRequest {
            run_id: self.run_id,
            idempotency_key,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum InteractionResponseInput {
    PromptChoice {
        #[serde(rename = "choiceId")]
        choice_id: String,
        label: String,
    },
    PromptText {
        text: String,
    },
    Approval {
        approved: bool,
        #[serde(rename = "requestHash")]
        request_hash: String,
    },
}

impl InteractionResponseInput {
    fn into_sdk(self) -> Result<InteractionAnswer, CommandErrorDto> {
        match self {
            Self::PromptChoice { choice_id, label } => {
                validate_identifier(&choice_id, "response.choiceId")?;
                validate_text(&label, "response.label", false)?;
                Ok(InteractionAnswer::Prompt(PromptAnswer::Choice(
                    PromptChoice { choice_id, label },
                )))
            }
            Self::PromptText { text } => {
                validate_text(&text, "response.text", false)?;
                Ok(InteractionAnswer::Prompt(PromptAnswer::FreeForm(text)))
            }
            Self::Approval {
                approved,
                request_hash,
            } => {
                validate_opaque(&request_hash, "response.requestHash")?;
                Ok(InteractionAnswer::Approval {
                    approved,
                    request_hash,
                })
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RespondInteractionInput {
    run_id: String,
    interaction_id: String,
    etag: String,
    idempotency_key: String,
    response: InteractionResponseInput,
}

impl RespondInteractionInput {
    pub(crate) fn into_sdk(self) -> Result<RespondInteractionRequest, CommandErrorDto> {
        validate_identifier(&self.run_id, "runId")?;
        validate_identifier(&self.interaction_id, "interactionId")?;
        validate_opaque(&self.etag, "etag")?;
        let idempotency_key =
            IdempotencyKey::new(self.idempotency_key).map_err(CommandErrorDto::from_api)?;
        Ok(RespondInteractionRequest {
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            etag: self.etag,
            idempotency_key,
            response: self.response.into_sdk()?,
        })
    }
}

fn validate_identifier(value: &str, field: &str) -> Result<(), CommandErrorDto> {
    let valid = !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        });
    if valid {
        Ok(())
    } else {
        Err(CommandErrorDto::invalid(
            field,
            "The identifier is empty, oversized, or contains unsupported characters.",
        ))
    }
}

fn validate_text(value: &str, field: &str, allow_empty: bool) -> Result<(), CommandErrorDto> {
    if value.len() > MAX_TEXT_BYTES || (!allow_empty && value.trim().is_empty()) {
        Err(CommandErrorDto::invalid(
            field,
            "Text must be non-empty and at most 65536 bytes.",
        ))
    } else {
        Ok(())
    }
}

fn validate_opaque(value: &str, field: &str) -> Result<(), CommandErrorDto> {
    if value.is_empty()
        || value.len() > MAX_OPAQUE_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(CommandErrorDto::invalid(
            field,
            "The opaque value is empty, oversized, or malformed.",
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_opaque(value: &str, field: &str) -> Result<(), CommandErrorDto> {
    if value.is_empty() {
        Ok(())
    } else {
        validate_opaque(value, field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_input_is_bounded_and_cannot_activate_skills() {
        let input: CreateRunInput = serde_json::from_value(json!({
            "prompt": "Plan the release",
            "sessionId": null,
            "role": "primary",
            "mode": "plan",
            "maxTurns": 12,
            "idempotencyKey": "01968a3e-0ab3-7f10-bb27-4eadbd550007"
        }))
        .expect("valid desktop request");
        let request = input.into_sdk().expect("valid SDK request");
        assert!(request.selected_skills.is_empty());
        assert_eq!(request.mode, RunMode::Plan);

        let oversized: CreateRunInput = serde_json::from_value(json!({
            "prompt": "x".repeat(MAX_TEXT_BYTES + 1),
            "sessionId": null,
            "role": "primary",
            "mode": "execute",
            "maxTurns": 1,
            "idempotencyKey": "safe-key"
        }))
        .expect("shape is valid");
        assert_eq!(
            oversized
                .into_sdk()
                .expect_err("oversized prompt rejected")
                .code,
            "invalid_argument"
        );
    }

    #[test]
    fn request_deserialization_rejects_unknown_authority_fields() {
        let result = serde_json::from_value::<CreateRunInput>(json!({
            "prompt": "hello",
            "sessionId": null,
            "role": "primary",
            "mode": "execute",
            "maxTurns": 1,
            "idempotencyKey": "safe-key",
            "selectedSkills": ["admin"]
        }));
        assert!(result.is_err());
    }

    #[test]
    fn interaction_response_uses_camel_case_and_preserves_one_use_binding() {
        let input: RespondInteractionInput = serde_json::from_value(json!({
            "runId": "run-1",
            "interactionId": "interaction-1",
            "etag": "opaque-etag",
            "idempotencyKey": "response-1",
            "response": {
                "type": "approval",
                "approved": false,
                "requestHash": "random-binding"
            }
        }))
        .expect("valid response shape");
        let request = input.into_sdk().expect("valid SDK request");
        assert!(matches!(
            request.response,
            InteractionAnswer::Approval {
                approved: false,
                request_hash
            } if request_hash == "random-binding"
        ));
    }

    #[test]
    fn run_update_serialization_preserves_ordering_and_tag_shape() {
        let dto = RunUpdateDto::from(RunUpdate {
            run_id: "run-1".into(),
            sequence: 7,
            created_at: "2026-07-20T12:00:00Z".into(),
            update: RunUpdateKind::OutputDelta("hello".into()),
        });
        let value = serde_json::to_value(dto).expect("update serializes");
        assert_eq!(value["runId"], "run-1");
        assert_eq!(value["sequence"], 7);
        assert_eq!(value["update"]["type"], "output_delta");
        assert_eq!(value["update"]["delta"], "hello");
    }

    #[test]
    fn approval_dto_exposes_only_the_public_sdk_projection() {
        let interaction = Interaction {
            interaction_id: "interaction-1".into(),
            run_id: "run-1".into(),
            kind: InteractionKind::Approval,
            status: InteractionStatus::Pending,
            created_at: "2026-07-20T12:00:00Z".into(),
            expires_at: "2026-07-20T12:05:00Z".into(),
            respondable_by_caller: true,
            etag: "opaque-etag".into(),
            content: InteractionContent::Approval(ApprovalInteraction {
                reason: "Allow this network action?".into(),
                action: "network_request".into(),
                resource: "https://example.com".into(),
                risk: Some(ApprovalRisk::Medium),
                request_hash: "random-binding".into(),
            }),
        };
        let value = serde_json::to_value(InteractionDto::from(interaction))
            .expect("interaction serializes");
        assert_eq!(value["content"]["type"], "approval");
        assert_eq!(value["content"]["requestHash"], "random-binding");
        assert!(value.get("path").is_none());
        assert!(value.get("arguments").is_none());
    }

    #[test]
    fn native_errors_are_stable_and_redact_sdk_details() {
        let error = CommandErrorDto::from_sdk(SdkError::InvalidConfiguration(
            "secret native configuration detail",
        ));
        let value = serde_json::to_string(&error).expect("error serializes");
        assert!(value.contains("not_configured"));
        assert!(!value.contains("secret native configuration detail"));
        assert!(!value.contains("correlation"));
    }
}
