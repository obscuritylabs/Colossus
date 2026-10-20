//! Reconcile Windows viewers through the canonical run and acknowledged native detach.
use super::{manager::BrowserManager, registry::CRASHED_MESSAGE};
use crate::{dto::CommandErrorDto, state::AppState};
use colossus_sdk::{GetRunRequest, RunStatus};
use std::time::Duration;
use tauri::{AppHandle, Manager as _};

mod close_intent;
mod completed;
mod recovery;

pub(super) enum AgentViewerPlan {
    Keep,
    Attach,
    Replace(String),
}

fn terminal(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Completed
            | RunStatus::Failed
            | RunStatus::Cancelled
            | RunStatus::Interrupted
            | RunStatus::OutcomeUnknown
    )
}

impl BrowserManager {
    pub(super) async fn reconcile_windows_agent(
        &self,
        app: &AppHandle,
        scope: &str,
        run_id: &str,
        conversation_id: &str,
    ) -> Result<AgentViewerPlan, CommandErrorDto> {
        // Only the selected conversation's current run can own this viewer.
        // Keep its bounded recovery budget across fresh attachments, without
        // accumulating counters for already replaced runs or workspaces.
        self.lock()?.viewer_recoveries.retain(|id, _| id == run_id);
        let existing = self
            .lock()?
            .tabs
            .iter()
            .find(|tab| {
                tab.scope == scope
                    && tab.agent_run_id.as_deref() == Some(run_id)
                    && tab.dto.conversation_id.as_deref() == Some(conversation_id)
            })
            .map(|tab| {
                (
                    tab.dto.id.clone(),
                    tab.dto.error.as_deref() == Some(CRASHED_MESSAGE),
                    matches!(tab.dto.control, "completed" | "expired"),
                    tab.dto.session_id.clone(),
                    tab.close_requested,
                )
            });
        let Some((id, crashed, completed, session_id, close_requested)) = existing else {
            return Ok(AgentViewerPlan::Attach);
        };
        if close_requested {
            self.close(scope, &id).await?;
            return Ok(AgentViewerPlan::Keep);
        }
        if completed {
            return Ok(AgentViewerPlan::Keep);
        }
        let state = app.state::<AppState>();
        let Some(target) = state.target(scope).await else {
            return Ok(AgentViewerPlan::Keep);
        };
        let Some(_slot) = target.try_unary_slot() else {
            return Ok(AgentViewerPlan::Keep);
        };
        let Ok(Ok(response)) = tokio::time::timeout(
            Duration::from_secs(1),
            target.client.get_run(GetRunRequest {
                run_id: run_id.into(),
            }),
        )
        .await
        else {
            // Unavailable run metadata never becomes terminal/cleanup authority.
            return Ok(AgentViewerPlan::Keep);
        };
        if response.run.session_id != conversation_id || response.run.run_id != run_id {
            return Ok(AgentViewerPlan::Keep);
        }
        if terminal(response.run.status) {
            let closed =
                close_intent::explicitly_closed(&target.client, &response.run, &session_id).await;
            self.settle_terminal_agent(scope, &id, closed).await?;
            self.lock()?.viewer_recoveries.remove(run_id);
            return Ok(AgentViewerPlan::Keep);
        }
        if !crashed {
            return Ok(AgentViewerPlan::Keep);
        }
        if !self.reserve_agent_recovery(run_id)? {
            return Ok(AgentViewerPlan::Keep);
        }
        // Recovery retires only the failed presentation. Keep its tab and last
        // metadata until a fresh admission succeeds: Core can finish while the
        // next watch is awaiting its reply, leaving no live page to reattach.
        Ok(AgentViewerPlan::Replace(id))
    }

    async fn settle_terminal_agent(
        &self,
        scope: &str,
        id: &str,
        explicitly_closed: Option<bool>,
    ) -> Result<(), CommandErrorDto> {
        match explicitly_closed {
            Some(true) => self.close(scope, id).await,
            closed => {
                // Unknown/over-budget close evidence retains only the URL
                // already displayed by Desktop, with a fresh-session path.
                // It cannot authorize a new private pixel copy.
                self.retain_completed_agent(scope, id, closed == Some(false))
                    .await
            }
        }
    }
}
