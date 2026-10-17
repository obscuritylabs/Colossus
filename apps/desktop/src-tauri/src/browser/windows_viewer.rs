//! Reconcile Windows viewers through the canonical run and acknowledged native detach.
use super::{
    manager::BrowserManager,
    registry::{CRASHED_MESSAGE, MAX_TABS},
};
use crate::{dto::CommandErrorDto, state::AppState};
use colossus_sdk::{GetRunRequest, RunStatus};
use std::time::Duration;
use tauri::{AppHandle, Manager as _};

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
    ) -> Result<bool, CommandErrorDto> {
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
                )
            });
        let Some((id, crashed)) = existing else {
            return Ok(true);
        };
        let state = app.state::<AppState>();
        let Some(target) = state.target(scope).await else {
            return Ok(true);
        };
        let Some(_slot) = target.try_unary_slot() else {
            return Ok(true);
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
            return Ok(true);
        };
        if response.run.session_id != conversation_id || response.run.run_id != run_id {
            return Ok(true);
        }
        if terminal(response.run.status) {
            // Ending a viewer is distinct from claiming browser cleanup. Detach
            // must acknowledge either an active writer's hidden view or Core's
            // completed cleanup before any Desktop tab metadata is removed.
            self.close(scope, &id).await?;
            self.lock()?.viewer_recoveries.remove(run_id);
            return Ok(false);
        }
        if !crashed {
            return Ok(true);
        }
        {
            let mut registry = self.lock()?;
            if registry.viewer_recoveries.len() >= MAX_TABS
                && !registry.viewer_recoveries.contains_key(run_id)
            {
                return Ok(true);
            }
            let attempts = registry.viewer_recoveries.entry(run_id.into()).or_default();
            if *attempts >= 2 {
                return Ok(true);
            }
            *attempts += 1;
        }
        // Recover only presentation. The following normal watch obtains a fresh
        // authenticated read-only admission to the existing Core-owned page.
        self.close(scope, &id).await?;
        Ok(true)
    }
}
