//! Resolve public Plan references through the exact personal cloud conversation.
use super::*;
use colossus_sdk::{PlanExecutionStrategy, PlanRunAction, PlanStatus, RunMode, RunTerminal};

impl CloudRepository {
    pub(super) async fn require_personal_plan(
        &self,
        caller: &CloudCaller,
        node: &CloudNode,
        conversation: Option<&(crate::CloudThread, u64)>,
        request: &CreateRunRequest,
    ) -> CloudResult<()> {
        caller.require(CloudPermission::Control)?;
        let thread = conversation
            .map(|(thread, _)| thread)
            .ok_or(CloudError::InvalidArgument)?;
        if node.owner_user_id.as_deref() != Some(caller.subject())
            || thread.owner_user_id.as_deref() != Some(caller.subject())
            || thread.node_id != node.node_id
            || thread.project_id != node.project_id
        {
            return Err(CloudError::PermissionDenied);
        }
        let action = request
            .plan_action
            .as_ref()
            .ok_or(CloudError::InvalidArgument)?;
        let (run_id, revision) = match action {
            PlanRunAction::Revise {
                source_run_id,
                expected_revision,
            } if request.mode == RunMode::Plan => (source_run_id, *expected_revision),
            PlanRunAction::Execute {
                source_run_id,
                expected_revision,
                strategy,
            } if request.mode == RunMode::Execute
                && match strategy {
                    PlanExecutionStrategy::Direct => true,
                    PlanExecutionStrategy::Goal { max_iterations } => {
                        (1..=50).contains(max_iterations)
                    }
                } =>
            {
                (source_run_id, *expected_revision)
            }
            _ => return Err(CloudError::InvalidArgument),
        };
        validate_identifier(run_id)?;
        if revision == 0 {
            return Err(CloudError::InvalidArgument);
        }
        let session = thread.session_id.as_ref().ok_or(CloudError::Conflict)?;
        if request.session_id.as_ref() != Some(session) {
            return Err(CloudError::InvalidArgument);
        }
        // Both keys were written atomically with the original Create receipt.
        // Caller-supplied runtime identifiers never establish their own mapping.
        let (source_id, _) = self
            .read::<String>(&format!(
                "cloud.run:{}:{}:{run_id}",
                node.project_id, node.node_id,
            ))
            .await?;
        let source = self.task_for_caller(caller, &source_id).await?;
        let (mapped_thread, _) = self
            .read::<String>(&format!(
                "cloud.session:{}:{}:{session}",
                node.project_id, node.node_id,
            ))
            .await?;
        if mapped_thread != thread.thread_id
            || source.node_id != node.node_id
            || source.thread_id.as_deref() != Some(thread.thread_id.as_str())
            || source.subject != caller.subject()
            || source.source_read_only
            || source.run_id.as_deref() != Some(run_id.as_str())
        {
            return Err(CloudError::PermissionDenied);
        }
        let released = source.snapshot.as_ref().ok_or(CloudError::Conflict)?;
        if released.run.run_id != *run_id || released.run.session_id != *session {
            return Err(CloudError::PermissionDenied);
        }
        let (plan_id, plan_revision, plan_status) = match &released.run.terminal {
            Some(RunTerminal::Result(result)) => {
                (&result.plan_id, result.plan_revision, result.plan_status)
            }
            Some(RunTerminal::Cancellation(result)) => {
                (&result.plan_id, result.plan_revision, result.plan_status)
            }
            _ => return Err(CloudError::Conflict),
        };
        if plan_id.as_ref().is_none_or(|id| id.is_empty())
            || plan_revision != Some(revision)
            || plan_status != Some(PlanStatus::Draft)
        {
            return Err(CloudError::Conflict);
        }
        // The public SDK independently verifies canonical Plan identity, Draft
        // lifecycle, revision and session immediately before mutation/consumption.
        Ok(())
    }
}
