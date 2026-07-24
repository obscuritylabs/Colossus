use crate::{
    ApprovalInteraction, ApprovalRisk, ArtifactPurpose, ArtifactReference, ArtifactState,
    CancelRunResponse, CreateRunRequest, CreateRunResponse, GetRunResponse, InputContentPart,
    Interaction, InteractionAnswer, InteractionContent, InteractionKind, InteractionStatus,
    ListRunsRequest, ListRunsResponse, MessageContentPart, MessageRole, OutcomeCertainty,
    PageResponse, PromptAnswer, PromptChoice, RespondInteractionRequest,
    RespondInteractionResponse, Run, RunCancellation, RunFailure, RunMode, RunResult, RunStatus,
    RunTerminal, RunUpdate, RunUpdateKind, SessionMessage, TokenUsage, ToolActivity,
    ToolActivityState,
};
use colossus_api::{self as core, ApiError, ApiErrorReason, ApiResult, CallerContext};

pub(super) fn create_request(value: CreateRunRequest) -> core::CreateRunRequest {
    core::CreateRunRequest {
        input: value
            .input
            .into_iter()
            .map(|part| match part {
                InputContentPart::Text(text) => core::ContentPart::Text { text },
            })
            .collect(),
        session_id: value.session_id,
        role: (!value.role.is_empty()).then_some(value.role),
        mode: match value.mode {
            RunMode::Execute => core::RunMode::Execute,
            RunMode::Plan => core::RunMode::Plan,
        },
        skill_ids: value.selected_skills,
        max_turns: value.max_turns,
        idempotency_key: value.idempotency_key,
    }
}

pub(super) fn create_response(value: core::CreateRunResponse) -> ApiResult<CreateRunResponse> {
    Ok(CreateRunResponse {
        run: run(value.run)?,
    })
}

pub(super) fn get_response(value: core::Run, caller: &CallerContext) -> ApiResult<GetRunResponse> {
    let pending_interactions = value
        .pending_interaction
        .as_ref()
        .map(|interaction| {
            public_interaction(interaction, &value.id, Some(&value.etag), caller)
                .map(|value| vec![value])
        })
        .transpose()?
        .unwrap_or_default();
    Ok(GetRunResponse {
        run: run(value)?,
        pending_interactions,
    })
}

pub(super) fn list_request(value: ListRunsRequest) -> core::ListRunsRequest {
    let (page_size, page_token) = value.page.map_or((0, None), |page| {
        (
            page.page_size,
            (!page.page_token.is_empty()).then_some(page.page_token),
        )
    });
    core::ListRunsRequest {
        session_id: value.session_id,
        statuses: value.statuses.into_iter().map(core_run_status).collect(),
        page_size,
        page_token,
    }
}

pub(super) fn list_response(value: core::ListRunsResponse) -> ApiResult<ListRunsResponse> {
    Ok(ListRunsResponse {
        runs: value
            .runs
            .into_iter()
            .map(run)
            .collect::<ApiResult<Vec<_>>>()?,
        page: Some(PageResponse {
            next_page_token: value.next_page_token.unwrap_or_default(),
        }),
    })
}

pub(super) fn cancel_response(value: core::Run) -> ApiResult<CancelRunResponse> {
    Ok(CancelRunResponse { run: run(value)? })
}

pub(super) fn interaction_request(
    value: RespondInteractionRequest,
) -> ApiResult<core::RespondInteractionRequest> {
    let response = match value.response {
        InteractionAnswer::Prompt(answer) => match answer {
            PromptAnswer::Choice(choice) => {
                let selected_index = parse_choice_id(&choice.choice_id)?;
                core::InteractionResponse::Prompt {
                    answer: choice.label,
                    selected_index: Some(selected_index),
                }
            }
            PromptAnswer::FreeForm(answer) => core::InteractionResponse::Prompt {
                answer,
                selected_index: None,
            },
        },
        InteractionAnswer::Approval {
            approved,
            request_hash,
        } => core::InteractionResponse::Approval {
            approved,
            request_hash,
        },
    };
    Ok(core::RespondInteractionRequest {
        run_id: value.run_id,
        interaction_id: value.interaction_id,
        etag: value.etag,
        idempotency_key: value.idempotency_key,
        response,
    })
}

