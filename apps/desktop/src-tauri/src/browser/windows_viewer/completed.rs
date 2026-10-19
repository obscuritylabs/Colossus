//! Retain inspectable pixels only after canonical run completion and native detach.
use super::BrowserManager;
use crate::{browser::manager::error, dto::CommandErrorDto};
use colossus_native_browser::BrowserView;
use std::sync::atomic::Ordering;

impl BrowserManager {
    pub(super) async fn retain_completed_agent(
        &self,
        scope: &str,
        id: &str,
        retain_pixels: bool,
    ) -> Result<(), CommandErrorDto> {
        let (view, _) = self.tab(id, scope)?;
        // This is a hidden static copy of already authenticated, presented
        // pixels. It has no Core lease, browser process, network or input path.
        // A missing frame must not prevent acknowledged browser cleanup.
        let pending = self
            .lock()?
            .tabs
            .iter()
            .find(|tab| tab.scope == scope && tab.dto.id == id)
            .and_then(|tab| tab.pending_completed_view.clone());
        let snapshot = match pending {
            // A prior complete canonical replay already admitted this hidden
            // copy. Retrying its detach needs no new pixel extraction.
            Some(snapshot) => Some(snapshot),
            None if retain_pixels => view.completed_snapshot().await.ok().flatten(),
            None => None,
        };
        // Native detach can need a retry. Keep the hidden copy with its tab so
        // a missing acknowledgment cannot consume the only accepted frame.
        if let Some(tab) = self
            .lock()?
            .tabs
            .iter_mut()
            .find(|tab| tab.scope == scope && tab.dto.id == id)
        {
            tab.pending_completed_view.clone_from(&snapshot);
        }
        let page = snapshot.as_ref().and_then(BrowserView::completed_page);
        view.close().await.map_err(|_| {
            error("The agent browser is still closing. Its last view is not ready.")
        })?;
        let mut registry = self.lock()?;
        let Some(tab) = registry
            .tabs
            .iter_mut()
            .find(|tab| tab.scope == scope && tab.dto.id == id)
        else {
            return Ok(());
        };
        tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
        tab.heartbeat = None;
        tab.dto.completed_snapshot = snapshot.is_some();
        tab.view = snapshot.unwrap_or(BrowserView::Empty);
        tab.pending_completed_view = None;
        if let Some(page) = page {
            tab.dto.page = page;
        }
        tab.dto.control = "completed";
        tab.dto.page.loading = false;
        tab.dto.page.can_go_back = false;
        tab.dto.page.can_go_forward = false;
        tab.dto.error = None;
        tab.dto.notice = None;
        tab.dto.popup_url = None;
        Ok(())
    }
}
