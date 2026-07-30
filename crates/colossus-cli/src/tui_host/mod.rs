//! Embedded-runtime adapter for the backend-neutral Colossus TUI host contract.

use super::{ApprovalMode, TERMINAL_HISTORY_CAPACITY, doctor_profile, terminal_completion_values};
use async_trait::async_trait;
use colossus_contracts::{
    AgentRunOutcome, ApprovalProof, ApprovalReviewNotice, AutomaticApprovalNotice, ContextStatus,
    ControlledAgentTerminal, EffectRequest, GoalRunOutcome, MemoryStatus, PlanExecutionOutcome,
    PlanRecord, PlanStatus, PolicyDecision, ProviderRoute, ResearchDepth, ResearchSourceKind,
    RiskReviewFallbackNotice, RunEventEnvelope, SessionMessagePage, SessionSummary,
    TerminalPreferences, UserPromptRequest, UserPromptResponse, WorkStateSnapshot,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTone, ThemeLibrary, ThemeName,
    automatic_approval_document, context_status_document, document_from_json,
    risk_review_fallback_document, work_state_document,
};
use colossus_runtime::{Runtime, RuntimeError, format_provider_response_diagnostic};
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostPlanExecutionOutcome,
    HostPlanExecutionResult, HostRunResult, InteractiveHost, InteractivePlanExecutionRequest,
    InteractivePrompt, InteractiveRunRequest, InteractiveSnapshot, PlanHostCommand,
    PlanSelectionUpdate, PromptResponse, RuntimeCommand,
};
use colossus_worker::{
    InteractiveWorkerRequest, WorkerClient, WorkerError, WorkerOperation, WorkerPrompt,
    WorkerPromptHandler, WorkerPromptKind,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const INTERACTIVE_PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

fn current_session_plan(
    plan: Option<PlanRecord>,
    plan_id: &str,
    session_id: &str,
) -> Result<PlanRecord, String> {
    let plan = plan.ok_or_else(|| format!("plan not found: {plan_id}"))?;
    if plan.session_id != session_id {
        return Err(format!(
            "plan {plan_id} does not belong to the active session"
        ));
    }
    Ok(plan)
}

fn selectable_plan(plan: PlanRecord) -> Result<PlanRecord, String> {
    if !matches!(plan.status, PlanStatus::Draft | PlanStatus::Approved) {
        return Err(format!(
            "plan {} is not selectable because it is {:?}",
            plan.id, plan.status
        ));
    }
    Ok(plan)
}

fn approved_plan_at_revision(
    plan: Option<PlanRecord>,
    plan_id: &str,
    session_id: &str,
    revision: u64,
) -> Result<PlanRecord, String> {
    let plan = current_session_plan(plan, plan_id, session_id)?;
    if plan.status != PlanStatus::Approved || plan.revision != revision {
        return Err(format!(
            "plan {plan_id} is no longer the selected approved revision {revision}; reload it with /plan use"
        ));
    }
    Ok(plan)
}

fn host_plan_execution_result(
    outcome: PlanExecutionOutcome,
    footer: FooterState,
) -> Result<HostPlanExecutionResult, String> {
    let value = serde_json::to_value(&outcome).map_err(|error| error.to_string())?;
    let document = document_from_json(&value, Some("Plan execution"));
    let (plan, outcome, plan_selection) = match outcome {
        PlanExecutionOutcome::CancelledBeforeStart { plan } => (
            plan.clone(),
            HostPlanExecutionOutcome::CancelledBeforeStart,
            PlanSelectionUpdate::Set(Box::new(plan)),
        ),
        PlanExecutionOutcome::Direct { plan, terminal } => {
            let outcome = match terminal {
                ControlledAgentTerminal::Completed { .. } => HostPlanExecutionOutcome::Completed,
                ControlledAgentTerminal::Cancelled { .. } => {
                    HostPlanExecutionOutcome::CancelledAfterConsumption
                }
                ControlledAgentTerminal::Failed { message, .. } => {
                    HostPlanExecutionOutcome::FailedAfterConsumption(message)
                }
            };
            (plan, outcome, PlanSelectionUpdate::Clear)
        }
        PlanExecutionOutcome::Goal { plan, terminal } => {
            let outcome = match terminal {
                GoalRunOutcome::Completed { .. } => HostPlanExecutionOutcome::Completed,
                GoalRunOutcome::Cancelled { .. } => {
                    HostPlanExecutionOutcome::CancelledAfterConsumption
                }
                GoalRunOutcome::Failed { message, .. } => {
                    HostPlanExecutionOutcome::FailedAfterConsumption(message)
                }
            };
            (plan, outcome, PlanSelectionUpdate::Clear)
        }
    };
    Ok(HostPlanExecutionResult {
        plan,
        document,
        outcome,
        footer,
        plan_selection,
    })
}

fn host_plan_execution_failure(
    selected: PlanRecord,
    readback: Result<Option<PlanRecord>, String>,
    error: String,
    footer: FooterState,
) -> HostPlanExecutionResult {
    let expected_consumed_revision = selected.revision.saturating_add(1);
    let (plan, outcome, plan_selection, title, explanation) = match readback {
        Ok(Some(plan))
            if plan.id == selected.id
                && plan.session_id == selected.session_id
                && plan.status == PlanStatus::Approved
                && plan.revision == selected.revision =>
        {
            (
                plan.clone(),
                HostPlanExecutionOutcome::FailedBeforeConsumption(error.clone()),
                PlanSelectionUpdate::Set(Box::new(plan)),
                "Plan execution did not start",
                format!("{error}\n\nThe approved plan was not consumed."),
            )
        }
        Ok(Some(plan))
            if plan.id == selected.id
                && plan.session_id == selected.session_id
                && plan.status == PlanStatus::Executed
                && plan.revision == expected_consumed_revision =>
        {
            (
                plan,
                HostPlanExecutionOutcome::ConsumedOutcomeUnknown(error.clone()),
                PlanSelectionUpdate::Clear,
                "Plan consumed; execution outcome unknown",
                format!(
                    "{error}\n\nInspect /plans and linked run or Goal evidence before retrying."
                ),
            )
        }
        _ => (
            selected,
            HostPlanExecutionOutcome::OutcomeUnknown(error.clone()),
            PlanSelectionUpdate::Clear,
            "Plan execution outcome unknown",
            format!("{error}\n\nInspect /plans before retrying this operation."),
        ),
    };
    HostPlanExecutionResult {
        plan,
        document: PresentationDocument::from_block(PresentationBlock::Card {
            title: title.into(),
            tone: PresentationTone::Error,
            body: vec![PresentationBlock::Text(explanation)],
        }),
        outcome,
        footer,
        plan_selection,
    }
}

fn host_plan_lifecycle_failure(
    selected: PlanRecord,
    readback: Result<Option<PlanRecord>, String>,
    committed_status: PlanStatus,
    error: String,
) -> HostCommandResult {
    let expected_committed_revision = selected.revision.saturating_add(1);
    let (plan_selection, title, explanation) = match readback {
        Ok(Some(plan))
            if plan.id == selected.id
                && plan.session_id == selected.session_id
                && plan.status == selected.status
                && plan.revision == selected.revision =>
        {
            (
                PlanSelectionUpdate::Set(Box::new(plan)),
                "Plan lifecycle operation did not commit",
                format!("{error}\n\nThe selected plan revision is unchanged."),
            )
        }
        Ok(Some(plan))
            if plan.id == selected.id
                && plan.session_id == selected.session_id
                && plan.status == committed_status
                && plan.revision == expected_committed_revision =>
        {
            let selection = if committed_status == PlanStatus::Approved {
                PlanSelectionUpdate::Set(Box::new(plan))
            } else {
                PlanSelectionUpdate::Clear
            };
            (
                selection,
                "Plan lifecycle response interrupted",
                format!(
                    "{error}\n\nThe {:?} transition is durable despite the interrupted response.",
                    committed_status
                ),
            )
        }
        _ => (
            PlanSelectionUpdate::Clear,
            "Plan lifecycle outcome unknown",
            format!("{error}\n\nSelection was cleared. Inspect /plans before retrying."),
        ),
    };
    let mut result =
        HostCommandResult::document(PresentationDocument::from_block(PresentationBlock::Card {
            title: title.into(),
            tone: PresentationTone::Error,
            body: vec![PresentationBlock::Text(explanation)],
        }));
    result.plan_selection = plan_selection;
    result.continue_queue = false;
    result
}

fn append_footer_warning(result: &mut HostPlanExecutionResult, error: String) {
    result.document.blocks.push(PresentationBlock::Card {
        title: "Footer refresh failed".into(),
        tone: PresentationTone::Warning,
        body: vec![PresentationBlock::Text(error)],
    });
}

fn interactive_runtime_error(error: &RuntimeError) -> String {
    match error.provider_response_diagnostic() {
        Some(diagnostic) => format!(
            "{error}\n\n{}",
            format_provider_response_diagnostic(diagnostic)
        ),
        None => error.to_string(),
    }
}

mod common;
mod embedded;
mod worker;

pub(crate) use common::{TuiApprovalProvider, TuiPromptRouter, TuiUserPromptProvider};
pub(crate) use embedded::EmbeddedInteractiveHost;
pub(crate) use worker::WorkerInteractiveHost;

use common::*;
use worker::parse_toggle;

#[cfg(test)]
mod tests;