pub(super) fn interaction_response(
    value: core::Interaction,
    run_id: &str,
    caller: &CallerContext,
) -> ApiResult<RespondInteractionResponse> {
    Ok(RespondInteractionResponse {
        interaction: public_interaction(&value, run_id, None, caller)?,
    })
}

pub(super) fn run_update(
    value: core::RunUpdate,
    interaction_etag: Option<&str>,
    caller: &CallerContext,
) -> ApiResult<RunUpdate> {
    let update = match value.kind {
        core::RunUpdateKind::State { status } => RunUpdateKind::State(run_status(status)),
        core::RunUpdateKind::OutputDelta { text } => RunUpdateKind::OutputDelta(text),
        core::RunUpdateKind::ReasoningSummary { summary } => {
            RunUpdateKind::ReasoningSummary(summary)
        }
        core::RunUpdateKind::ToolActivity { activity } => {
            RunUpdateKind::ToolActivity(ToolActivity {
                call_id: activity.call_id,
                tool_name: activity.tool_name,
                state: match activity.state {
                    core::ToolActivityState::Requested => ToolActivityState::Requested,
                    core::ToolActivityState::WaitingApproval => ToolActivityState::WaitingApproval,
                    core::ToolActivityState::Started => ToolActivityState::Started,
                    core::ToolActivityState::Completed => ToolActivityState::Completed,
                    core::ToolActivityState::Failed => ToolActivityState::Failed,
                    core::ToolActivityState::OutcomeUnknown => ToolActivityState::OutcomeUnknown,
                },
                summary: activity.summary,
            })
        }
        core::RunUpdateKind::Usage { usage } => RunUpdateKind::Usage(TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }),
        core::RunUpdateKind::Interaction { interaction } => RunUpdateKind::Interaction(
            public_interaction(&interaction, &value.run_id, interaction_etag, caller)?,
        ),
        core::RunUpdateKind::Message { message } => {
            RunUpdateKind::Message(released_message(message))
        }
        core::RunUpdateKind::Notice { notice } => RunUpdateKind::Notice {
            reason: notice.reason,
            message: notice.message,
        },
        core::RunUpdateKind::Result { result } => RunUpdateKind::Result(run_result(result)),
        core::RunUpdateKind::Failure { status, failure } => RunUpdateKind::Failure {
            status: run_status(status),
            failure: run_failure(failure),
        },
        core::RunUpdateKind::Cancellation { cancellation } => {
            RunUpdateKind::Cancellation(RunCancellation {
                turn: cancellation.turn,
                message: cancellation.message,
            })
        }
    };
    Ok(RunUpdate {
        run_id: value.run_id,
        sequence: value.sequence,
        created_at: value.occurred_at,
        update,
    })
}

fn run(value: core::Run) -> ApiResult<Run> {
    let terminal = match value.status {
        core::RunStatus::Completed => Some(RunTerminal::Result(run_result(
            value.result.ok_or_else(projection_error)?,
        ))),
        core::RunStatus::Failed
        | core::RunStatus::Interrupted
        | core::RunStatus::OutcomeUnknown => Some(RunTerminal::Failure(run_failure(
            value.failure.ok_or_else(projection_error)?,
        ))),
        core::RunStatus::Cancelled => {
            let cancellation = value.cancellation.ok_or_else(projection_error)?;
            Some(RunTerminal::Cancellation(RunCancellation {
                turn: cancellation.turn,
                message: cancellation.message,
            }))
        }
        _ if value.result.is_none() && value.failure.is_none() && value.cancellation.is_none() => {
            None
        }
        _ => return Err(projection_error()),
    };
    Ok(Run {
        run_id: value.id,
        session_id: value.session_id,
        role: value.role,
        mode: match value.mode {
            core::RunMode::Execute => RunMode::Execute,
            core::RunMode::Plan => RunMode::Plan,
        },
        status: run_status(value.status),
        created_at: value.created_at,
        updated_at: value.updated_at,
        started_at: value.started_at,
        finished_at: value.finished_at,
        last_sequence: value.last_sequence,
        pending_interaction_count: u32::from(value.pending_interaction.is_some()),
        terminal,
        etag: value.etag,
        selected_skills: value.skill_ids,
    })
}

fn run_result(value: core::RunResult) -> RunResult {
    RunResult {
        output: value.output,
        profile: value.profile,
        model_profile: value.model_profile,
        provider_profile: value.provider_profile,
        model: value.model,
        elapsed_seconds: value.elapsed_seconds,
    }
}

