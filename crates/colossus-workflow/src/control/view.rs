//! Display projections: structural logic and recorded states, never raw effect payloads.
use super::*;
use colossus_contracts::{
    WorkflowLogic, WorkflowLogicBranch, WorkflowLogicKind as Kind, WorkflowLogicStep,
    WorkflowStepState, WorkflowStepStatus as Status,
};

pub(super) fn logic(definition: &WorkflowDefinition) -> Option<WorkflowLogic> {
    fn steps(
        source: &[WorkflowStep],
        depth: usize,
        count: &mut usize,
    ) -> Option<Vec<WorkflowLogicStep>> {
        if depth > 7 {
            return None;
        }
        source
            .iter()
            .map(|step| {
                *count += 1;
                if *count > 512 {
                    return None;
                }
                let mut branches = Vec::new();
                let (kind, summary) = match step {
                    WorkflowStep::Agent { .. } => (Kind::Agent, "Model task".into()),
                    WorkflowStep::Tool { tool, .. } => (Kind::Tool, tool.clone()),
                    WorkflowStep::Workflow {
                        workflow, version, ..
                    } => (Kind::Workflow, format!("{workflow}:{version}")),
                    WorkflowStep::Approval { .. } => {
                        (Kind::Approval, "Operator approval required".into())
                    }
                    WorkflowStep::WaitForInput { .. } => (
                        Kind::WaitForInput,
                        "Structured operator input required".into(),
                    ),
                    WorkflowStep::Emit { .. } => (Kind::Emit, "Emit a workflow value".into()),
                    WorkflowStep::Condition {
                        expression,
                        then,
                        otherwise,
                        ..
                    } => {
                        branches.push(WorkflowLogicBranch {
                            label: "True".into(),
                            steps: steps(then, depth + 1, count)?,
                        });
                        branches.push(WorkflowLogicBranch {
                            label: "False".into(),
                            steps: steps(otherwise, depth + 1, count)?,
                        });
                        (Kind::Condition, expression.clone())
                    }
                    WorkflowStep::Parallel {
                        branches: source,
                        max_concurrency,
                        ..
                    } => {
                        for (index, branch) in source.iter().enumerate() {
                            branches.push(WorkflowLogicBranch {
                                label: format!("Branch {}", index + 1),
                                steps: steps(branch, depth + 1, count)?,
                            });
                        }
                        (
                            Kind::Parallel,
                            format!(
                                "{} branches · at most {max_concurrency} concurrent",
                                source.len()
                            ),
                        )
                    }
                    WorkflowStep::Foreach {
                        items,
                        max_items,
                        steps: body,
                        ..
                    } => {
                        branches.push(WorkflowLogicBranch {
                            label: "Each item".into(),
                            steps: steps(body, depth + 1, count)?,
                        });
                        (
                            Kind::Foreach,
                            format!("{items} · at most {max_items} items"),
                        )
                    }
                };
                Some(WorkflowLogicStep {
                    id: step_id(step).into(),
                    kind,
                    summary,
                    branches,
                })
            })
            .collect()
    }
    let mut count = 0;
    let projection = WorkflowLogic {
        steps: steps(&definition.steps, 0, &mut count)?,
        compensation: steps(&definition.compensation, 0, &mut count)?,
    };
    projection.within_bounds().then_some(projection)
}

pub(super) fn step_states(
    journal: &dyn EventJournal,
    events: &[EventEnvelope],
    run_status: WorkflowStatus,
) -> Result<Vec<WorkflowStepState>, WorkflowError> {
    let mut executions: BTreeMap<String, BTreeMap<String, Status>> = BTreeMap::new();
    for event in events {
        let state = match event.event_type.as_str() {
            "workflow.step.started.v1" | "workflow.compensation.step.started.v1" => Status::Running,
            "workflow.step.completed.v1" | "workflow.compensation.step.completed.v1" => {
                Status::Completed
            }
            "workflow.run.waiting.v1" => Status::Waiting,
            "workflow.run.failed.v1" | "workflow.compensation.step.failed.v1" => Status::Failed,
            "workflow.step.outcome_unknown.v1" | "workflow.run.interrupted.v1" => {
                Status::Interrupted
            }
            _ => continue,
        };
        let payload = journal.decrypt_payload(event)?;
        let Some(id) = payload.get("step_id").and_then(Value::as_str) else {
            continue;
        };
        if !valid_step_id(id) {
            return Err(WorkflowError::InvalidTransition(
                "invalid recorded workflow step identity".into(),
            ));
        }
        if id.len() > 128 {
            return Ok(Vec::new());
        }
        let execution = payload
            .get("execution_id")
            .and_then(Value::as_str)
            .unwrap_or(id);
        executions
            .entry(id.into())
            .or_default()
            .insert(execution.into(), state);
        if executions.len() > 512 {
            return Ok(Vec::new());
        }
    }
    Ok(executions
        .into_iter()
        .map(|(step_id, executions)| {
            let completed_executions = executions
                .values()
                .filter(|state| **state == Status::Completed)
                .count();
            let completed_executions = u32::try_from(completed_executions).unwrap_or(u32::MAX);
            let mut status = [
                Status::Waiting,
                Status::Running,
                Status::Interrupted,
                Status::Failed,
                Status::Completed,
            ]
            .into_iter()
            .find(|state| executions.values().any(|value| value == state))
            .unwrap_or(Status::Interrupted);
            if matches!(status, Status::Running | Status::Waiting) {
                status = match run_status {
                    WorkflowStatus::Cancelled => Status::Cancelled,
                    WorkflowStatus::Completed
                    | WorkflowStatus::Failed
                    | WorkflowStatus::Interrupted => Status::Interrupted,
                    _ => status,
                };
            }
            WorkflowStepState {
                step_id,
                status,
                completed_executions,
            }
        })
        .collect())
}
