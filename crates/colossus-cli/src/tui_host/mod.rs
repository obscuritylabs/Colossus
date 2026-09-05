//! Embedded-runtime adapter for the backend-neutral Colossus TUI host contract.

use super::{ApprovalMode, TERMINAL_HISTORY_CAPACITY, doctor_profile, terminal_completion_values};
use async_trait::async_trait;
use colossus_contracts::{
    ActorType, AgentRunCancellation, AgentRunOutcome, ApprovalProof, ApprovalReviewNotice,
    AutomaticApprovalNotice, ContextStatus, ControlledAgentTerminal, EffectRequest, GoalRunOutcome,
    MemoryStatus, ModelContent, ModelContentPart, ModelImageReference, ModelMessageRole,
    PlanExecutionOutcome, PlanRecord, PlanStatus, PluginInventoryEntry, PolicyDecision,
    ProviderReadinessCheck, ProviderRoute, ReasoningEffort, ResearchDepth, ResearchSourceKind,
    RiskReviewFallbackNotice, RunEventEnvelope, SandboxBoundaryMode, SessionMessagePage,
    SessionSummary, TerminalPreferences, UserPromptRequest, UserPromptResponse, WorkStateSnapshot,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTable, PresentationTone, ThemeLibrary,
    ThemeName, automatic_approval_document, context_status_document, document_from_json,
    risk_review_fallback_document, work_state_document,
};
use colossus_runtime::{Runtime, RuntimeError, format_provider_response_diagnostic};
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostPlanExecutionOutcome,
    HostPlanExecutionResult, HostRunResult, InteractiveHost, InteractivePlanExecutionRequest,
    InteractivePrompt, InteractivePromptKind, InteractiveRunRequest, InteractiveSessionBrowser,
    InteractiveSessionBrowserEntry, InteractiveSessionBrowserMessage, InteractiveSnapshot,
    InteractiveThemePicker, InteractiveThemePickerEntry, PlanHostCommand, PlanSelectionUpdate,
    PromptResponse, RuntimeCommand, sandbox_boundary_acknowledgement_choice,
    sandbox_boundary_prompt,
};
use colossus_worker::{
    InteractiveWorkerRequest, SandboxBoundaryAcknowledgement, WorkerApprovalMode, WorkerClient,
    WorkerError, WorkerOperation, WorkerPrompt, WorkerPromptHandler, WorkerPromptKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const INTERACTIVE_PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const APPROVAL_CONTENT_PREVIEW_CHARACTERS: usize = 64 * 1024;

fn interactive_model_content(prompt: String, images: Vec<ModelImageReference>) -> ModelContent {
    if images.is_empty() {
        return ModelContent::Text(prompt);
    }
    let mut parts = Vec::with_capacity(images.len() + usize::from(!prompt.is_empty()));
    if !prompt.is_empty() {
        parts.push(ModelContentPart::Text { text: prompt });
    }
    parts.extend(
        images
            .into_iter()
            .map(|image| ModelContentPart::Image { image }),
    );
    ModelContent::Parts(parts)
}

fn approval_mode_document(mode: Option<ApprovalMode>, changed: bool) -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::Card {
        title: if changed {
            "Permissions updated".into()
        } else {
            "Permissions".into()
        },
        tone: if mode == Some(ApprovalMode::FullAccess) {
            PresentationTone::Warning
        } else {
            PresentationTone::Neutral
        },
        body: vec![
            PresentationBlock::KeyValue(vec![(
                "Approval mode".into(),
                mode.map(ApprovalMode::as_str)
                    .unwrap_or("worker-default")
                    .into(),
            )]),
            PresentationBlock::Markdown(
                "Applies to subsequent interactive agent and plan operations from this TUI. Approval mode does not turn policy denials into allows, add tool authority, or change sandbox boundaries.\n\nUsage: `/permissions [deny|ask|risk-auto|full-access]`"
                    .into(),
            ),
        ],
    })
}

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelDiagnostics {
    ready: bool,
    route: ProviderRoute,
    checks: Vec<ProviderReadinessCheck>,
}

fn grouped_u64(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

const fn reasoning_effort_label(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "none",
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::XHigh => "xhigh",
        ReasoningEffort::Max => "max",
        ReasoningEffort::Ultra => "ultra",
    }
}

fn check_status_label(status: &str) -> String {
    match status {
        "pass" => "Pass".into(),
        "fail" => "Fail".into(),
        "not_checked" => "Not checked".into(),
        "not_applicable" => "Not applicable".into(),
        value => value.replace('_', " "),
    }
}

fn model_diagnostics_document(value: &Value) -> Result<PresentationDocument, String> {
    let report: ModelDiagnostics =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let route = &report.route;
    let mut details = vec![
        (
            "Status".into(),
            if report.ready { "Ready" } else { "Not ready" }.into(),
        ),
        ("Model".into(), route.model.clone()),
        ("Profile".into(), route.model_profile.clone()),
        (
            "Provider".into(),
            format!("{} · {}", route.provider, route.provider_profile),
        ),
    ];
    if !route.role.is_empty() {
        details.push(("Role".into(), route.role.clone()));
    }
    details.extend([
        (
            "Reasoning".into(),
            route
                .reasoning_effort
                .map(reasoning_effort_label)
                .unwrap_or("provider default")
                .into(),
        ),
        (
            "Tokens".into(),
            format!(
                "{} context · {} input budget · {} max output",
                grouped_u64(route.limits.context_window_tokens),
                grouped_u64(route.limits.input_budget_tokens),
                grouped_u64(route.limits.max_output_tokens),
            ),
        ),
        (
            "Capabilities".into(),
            format!(
                "tools {} · streaming {} · images {}",
                if route.capabilities.tool_calls {
                    "on"
                } else {
                    "off"
                },
                if route.capabilities.streaming {
                    "on"
                } else {
                    "off"
                },
                if route.capabilities.image_inputs {
                    "on"
                } else {
                    "off"
                },
            ),
        ),
    ]);

    let mut checks = PresentationTable::new(
        ["Check", "Status", "Detail"],
        "No model checks were returned.",
    );
    for check in &report.checks {
        checks.push_row([
            check.name.clone(),
            check_status_label(&check.status),
            check.detail.clone(),
        ]);
    }

    let mut body = vec![
        PresentationBlock::KeyValue(details),
        PresentationBlock::Table(checks),
    ];
    for check in report.checks {
        if let Some(diagnostic) = check.provider_response {
            body.push(PresentationBlock::Card {
                title: format!("Provider response · {}", check.name),
                tone: PresentationTone::Error,
                body: vec![PresentationBlock::Code {
                    language: Some("text".into()),
                    content: format_provider_response_diagnostic(&diagnostic),
                }],
            });
        }
    }

    Ok(PresentationDocument::from_block(PresentationBlock::Card {
        title: "Model diagnostics".into(),
        tone: if report.ready {
            PresentationTone::Success
        } else {
            PresentationTone::Error
        },
        body,
    }))
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
mod plugins;
mod worker;

pub(crate) use common::{TuiApprovalProvider, TuiPromptRouter, TuiUserPromptProvider};
pub(crate) use embedded::EmbeddedInteractiveHost;
pub(crate) use worker::WorkerInteractiveHost;

use common::*;
use plugins::*;
use worker::parse_toggle;

#[cfg(test)]
mod tests;