fn run_failure(value: core::RunFailure) -> RunFailure {
    RunFailure {
        reason: value.code,
        message: value.message,
        outcome_certainty: match value.outcome {
            core::OutcomeCertainty::Known => OutcomeCertainty::Known,
            core::OutcomeCertainty::Unknown => OutcomeCertainty::Unknown,
        },
    }
}

fn run_status(value: core::RunStatus) -> RunStatus {
    match value {
        core::RunStatus::Queued => RunStatus::Queued,
        core::RunStatus::Running => RunStatus::Running,
        core::RunStatus::Waiting => RunStatus::Waiting,
        core::RunStatus::Cancelling => RunStatus::Cancelling,
        core::RunStatus::Completed => RunStatus::Completed,
        core::RunStatus::Failed => RunStatus::Failed,
        core::RunStatus::Cancelled => RunStatus::Cancelled,
        core::RunStatus::Interrupted => RunStatus::Interrupted,
        core::RunStatus::OutcomeUnknown => RunStatus::OutcomeUnknown,
    }
}

fn core_run_status(value: RunStatus) -> core::RunStatus {
    match value {
        RunStatus::Queued => core::RunStatus::Queued,
        RunStatus::Running => core::RunStatus::Running,
        RunStatus::Waiting => core::RunStatus::Waiting,
        RunStatus::Cancelling => core::RunStatus::Cancelling,
        RunStatus::Completed => core::RunStatus::Completed,
        RunStatus::Failed => core::RunStatus::Failed,
        RunStatus::Cancelled => core::RunStatus::Cancelled,
        RunStatus::Interrupted => core::RunStatus::Interrupted,
        RunStatus::OutcomeUnknown => core::RunStatus::OutcomeUnknown,
    }
}

fn public_interaction(
    value: &core::Interaction,
    run_id: &str,
    etag: Option<&str>,
    caller: &CallerContext,
) -> ApiResult<Interaction> {
    let (kind, content, response_scope) = match value.kind {
        core::InteractionKind::Prompt => (
            InteractionKind::UserPrompt,
            InteractionContent::UserPrompt(crate::UserPromptInteraction {
                question: value.prompt.clone(),
                choices: value
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(index, label)| PromptChoice {
                        choice_id: format!("choice:{index}"),
                        label: label.clone(),
                    })
                    .collect(),
                allow_free_form: value.allow_free_form,
            }),
            core::scopes::PROMPTS_RESPOND,
        ),
        core::InteractionKind::Approval => {
            let action = value.action.clone().ok_or_else(projection_error)?;
            let resource = value.resource.clone().ok_or_else(projection_error)?;
            core::validate_public_approval_display(&action, &resource)
                .map_err(|_| projection_error())?;
            (
                InteractionKind::Approval,
                InteractionContent::Approval(ApprovalInteraction {
                    reason: value.prompt.clone(),
                    action,
                    resource,
                    risk: value.risk.map(|risk| match risk {
                        core::ApprovalRisk::Low => ApprovalRisk::Low,
                        core::ApprovalRisk::Medium => ApprovalRisk::Medium,
                        core::ApprovalRisk::High => ApprovalRisk::High,
                    }),
                    request_hash: value.request_hash.clone().ok_or_else(projection_error)?,
                }),
                core::scopes::APPROVALS_RESPOND,
            )
        }
    };
    let status = match value.status {
        core::InteractionStatus::Pending => InteractionStatus::Pending,
        core::InteractionStatus::Responded => InteractionStatus::Answered,
        core::InteractionStatus::Expired => InteractionStatus::Expired,
        core::InteractionStatus::Cancelled => InteractionStatus::Cancelled,
    };
    let respondable_by_caller = value.status == core::InteractionStatus::Pending
        && value.application_id == caller.principal().application_id()
        && caller.principal().has_scope(response_scope)
        && etag.is_some();
    Ok(Interaction {
        interaction_id: value.id.clone(),
        run_id: run_id.into(),
        kind,
        status,
        created_at: value.created_at.clone(),
        expires_at: value.expires_at.clone(),
        respondable_by_caller,
        etag: etag.unwrap_or_default().into(),
        content,
    })
}

