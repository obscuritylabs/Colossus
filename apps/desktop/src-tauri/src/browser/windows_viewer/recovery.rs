//! Retire a failed presentation without discarding its inspectable tab identity.
use super::BrowserManager;
use crate::{browser::manager::error, dto::CommandErrorDto};
use std::future::Future;

impl BrowserManager {
    pub(in crate::browser) async fn recover_agent_presentation<T>(
        &self,
        scope: &str,
        id: &str,
        admission: impl Future<Output = Result<Option<T>, CommandErrorDto>>,
    ) -> Result<Option<T>, CommandErrorDto> {
        let (view, _) = self.tab(id, scope)?;
        let _ = view.hide();
        view.close()
            .await
            .map_err(|_| error("The agent browser presentation is still closing."))?;
        // A missing or rejected replacement is never authority to delete this
        // tab. Terminal reconciliation still needs its session and last page.
        admission.await
    }
}

#[cfg(test)]
mod tests;
