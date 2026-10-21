//! Retire a failed presentation without discarding its inspectable tab identity.
use super::BrowserManager;
use crate::{
    browser::{manager::error, registry::MAX_TABS},
    dto::CommandErrorDto,
};
use std::future::Future;

impl BrowserManager {
    pub(in crate::browser) fn reserve_agent_recovery(
        &self,
        run_id: &str,
    ) -> Result<bool, CommandErrorDto> {
        let mut registry = self.lock()?;
        if registry.viewer_recoveries.len() >= MAX_TABS
            && !registry.viewer_recoveries.contains_key(run_id)
        {
            return Ok(false);
        }
        let attempts = registry.viewer_recoveries.entry(run_id.into()).or_default();
        if *attempts >= 2 {
            return Ok(false);
        }
        // Charge before awaiting native detach/admission. Errors, cancellation,
        // and an actual replacement keep this bounded recovery reservation.
        *attempts += 1;
        Ok(true)
    }

    pub(in crate::browser) async fn recover_agent_presentation<T>(
        &self,
        scope: &str,
        run_id: &str,
        id: &str,
        admission: impl Future<Output = Result<Option<T>, CommandErrorDto>>,
    ) -> Result<Option<T>, CommandErrorDto> {
        let (view, _) = self.tab(id, scope)?;
        let _ = view.hide();
        // Recovery can race the final canonical update. Preserve the already
        // authenticated pixels privately before detach clears the live cache.
        // Only terminal reconciliation may make this hidden copy inspectable.
        let pending = self
            .lock()?
            .tabs
            .iter()
            .find(|tab| tab.scope == scope && tab.dto.id == id)
            .and_then(|tab| tab.pending_completed_view.clone());
        if pending.is_none()
            && let Some(snapshot) = view.completed_snapshot().await.ok().flatten()
            && let Some(tab) = self
                .lock()?
                .tabs
                .iter_mut()
                .find(|tab| tab.scope == scope && tab.dto.id == id)
        {
            tab.pending_completed_view = Some(snapshot);
        }
        view.close()
            .await
            .map_err(|_| error("The agent browser presentation is still closing."))?;
        // A missing or rejected replacement is never authority to delete this
        // tab. Terminal reconciliation still needs its session and last page.
        let admitted = admission.await?;
        if admitted.is_none()
            && let Some(attempts) = self.lock()?.viewer_recoveries.get_mut(run_id)
        {
            // A closed session can precede the next eligible page by several
            // model turns. A positive missing-page reply created no replacement;
            // it must not exhaust recovery before that page becomes available.
            *attempts = attempts.saturating_sub(1);
        }
        Ok(admitted)
    }
}

#[cfg(test)]
mod tests;
