//! Native approval challenge validation shared with the opt-in runtime acceptance driver.
//! No window, credentials, OS confirmation, or authority is accepted from a renderer.

use colossus_sdk::{
    ApiError, ApiErrorReason, ApiResult, ApprovalInteraction, Colossus, GetRunRequest, Interaction,
    InteractionAnswer, InteractionContent, InteractionKind, InteractionStatus,
    RespondInteractionRequest,
};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandApprovalContextDto {
    justification: String,
    executable: String,
    arguments: Vec<String>,
    working_directory: String,
    redacted: bool,
}

impl From<colossus_sdk::CommandApprovalContext> for CommandApprovalContextDto {
    fn from(value: colossus_sdk::CommandApprovalContext) -> Self {
        Self {
            justification: value.justification,
            executable: value.executable,
            arguments: value.arguments,
            working_directory: value.working_directory,
            redacted: value.redacted,
        }
    }
}

pub(crate) async fn pending(
    client: &Colossus,
    request: &RespondInteractionRequest,
) -> ApiResult<ApprovalInteraction> {
    let details = bounded_lookup(
        client.get_run(GetRunRequest {
            run_id: request.run_id.clone(),
        }),
        std::time::Duration::from_secs(5),
    )
    .await?;
    if details.run.run_id != request.run_id {
        return Err(unavailable());
    }
    let interaction = details
        .pending_interactions
        .iter()
        .find(|interaction| interaction.interaction_id == request.interaction_id)
        .ok_or_else(unavailable)?;
    validate(interaction, request)
}

async fn bounded_lookup<T>(
    lookup: impl std::future::Future<Output = ApiResult<T>>,
    deadline: std::time::Duration,
) -> ApiResult<T> {
    tokio::time::timeout(deadline, lookup)
        .await
        .map_err(|_| unavailable())?
}

fn validate(
    interaction: &Interaction,
    request: &RespondInteractionRequest,
) -> ApiResult<ApprovalInteraction> {
    let InteractionContent::Approval(approval) = &interaction.content else {
        return Err(unavailable());
    };
    let InteractionAnswer::Approval { request_hash, .. } = &request.response else {
        return Err(unavailable());
    };
    if interaction.run_id != request.run_id
        || interaction.interaction_id != request.interaction_id
        || interaction.kind != InteractionKind::Approval
        || interaction.status != InteractionStatus::Pending
        || !interaction.respondable_by_caller
        || interaction.etag.is_empty()
        || interaction.etag != request.etag
        || approval.request_hash != *request_hash
    {
        return Err(unavailable());
    }
    if let Some(context) = &approval.command_context {
        if approval.action != "process.execute" {
            return Err(unavailable());
        }
        context.validate().map_err(|_| unavailable())?;
    }
    Ok(approval.clone())
}

fn unavailable() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InvalidRunTransition,
        "The command approval is no longer current. Refresh the run.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unresponsive_authoritative_lookup_expires_without_retry() {
        let result = bounded_lookup(
            std::future::pending::<ApiResult<()>>(),
            std::time::Duration::from_millis(5),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            bounded_lookup(async { Ok(7) }, std::time::Duration::from_secs(1))
                .await
                .unwrap(),
            7
        );
    }

    #[test]
    fn authoritative_pending_identity_status_and_display_are_required() {
        let interaction = Interaction {
            interaction_id: "interaction".into(),
            run_id: "run".into(),
            kind: InteractionKind::Approval,
            status: InteractionStatus::Pending,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            respondable_by_caller: true,
            etag: "etag".into(),
            content: InteractionContent::Approval(ApprovalInteraction {
                reason: "An effect requires approval".into(),
                action: "process.execute".into(),
                resource: "configured executable".into(),
                risk: None,
                request_hash: "binding".into(),
                command_context: Some(colossus_sdk::CommandApprovalContext {
                    justification: "Check the build.".into(),
                    executable: "/bin/sh".into(),
                    arguments: vec!["-c".into(), "echo test".into()],
                    working_directory: "/work".into(),
                    redacted: false,
                }),
            }),
        };
        let request = RespondInteractionRequest {
            run_id: "run".into(),
            interaction_id: "interaction".into(),
            etag: "etag".into(),
            idempotency_key: colossus_sdk::IdempotencyKey::new("approval-test").unwrap(),
            response: InteractionAnswer::Approval {
                approved: true,
                request_hash: "binding".into(),
            },
        };
        assert!(validate(&interaction, &request).is_ok());
        for field in [
            "run",
            "interaction",
            "etag",
            "binding",
            "status",
            "scope",
            "context",
        ] {
            let mut changed = interaction.clone();
            match field {
                "run" => changed.run_id = "other-run".into(),
                "interaction" => changed.interaction_id = "other-interaction".into(),
                "etag" => changed.etag = "new-etag".into(),
                "status" => changed.status = InteractionStatus::Expired,
                "scope" => changed.respondable_by_caller = false,
                _ => {
                    let InteractionContent::Approval(approval) = &mut changed.content else {
                        unreachable!()
                    };
                    if field == "binding" {
                        approval.request_hash = "other-binding".into();
                    } else {
                        approval
                            .command_context
                            .as_mut()
                            .unwrap()
                            .arguments
                            .push("\u{202e}spoof".into());
                    }
                }
            }
            assert!(
                validate(&changed, &request).is_err(),
                "accepted changed {field}"
            );
        }
    }
}