fn released_message(value: core::ReleasedSessionMessage) -> SessionMessage {
    SessionMessage {
        session_id: value.session_id,
        run_id: value.run_id,
        sequence: value.sequence,
        role: match value.role {
            core::ReleasedMessageRole::User => MessageRole::User,
            core::ReleasedMessageRole::Assistant => MessageRole::Assistant,
            core::ReleasedMessageRole::Tool => MessageRole::Tool,
            core::ReleasedMessageRole::System => MessageRole::System,
        },
        content: value
            .content
            .into_iter()
            .map(|part| match part {
                core::ReleasedContentPart::Text { text } => MessageContentPart::Text(text),
                core::ReleasedContentPart::Artifact { artifact } => {
                    MessageContentPart::Artifact(ArtifactReference {
                        artifact_id: artifact.artifact_id,
                        file_name: artifact.file_name,
                        media_type: artifact.media_type,
                        size_bytes: artifact.size_bytes,
                        sha256: artifact.sha256,
                        purpose: match artifact.purpose {
                            core::ReleasedArtifactPurpose::RunInput => ArtifactPurpose::RunInput,
                            core::ReleasedArtifactPurpose::RunOutput => ArtifactPurpose::RunOutput,
                            core::ReleasedArtifactPurpose::Workflow => ArtifactPurpose::Workflow,
                            core::ReleasedArtifactPurpose::Extension => ArtifactPurpose::Extension,
                            core::ReleasedArtifactPurpose::Archive => ArtifactPurpose::Archive,
                        },
                        state: match artifact.state {
                            core::ReleasedArtifactState::Uploading => ArtifactState::Uploading,
                            core::ReleasedArtifactState::Quarantined => ArtifactState::Quarantined,
                            core::ReleasedArtifactState::Available => ArtifactState::Available,
                            core::ReleasedArtifactState::Rejected => ArtifactState::Rejected,
                            core::ReleasedArtifactState::Expired => ArtifactState::Expired,
                        },
                        created_at: artifact.created_at,
                    })
                }
            })
            .collect(),
        created_at: value.created_at,
    }
}

fn parse_choice_id(value: &str) -> ApiResult<u32> {
    let digits = value.strip_prefix("choice:").ok_or_else(|| {
        ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "response.prompt.choice_id",
            "choice identifier is malformed",
        )
    })?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "response.prompt.choice_id",
            "choice identifier is malformed",
        ));
    }
    digits.parse().map_err(|_| {
        ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "response.prompt.choice_id",
            "choice identifier is outside the supported range",
        )
    })
}

fn projection_error() -> ApiError {
    ApiError {
        code: core::ApiErrorCode::Internal,
        reason: ApiErrorReason::InternalInvariant,
        message: "the embedded public projection is invalid".into(),
        correlation_id: None,
        retryable: false,
        outcome: core::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_api::{ApiErrorCode, ApiScope, ApplicationKind, ApplicationPrincipal, RequestId};

    fn caller() -> CallerContext {
        CallerContext::authenticated(
            ApplicationPrincipal::authenticated(
                "app:test",
                "credential:test",
                ApplicationKind::Embedded,
                [ApiScope::new(core::scopes::APPROVALS_RESPOND).expect("approval scope")],
                Vec::<String>::new(),
                Vec::<String>::new(),
            )
            .expect("principal"),
            RequestId::new("request:test").expect("request id"),
        )
    }

    #[test]
    fn embedded_projection_rejects_private_approval_display_without_echoing_it() {
        let private_action = "filesystem.write.customer-secret";
        let private_resource = "/Users/alex/private/customer-secret.txt";
        let value = core::Interaction {
            id: "interaction-private-approval".into(),
            kind: core::InteractionKind::Approval,
            status: core::InteractionStatus::Pending,
            application_id: "app:test".into(),
            created_at: "2026-07-19T12:00:00Z".into(),
            prompt: "An effect requires explicit approval".into(),
            choices: Vec::new(),
            allow_free_form: false,
            request_hash: Some("ab".repeat(32)),
            action: Some(private_action.into()),
            resource: Some(private_resource.into()),
            risk: Some(core::ApprovalRisk::High),
            expires_at: "2026-07-19T12:05:00Z".into(),
            response: None,
            responded_at: None,
        };

        let error = public_interaction(&value, "run-1", Some("etag-1"), &caller())
            .expect_err("private approval display must not cross the embedded boundary");
        assert_eq!(error.code, ApiErrorCode::Internal);
        assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
        assert_eq!(error.message, "the embedded public projection is invalid");
        assert!(!error.to_string().contains(private_action));
        assert!(!error.to_string().contains(private_resource));
    }
}
