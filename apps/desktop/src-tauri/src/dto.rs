use colossus_sdk::{
    ApiError, ApiErrorCode, ApprovalInteraction, ApprovalRisk, ArtifactPurpose, ArtifactReference,
    ArtifactState, CancelRunRequest, CreateRunRequest, FieldViolation, GetRunRequest,
    IdempotencyKey, InputContentPart, Interaction, InteractionAnswer, InteractionContent,
    InteractionKind, InteractionStatus, ListRunsRequest, MessageContentPart, MessageRole,
    OutcomeCertainty, PageRequest, PlanExecutionStrategy, PlanRunAction, PlanStatus, PromptAnswer,
    PromptChoice, RespondInteractionRequest, Run, RunCancellation, RunFailure, RunMode, RunResult,
    RunStatus, RunTerminal, RunUpdate, RunUpdateKind, SdkError, SessionMessage, TokenUsage,
    ToolActivity, ToolActivityState, WatchRunRequest,
};
use serde::{Deserialize, Serialize};

use crate::terminal::{
    TerminalError, TerminalEvent, TerminalKind, TerminalPlanContext, TerminalSignal,
};

const MAX_TEXT_BYTES: usize = 65_536;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_OPAQUE_BYTES: usize = 512;
const MAX_ENCODED_TERMINAL_INPUT_BYTES: usize = 87_384;
const PAGE_SIZE: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionStateDto {
    Connected,
    Disconnected,
    NotConfigured,
    Starting,
    Restarting,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionStatusDto {
    pub(crate) state: ConnectionStateDto,
    pub(crate) message: String,
    pub(crate) target_id: Option<String>,
}

impl ConnectionStatusDto {
    pub(crate) fn connected(target_id: impl Into<String>) -> Self {
        Self {
            state: ConnectionStateDto::Connected,
            message: "Connected to Colossus.".into(),
            target_id: Some(target_id.into()),
        }
    }

    pub(crate) fn disconnected(target_id: Option<String>) -> Self {
        Self {
            state: ConnectionStateDto::Disconnected,
            message: "Colossus is not connected.".into(),
            target_id,
        }
    }

    pub(crate) fn not_configured() -> Self {
        Self {
            state: ConnectionStateDto::NotConfigured,
            message: "Desktop enrollment is not configured.".into(),
            target_id: None,
        }
    }

    pub(crate) fn managed(state: ConnectionStateDto, message: impl Into<String>) -> Self {
        Self {
            state,
            message: message.into(),
            target_id: Some(crate::state::MANAGED_TARGET_ID.into()),
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
    pub(crate) fn local_sanitized(code: &str, message: &str, retryable: bool) -> Self {
        Self::local(code, message, retryable, false)
    }

    pub(crate) fn not_configured() -> Self {
        Self::local(
            "not_configured",
            "Desktop enrollment is not configured.",
            false,
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
            SdkError::WorkspaceIdentityChanged => Self::local(
                "workspace_changed",
                "The selected workspace changed. Choose the workspace again.",
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

    pub(crate) fn from_terminal(error: TerminalError) -> Self {
        Self::local(error.code(), error.message(), error.retryable(), false)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalKindDto {
    ColossusTui,
    Shell,
}

impl From<TerminalKindDto> for TerminalKind {
    fn from(value: TerminalKindDto) -> Self {
        match value {
            TerminalKindDto::ColossusTui => Self::ColossusTui,
            TerminalKindDto::Shell => Self::Shell,
        }
    }
}

impl From<TerminalKind> for TerminalKindDto {
    fn from(value: TerminalKind) -> Self {
        match value {
            TerminalKind::ColossusTui => Self::ColossusTui,
            TerminalKind::Shell => Self::Shell,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ShowTerminalInput {
    pub(crate) kind: TerminalKindDto,
    pub(crate) session_id: Option<String>,
    pub(crate) plan_id: Option<String>,
}

impl ShowTerminalInput {
    pub(crate) fn into_launch(
        self,
    ) -> Result<(TerminalKind, Option<TerminalPlanContext>), CommandErrorDto> {
        let kind = TerminalKind::from(self.kind);
        let plan_context = match (self.session_id, self.plan_id) {
            (None, None) => None,
            (Some(session_id), Some(plan_id)) if kind == TerminalKind::ColossusTui => {
                validate_identifier(&session_id, "sessionId")?;
                validate_identifier(&plan_id, "planId")?;
                Some(TerminalPlanContext {
                    session_id,
                    plan_id,
                })
            }
            _ => {
                return Err(CommandErrorDto::invalid(
                    "planId",
                    "Plan context requires both identifiers and the Colossus TUI.",
                ));
            }
        };
        Ok((kind, plan_context))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalContextDto {
    pub(crate) enabled: bool,
    pub(crate) shell_enabled: bool,
    pub(crate) tui_enabled: bool,
    pub(crate) context_generation: u64,
    pub(crate) launch_request_id: u64,
    pub(crate) workspace_id: Option<String>,
    pub(crate) workspace_name: Option<String>,
    pub(crate) requested_kind: Option<TerminalKindDto>,
    pub(crate) requested_plan_session_id: Option<String>,
    pub(crate) requested_plan_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OpenTerminalInput {
    pub(crate) workspace_id: String,
    pub(crate) context_generation: u64,
    pub(crate) kind: TerminalKindDto,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
}

impl OpenTerminalInput {
    pub(crate) fn validate(&self) -> Result<(), CommandErrorDto> {
        validate_identifier(&self.workspace_id, "workspaceId")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenTerminalDto {
    pub(crate) session_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WriteTerminalInput {
    pub(crate) session_id: String,
    pub(crate) data_base64: String,
}

impl WriteTerminalInput {
    pub(crate) fn decode(&self) -> Result<Vec<u8>, CommandErrorDto> {
        use base64::Engine as _;

        validate_identifier(&self.session_id, "sessionId")?;
        if self.data_base64.len() > MAX_ENCODED_TERMINAL_INPUT_BYTES {
            return Err(CommandErrorDto::invalid(
                "dataBase64",
                "Terminal input exceeds the per-request limit.",
            ));
        }
        base64::engine::general_purpose::STANDARD
            .decode(&self.data_base64)
            .map_err(|_| {
                CommandErrorDto::invalid("dataBase64", "Terminal input is not canonical base64.")
            })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResizeTerminalInput {
    pub(crate) session_id: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
}

impl ResizeTerminalInput {
    pub(crate) fn validate(&self) -> Result<(), CommandErrorDto> {
        validate_identifier(&self.session_id, "sessionId")
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalSignalDto {
    Interrupt,
    Terminate,
}

impl From<TerminalSignalDto> for TerminalSignal {
    fn from(value: TerminalSignalDto) -> Self {
        match value {
            TerminalSignalDto::Interrupt => Self::Interrupt,
            TerminalSignalDto::Terminate => Self::Terminate,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SignalTerminalInput {
    pub(crate) session_id: String,
    pub(crate) signal: TerminalSignalDto,
}

impl SignalTerminalInput {
    pub(crate) fn validate(&self) -> Result<(), CommandErrorDto> {
        validate_identifier(&self.session_id, "sessionId")
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CloseTerminalInput {
    pub(crate) session_id: String,
}

impl CloseTerminalInput {
    pub(crate) fn validate(&self) -> Result<(), CommandErrorDto> {
        validate_identifier(&self.session_id, "sessionId")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TerminalEventDto {
    Output {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "dataBase64")]
        data_base64: String,
    },
    Exited {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "exitCode")]
        exit_code: Option<u32>,
        signal: Option<String>,
    },
    Error {
        #[serde(rename = "sessionId")]
        session_id: String,
        code: String,
        message: String,
    },
}

impl From<TerminalEvent> for TerminalEventDto {
    fn from(value: TerminalEvent) -> Self {
        use base64::Engine as _;

        match value {
            TerminalEvent::Output { session_id, bytes } => Self::Output {
                session_id,
                data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
            TerminalEvent::Exited {
                session_id,
                exit_code,
                signal,
            } => Self::Exited {
                session_id,
                exit_code,
                signal,
            },
            TerminalEvent::Failed {
                session_id,
                code,
                message,
            } => Self::Error {
                session_id,
                code: code.into(),
                message: message.into(),
            },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_status: Option<PlanStatusDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) goal_id: Option<String>,
    pub(crate) profile: String,
    pub(crate) model_profile: String,
    pub(crate) provider_profile: String,
    pub(crate) model: String,
    pub(crate) elapsed_seconds: f64,
}

impl From<RunResult> for RunResultDto {
    fn from(value: RunResult) -> Self {
        Self {
            output: value.output,
            plan_id: value.plan_id,
            plan_revision: value.plan_revision,
            plan_status: value.plan_status.map(Into::into),
            goal_id: value.goal_id,
            profile: value.profile,
            model_profile: value.model_profile,
            provider_profile: value.provider_profile,
            model: value.model,
            elapsed_seconds: value.elapsed_seconds,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanStatusDto {
    Draft,
    Approved,
    Executed,
    Discarded,
}

impl From<PlanStatus> for PlanStatusDto {
    fn from(value: PlanStatus) -> Self {
        match value {
            PlanStatus::Draft => Self::Draft,
            PlanStatus::Approved => Self::Approved,
            PlanStatus::Executed => Self::Executed,
            PlanStatus::Discarded => Self::Discarded,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunFailureDto {
    pub(crate) reason: String,
    pub(crate) message: String,
    pub(crate) outcome_certainty: OutcomeCertaintyDto,
    pub(crate) recoverable: bool,
    pub(crate) http_status: Option<u16>,
    pub(crate) retry_after_ms: Option<u64>,
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
            recoverable: value.recoverable,
            http_status: value.http_status,
            retry_after_ms: value.retry_after_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunCancellationDto {
    pub(crate) turn: u32,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan_status: Option<PlanStatusDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) goal_id: Option<String>,
}

impl From<RunCancellation> for RunCancellationDto {
    fn from(value: RunCancellation) -> Self {
        Self {
            turn: value.turn,
            message: value.message,
            plan_id: value.plan_id,
            plan_revision: value.plan_revision,
            plan_status: value.plan_status.map(Into::into),
            goal_id: value.goal_id,
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
    pub(crate) title: String,
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
            title: value.title,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactContentDto {
    pub(crate) artifact: ArtifactReferenceDto,
    pub(crate) text: String,
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

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PlanExecutionStrategyInput {
    Direct,
    Goal {
        #[serde(rename = "maxIterations")]
        max_iterations: u16,
    },
}

impl From<PlanExecutionStrategyInput> for PlanExecutionStrategy {
    fn from(value: PlanExecutionStrategyInput) -> Self {
        match value {
            PlanExecutionStrategyInput::Direct => Self::Direct,
            PlanExecutionStrategyInput::Goal { max_iterations } => Self::Goal { max_iterations },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PlanRunActionInput {
    Revise {
        #[serde(rename = "sourceRunId")]
        source_run_id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
    },
    Execute {
        #[serde(rename = "sourceRunId")]
        source_run_id: String,
        #[serde(rename = "expectedRevision")]
        expected_revision: u64,
        strategy: PlanExecutionStrategyInput,
    },
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
    #[serde(default)]
    artifact_ids: Vec<String>,
    session_id: Option<String>,
    role: String,
    mode: RunModeInput,
    #[serde(default)]
    plan_action: Option<PlanRunActionInput>,
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
        if self.artifact_ids.len() > 16 {
            return Err(CommandErrorDto::invalid(
                "artifactIds",
                "A run can include at most 16 attachments.",
            ));
        }
        let mut input = vec![InputContentPart::Text(self.prompt)];
        for artifact_id in self.artifact_ids {
            validate_identifier(&artifact_id, "artifactIds")?;
            input.push(InputContentPart::Artifact(artifact_id));
        }
        let plan_action = self
            .plan_action
            .map(|action| match action {
                PlanRunActionInput::Revise {
                    source_run_id,
                    expected_revision,
                } => {
                    validate_identifier(&source_run_id, "planAction.sourceRunId")?;
                    if expected_revision == 0 {
                        return Err(CommandErrorDto::invalid(
                            "planAction.expectedRevision",
                            "The Plan revision must be greater than zero.",
                        ));
                    }
                    Ok(PlanRunAction::Revise {
                        source_run_id,
                        expected_revision,
                    })
                }
                PlanRunActionInput::Execute {
                    source_run_id,
                    expected_revision,
                    strategy,
                } => {
                    validate_identifier(&source_run_id, "planAction.sourceRunId")?;
                    if expected_revision == 0 {
                        return Err(CommandErrorDto::invalid(
                            "planAction.expectedRevision",
                            "The Plan revision must be greater than zero.",
                        ));
                    }
                    if let PlanExecutionStrategyInput::Goal { max_iterations } = strategy
                        && !(1..=50).contains(&max_iterations)
                    {
                        return Err(CommandErrorDto::invalid(
                            "planAction.strategy.maxIterations",
                            "Goal iterations must be in 1..=50.",
                        ));
                    }
                    Ok(PlanRunAction::Execute {
                        source_run_id,
                        expected_revision,
                        strategy: strategy.into(),
                    })
                }
            })
            .transpose()?;
        Ok(CreateRunRequest {
            input,
            session_id: self.session_id,
            role: self.role,
            mode: self.mode.into(),
            selected_skills: Vec::new(),
            plan_action,
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
        assert!(request.plan_action.is_none());
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
    fn create_input_preserves_the_configured_turn_default_sentinel() {
        let input: CreateRunInput = serde_json::from_value(json!({
            "prompt": "Continue until the work is complete",
            "sessionId": null,
            "role": "primary",
            "mode": "execute",
            "maxTurns": 0,
            "idempotencyKey": "01968a3e-0ab3-7f10-bb27-4eadbd550010"
        }))
        .expect("defaulted desktop request");

        let request = input.into_sdk().expect("valid SDK request");
        assert_eq!(request.max_turns, 0);
    }

    #[test]
    fn plan_action_is_exact_revision_bound_and_goal_budgeted() {
        let input: CreateRunInput = serde_json::from_value(json!({
            "prompt": "Run the reviewed plan",
            "sessionId": "session-1",
            "role": "primary",
            "mode": "execute",
            "planAction": {
                "type": "execute",
                "sourceRunId": "run-plan-source",
                "expectedRevision": 4,
                "strategy": {
                    "type": "goal",
                    "maxIterations": 5
                }
            },
            "maxTurns": 12,
            "idempotencyKey": "01968a3e-0ab3-7f10-bb27-4eadbd550008"
        }))
        .expect("valid Plan action");
        assert!(matches!(
            input.into_sdk().expect("valid SDK request").plan_action,
            Some(PlanRunAction::Execute {
                source_run_id,
                expected_revision: 4,
                strategy: PlanExecutionStrategy::Goal { max_iterations: 5 },
            }) if source_run_id == "run-plan-source"
        ));

        let invalid: CreateRunInput = serde_json::from_value(json!({
            "prompt": "Run the reviewed plan",
            "sessionId": "session-1",
            "role": "primary",
            "mode": "execute",
            "planAction": {
                "type": "execute",
                "sourceRunId": "run-plan-source",
                "expectedRevision": 4,
                "strategy": {
                    "type": "goal",
                    "maxIterations": 51
                }
            },
            "maxTurns": 12,
            "idempotencyKey": "01968a3e-0ab3-7f10-bb27-4eadbd550009"
        }))
        .expect("bounded type shape");
        assert_eq!(
            invalid
                .into_sdk()
                .expect_err("oversized Goal budget rejected")
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
    fn run_terminal_dto_preserves_optional_plan_identity() {
        let result = serde_json::to_value(RunResultDto::from(RunResult {
            output: "Draft saved.".into(),
            plan_id: Some("plan-1".into()),
            plan_revision: Some(3),
            plan_status: Some(PlanStatus::Draft),
            goal_id: None,
            profile: "primary".into(),
            model_profile: "primary".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            elapsed_seconds: 0.5,
        }))
        .expect("result serializes");
        assert_eq!(result["planId"], "plan-1");
        assert_eq!(result["planRevision"], 3);
        assert_eq!(result["planStatus"], "draft");

        let cancellation = serde_json::to_value(RunCancellationDto::from(RunCancellation {
            turn: 2,
            message: "Cancelled after the draft was saved.".into(),
            plan_id: Some("plan-2".into()),
            plan_revision: Some(2),
            plan_status: Some(PlanStatus::Draft),
            goal_id: None,
        }))
        .expect("cancellation serializes");
        assert_eq!(cancellation["planId"], "plan-2");

        let execute_result = serde_json::to_value(RunResultDto::from(RunResult {
            output: "Done.".into(),
            plan_id: None,
            plan_revision: None,
            plan_status: None,
            goal_id: None,
            profile: "primary".into(),
            model_profile: "primary".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            elapsed_seconds: 0.5,
        }))
        .expect("execute result serializes");
        assert!(execute_result.get("planId").is_none());
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

    #[test]
    fn terminal_plan_handoff_accepts_only_bounded_tui_selection() {
        let request: ShowTerminalInput = serde_json::from_value(json!({
            "kind": "colossus_tui",
            "sessionId": "session-1",
            "planId": "plan-1"
        }))
        .expect("plan handoff shape");
        let (kind, context) = request.into_launch().expect("valid plan handoff");
        assert_eq!(kind, TerminalKind::ColossusTui);
        assert_eq!(
            context,
            Some(TerminalPlanContext {
                session_id: "session-1".into(),
                plan_id: "plan-1".into(),
            })
        );

        for invalid in [
            json!({
                "kind": "shell",
                "sessionId": "session-1",
                "planId": "plan-1"
            }),
            json!({
                "kind": "colossus_tui",
                "sessionId": "session-1"
            }),
            json!({
                "kind": "colossus_tui",
                "sessionId": "session-1\n/plan approve",
                "planId": "plan-1"
            }),
        ] {
            let request: ShowTerminalInput = serde_json::from_value(invalid).expect("known fields");
            assert!(request.into_launch().is_err());
        }
    }

    #[test]
    fn terminal_inputs_cannot_choose_process_or_path_authority() {
        let result = serde_json::from_value::<OpenTerminalInput>(json!({
            "workspaceId": "workspace:managed",
            "contextGeneration": 7,
            "kind": "shell",
            "rows": 24,
            "cols": 80
        }))
        .expect("fixed shell kind");
        assert_eq!(result.kind, TerminalKindDto::Shell);

        for field in ["executable", "program", "arguments", "environment", "cwd"] {
            let mut value = json!({
                "workspaceId": "workspace:managed",
                "contextGeneration": 7,
                "kind": "shell",
                "rows": 24,
                "cols": 80
            });
            value[field] = json!("/bin/other");
            assert!(
                serde_json::from_value::<OpenTerminalInput>(value).is_err(),
                "renderer field {field} must be rejected"
            );
        }

        assert!(
            serde_json::from_value::<OpenTerminalInput>(json!({
                "workspaceId": "workspace:managed",
                "contextGeneration": 7,
                "kind": "colossus_tui",
                "rows": 24,
                "cols": 80,
                "executable": "/bin/other"
            }))
            .is_err()
        );

        assert!(
            serde_json::from_value::<OpenTerminalInput>(json!({
                "workspaceId": "workspace:managed",
                "contextGeneration": 7,
                "kind": "colossus_tui",
                "rows": 24,
                "cols": 80,
                "cwd": "/private/tmp"
            }))
            .is_err()
        );
    }

    #[test]
    fn terminal_output_is_encoded_without_native_paths_or_arguments() {
        let event = TerminalEventDto::from(TerminalEvent::Output {
            session_id: "session-1".into(),
            bytes: b"hello".to_vec(),
        });
        let value = serde_json::to_value(event).expect("terminal event serializes");
        assert_eq!(value["type"], "output");
        assert_eq!(value["dataBase64"], "aGVsbG8=");
        assert!(value.get("workspace").is_none());
        assert!(value.get("executable").is_none());
    }
}
