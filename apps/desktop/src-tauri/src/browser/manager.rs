use colossus_native_browser::{
    self as engine, BrowserEvent, BrowserView, EventSink, NavigationAction, NavigationPolicy,
    PageState,
};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering},
};
use tauri::{AppHandle, Manager as _, Webview};

use super::{
    dto::{BrowserAction, BrowserSnapshotDto, BrowserTabDto},
    inspection::{SNAPSHOT_BUDGET, collect_pages},
    registry::{DRAINING_MESSAGE, MAX_TABS, Registry, Tab},
};
use crate::{desktop_settings::SettingsStore, dto::CommandErrorDto};

#[derive(Default)]
pub(crate) struct BrowserManager {
    pub(super) data: Arc<Mutex<Registry>>,
    pub(crate) operation: tokio::sync::Mutex<()>,
}

pub(super) fn error(message: &str) -> CommandErrorDto {
    CommandErrorDto::local_sanitized("browser_unavailable", message, true)
}
fn engine_error(value: engine::BrowserError) -> CommandErrorDto {
    error(&value.to_string())
}

fn teardown_hide_result(result: Result<(), engine::BrowserError>) -> Result<(), CommandErrorDto> {
    match result {
        // A timed-out close may have completed after the caller retained its tab.
        // Still require close() below to reconcile actual native ownership.
        Ok(()) | Err(engine::BrowserError::Closed) => Ok(()),
        Err(error) => Err(engine_error(error)),
    }
}

impl BrowserManager {
    pub(super) fn require_open(&self) -> Result<(), CommandErrorDto> {
        if self.lock()?.draining {
            return Err(error(DRAINING_MESSAGE));
        }
        Ok(())
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Registry>, CommandErrorDto> {
        self.data
            .lock()
            .map_err(|_| error("The browser session is unavailable."))
    }

    pub(crate) fn selection_changed(&self, scope: Option<String>) {
        let views = if let Ok(mut state) = self.data.lock()
            && state.scope != scope
        {
            for tab in &mut state.tabs {
                tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
                tab.heartbeat = None;
            }
            state.generation = state.generation.wrapping_add(1);
            state.scope = scope;
            state
                .tabs
                .iter()
                .map(|tab| tab.view.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for view in views {
            let _ = view.hide();
        }
    }

    pub(crate) fn hide_all(&self) {
        let views = if let Ok(mut state) = self.data.lock() {
            for tab in &mut state.tabs {
                tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
                tab.heartbeat = None;
            }
            state
                .tabs
                .iter()
                .map(|tab| tab.view.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for view in views {
            let _ = view.hide();
        }
    }

    pub(crate) fn controller_loading(&self) {
        let views = if let Ok(mut state) = self.data.lock() {
            state.generation = state.generation.wrapping_add(1);
            for tab in &mut state.tabs {
                tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
                tab.heartbeat = None;
            }
            state
                .tabs
                .iter()
                .map(|tab| tab.view.clone())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for view in views {
            let _ = view.hide();
        }
    }

    pub(crate) fn validate(&self, generation: u64) -> Result<String, CommandErrorDto> {
        let state = self.lock()?;
        if state.draining {
            return Err(error(DRAINING_MESSAGE));
        }
        if !state.snapshot().available {
            return Err(error(
                state
                    .snapshot()
                    .engine
                    .message
                    .as_deref()
                    .unwrap_or("Browser preview is not enabled in this build."),
            ));
        }
        if state.generation != generation {
            return Err(error(
                "The selected workspace changed. Reopen the browser pane.",
            ));
        }
        state
            .scope
            .clone()
            .ok_or_else(|| error("Select a workspace before opening the browser."))
    }

    pub(super) fn tab(
        &self,
        id: &str,
        scope: &str,
    ) -> Result<(BrowserView, NavigationPolicy), CommandErrorDto> {
        self.lock()?
            .tabs
            .iter()
            .find(|t| t.dto.id == id && t.scope == scope)
            .map(|t| (t.view.clone(), t.policy.clone()))
            .ok_or_else(|| error("This tab is no longer available in the selected workspace."))
    }

    #[cfg(all(
        feature = "browser-test-bridge",
        not(feature = "embedded-chromium-preview")
    ))]
    pub(super) fn legacy_tab(
        &self,
        id: &str,
        scope: &str,
    ) -> Result<(Webview, NavigationPolicy), CommandErrorDto> {
        let (view, policy) = self.tab(id, scope)?;
        let view = view
            .system_view()
            .cloned()
            .ok_or_else(|| error("Use the Chromium native acceptance harness for this build."))?;
        Ok((view, policy))
    }

    /// Revoke controller generations, hide, then settle native teardown before
    /// temporary profiles are dropped. Failed cleanup keeps profile ownership.
    #[cfg(any(test, feature = "browser-test-bridge"))]
    pub(crate) async fn close_all_settled(&self) -> Result<u64, CommandErrorDto> {
        let operation = self.operation.lock().await;
        self.close_all_locked(&operation, None).await
    }

    /// Stop admitting work before teardown, without retaining the operation lock
    /// through the application's later runtime/client cleanup. Trusted Quit retries
    /// may reconcile delayed close acknowledgements while ordinary controls stay revoked.
    pub(crate) async fn drain_for_shutdown(&self) -> Result<u64, CommandErrorDto> {
        let operation = self.operation.lock().await;
        self.lock()?.draining = true;
        self.close_all_locked(&operation, None).await
    }

    #[cfg(any(test, feature = "embedded-chromium-preview"))]
    pub(super) async fn close_all_authorized<'a>(
        &'a self,
        expected_generation: u64,
        expected_scope: &str,
    ) -> Result<(u64, tokio::sync::MutexGuard<'a, ()>), CommandErrorDto> {
        let operation = self.operation.lock().await;
        let generation = self
            .close_all_locked(&operation, Some((expected_generation, expected_scope)))
            .await?;
        // The native store caller retains this guard through its final
        // lifecycle check and mutation, excluding creation of a new guest.
        Ok((generation, operation))
    }

    async fn close_all_locked(
        &self,
        _operation: &tokio::sync::MutexGuard<'_, ()>,
        expected: Option<(u64, &str)>,
    ) -> Result<u64, CommandErrorDto> {
        let (generation, tabs, views) = {
            let mut state = self.lock()?;
            if expected.is_some() && state.draining {
                return Err(error(DRAINING_MESSAGE));
            }
            if let Some((generation, scope)) = expected
                && (state.generation != generation || state.scope.as_deref() != Some(scope))
            {
                return Err(error(
                    "The selected workspace changed before browser teardown.",
                ));
            }
            state.generation = state.generation.wrapping_add(1);
            for tab in &mut state.tabs {
                tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
                tab.heartbeat = None;
            }
            (
                state.generation,
                state
                    .tabs
                    .iter()
                    .map(|tab| (tab.scope.clone(), tab.dto.id.clone()))
                    .collect::<Vec<_>>(),
                state
                    .tabs
                    .iter()
                    .map(|tab| tab.view.clone())
                    .collect::<Vec<_>>(),
            )
        };
        for view in views {
            teardown_hide_result(view.hide())?;
        }
        for (scope, id) in tabs {
            self.close(&scope, &id).await?;
        }
        if !self.lock()?.tabs.is_empty() {
            return Err(error("Native browser sessions did not finish closing."));
        }
        Ok(generation)
    }

    pub(crate) async fn snapshot(&self) -> Result<BrowserSnapshotDto, CommandErrorDto> {
        let views: Vec<_> = {
            let state = self.lock()?;
            if state.draining {
                return Ok(state.snapshot());
            }
            state
                .tabs
                .iter()
                .filter(|t| Some(&t.scope) == state.scope.as_ref())
                .map(|t| (t.dto.id.clone(), t.view.clone()))
                .collect()
        };
        let inspections = views
            .into_iter()
            .map(|(id, view)| async move { (id, view.inspect().await) });
        for (id, mut page) in collect_pages(inspections, SNAPSHOT_BUDGET).await {
            let mut state = self.lock()?;
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.dto.id == id) {
                #[cfg(windows)]
                {
                    page.loading = tab.dto.page.loading;
                }
                if page.url == "about:blank" {
                    if page.loading {
                        page.url.clone_from(&tab.dto.page.url);
                    } else {
                        page.url.clear();
                    }
                }
                tab.dto.page = page;
            }
        }
        Ok(self.lock()?.snapshot())
    }

    pub(crate) async fn apply(
        &self,
        app: &AppHandle,
        generation: u64,
        action: BrowserAction,
    ) -> Result<(), CommandErrorDto> {
        let scope = self.validate(generation)?;
        match action {
            BrowserAction::New { url } => self.create(app, &scope, &url).await?,
            BrowserAction::Clear => {
                let ids: Vec<_> = self
                    .lock()?
                    .tabs
                    .iter()
                    .filter(|t| t.scope == scope)
                    .map(|t| t.dto.id.clone())
                    .collect();
                for id in ids {
                    self.close(&scope, &id).await?;
                }
            }
            BrowserAction::Navigate { tab_id, url } => {
                let (view, policy) = self.tab(&tab_id, &scope)?;
                let url = engine::parse_address(&url).map_err(engine_error)?;
                policy.authorize(&url).map_err(engine_error)?;
                self.reset_navigation(&tab_id, Some(url.as_str()))?;
                view.navigate(url)
                    .map_err(|_| error("This page could not be opened."))?;
            }
            BrowserAction::Select { tab_id } => {
                self.tab(&tab_id, &scope)?;
                self.hide_all();
                self.lock()?.selected.insert(scope, tab_id);
            }
            BrowserAction::Close { tab_id } => self.close(&scope, &tab_id).await?,
            BrowserAction::OpenExternal { tab_id } => {
                self.tab(&tab_id, &scope)?;
                let view = app
                    .get_webview("main")
                    .ok_or_else(|| error("The Desktop controller has closed."))?;
                let url = self
                    .lock()?
                    .tabs
                    .iter()
                    .find(|t| t.dto.id == tab_id)
                    .map(|t| t.dto.page.url.clone())
                    .ok_or_else(|| error("This tab has closed."))?;
                self.hide_all();
                engine::open_external(&view, &url)
                    .await
                    .map_err(engine_error)?;
            }
            BrowserAction::OpenPopup { tab_id } => {
                self.tab(&tab_id, &scope)?;
                let url = self
                    .lock()?
                    .tabs
                    .iter()
                    .find(|t| t.dto.id == tab_id)
                    .and_then(|t| t.dto.popup_url.clone())
                    .ok_or_else(|| error("This page has no pending popup."))?;
                self.create(app, &scope, &url).await?;
                self.dismiss(&tab_id)?;
            }
            BrowserAction::DismissNotice { tab_id } => {
                self.tab(&tab_id, &scope)?;
                self.dismiss(&tab_id)?;
            }
            other => {
                let (id, action) = match other {
                    BrowserAction::Back { tab_id } => (tab_id, NavigationAction::Back),
                    BrowserAction::Forward { tab_id } => (tab_id, NavigationAction::Forward),
                    BrowserAction::Reload { tab_id } => (tab_id, NavigationAction::Reload),
                    BrowserAction::Stop { tab_id } => (tab_id, NavigationAction::Stop),
                    _ => return Err(error("This browser operation is unavailable.")),
                };
                let (view, _) = self.tab(&id, &scope)?;
                if matches!(action, NavigationAction::Stop) {
                    self.lock()?.event(&id, BrowserEvent::Loading(false));
                } else {
                    self.reset_navigation(&id, None)?;
                }
                view.control(action).await.map_err(engine_error)?;
            }
        }
        Ok(())
    }

    fn dismiss(&self, id: &str) -> Result<(), CommandErrorDto> {
        if let Some(tab) = self.lock()?.tabs.iter_mut().find(|t| t.dto.id == id) {
            tab.dto.popup_url = None;
            tab.dto.notice = None;
        }
        Ok(())
    }

    fn reset_navigation(&self, id: &str, url: Option<&str>) -> Result<(), CommandErrorDto> {
        if let Some(tab) = self.lock()?.tabs.iter_mut().find(|t| t.dto.id == id) {
            tab.dto.error = None;
            tab.dto.notice = None;
            tab.dto.popup_url = None;
            tab.dto.page.loading = true;
            if let Some(url) = url {
                tab.dto.page.url = url.into();
            }
        }
        Ok(())
    }

    fn profile(
        &self,
        scope: &str,
    ) -> Result<(std::path::PathBuf, Option<Webview>), CommandErrorDto> {
        let mut state = self.lock()?;
        if state.draining {
            return Err(error(DRAINING_MESSAGE));
        }
        if state.tabs.len() >= MAX_TABS {
            return Err(error(
                "Eight browser tabs are already open. Close a tab before opening another.",
            ));
        }
        if !state.profiles.contains_key(scope) {
            let store = SettingsStore::open_application()?;
            let directory = tempfile::Builder::new()
                .prefix("browser-session-")
                .tempdir_in(store.application_root())
                .map_err(|_| error("The temporary browser session could not be created."))?;
            state.profiles.insert(scope.to_owned(), directory);
            state.sessions.insert(
                scope.to_owned(),
                format!("browser-session-{}", uuid::Uuid::new_v4()),
            );
        }
        Ok((
            state
                .profiles
                .get(scope)
                .ok_or_else(|| error("The browser profile is unavailable."))?
                .path()
                .to_owned(),
            state
                .tabs
                .iter()
                .filter(|t| t.scope == scope)
                .find_map(|t| t.view.system_view().cloned()),
        ))
    }

    async fn create(
        &self,
        app: &AppHandle,
        scope: &str,
        address: &str,
    ) -> Result<(), CommandErrorDto> {
        let url = if address.trim().is_empty() {
            None
        } else {
            Some(engine::parse_address(address).map_err(engine_error)?)
        };
        let policy = NavigationPolicy::default();
        if let Some(url) = &url {
            policy.authorize(url).map_err(engine_error)?;
        }
        let (directory, source) = self.profile(scope)?;
        let id = format!("browser-{}", uuid::Uuid::new_v4());
        let weak = Arc::downgrade(&self.data);
        let event_id = id.clone();
        let sink: EventSink = Arc::new(move |event| {
            let crashed = matches!(event, BrowserEvent::Crashed);
            let view = weak.upgrade().and_then(|data| {
                let mut state = data.lock().ok()?;
                state.event(&event_id, event);
                crashed
                    .then(|| {
                        state
                            .tabs
                            .iter()
                            .find(|tab| tab.dto.id == event_id)
                            .map(|tab| tab.view.clone())
                    })
                    .flatten()
            });
            if let Some(view) = view {
                let _ = view.hide();
            }
        });
        let generation = self.lock()?.generation;
        let view = super::guest::create(
            app,
            &id,
            generation,
            directory,
            source,
            policy.clone(),
            sink,
        )
        .await?;
        self.hide_all();
        {
            let mut state = self.lock()?;
            let session_id = state
                .sessions
                .get(scope)
                .cloned()
                .ok_or_else(|| error("The browser session identity is unavailable."))?;
            state.selected.insert(scope.to_owned(), id.clone());
            state.tabs.push(Tab {
                scope: scope.to_owned(),
                view: view.clone(),
                policy,
                heartbeat: None,
                presentation_epoch: Arc::new(AtomicU64::new(0)),
                dto: BrowserTabDto {
                    id: id.clone(),
                    session_id,
                    control: "human",
                    page: PageState {
                        url: url.as_ref().map(ToString::to_string).unwrap_or_default(),
                        title: "New tab".into(),
                        loading: url.is_some(),
                        ..PageState::default()
                    },
                    error: None,
                    notice: None,
                    popup_url: None,
                },
            });
        }
        if let Some(url) = url
            && view.navigate(url).is_err()
        {
            self.lock()?.event(&id, BrowserEvent::Failed);
        }
        Ok(())
    }

    async fn close(&self, scope: &str, id: &str) -> Result<(), CommandErrorDto> {
        let (view, _) = self.tab(id, scope)?;
        let _ = view.hide();
        view.close()
            .await
            .map_err(|_| error("This browser tab could not close."))?;
        let mut state = self.lock()?;
        state.tabs.retain(|t| t.dto.id != id);
        if state
            .selected
            .get(scope)
            .is_some_and(|selected| selected == id)
        {
            state.selected.remove(scope);
            if let Some(next) = state
                .tabs
                .iter()
                .find(|t| t.scope == scope)
                .map(|t| t.dto.id.clone())
            {
                state.selected.insert(scope.to_owned(), next);
            }
        }
        if !state.tabs.iter().any(|t| t.scope == scope) {
            state.profiles.remove(scope);
            state.sessions.remove(scope);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_close_can_reach_reconciliation_but_other_hide_errors_block_teardown() {
        assert!(teardown_hide_result(Err(engine::BrowserError::Closed)).is_ok());
        for failure in [
            engine::BrowserError::TimedOut,
            engine::BrowserError::Unavailable,
        ] {
            assert!(teardown_hide_result(Err(failure)).is_err());
        }
    }

    #[tokio::test]
    async fn ordinary_tab_teardown_does_not_enter_application_shutdown() {
        let manager = BrowserManager::default();
        manager.close_all_settled().await.unwrap();
        assert!(manager.require_open().is_ok());
        assert!(!manager.lock().unwrap().draining);
    }

    #[tokio::test]
    async fn browser_admission_stays_revoked_during_remaining_application_cleanup() {
        let manager = BrowserManager::default();
        manager.lock().unwrap().scope = Some("workspace".into());
        let generation = manager.drain_for_shutdown().await.unwrap();
        // AppState still has asynchronous runtime/client cleanup ahead of it.
        // No operation guard is held, but even a refreshed generation is rejected.
        assert!(manager.operation.try_lock().is_ok());
        assert_eq!(
            manager.require_open().unwrap_err().message,
            DRAINING_MESSAGE
        );
        assert_eq!(
            manager.validate(generation).unwrap_err().message,
            DRAINING_MESSAGE
        );
        let snapshot = manager.snapshot().await.unwrap();
        assert!(!snapshot.available && !snapshot.engine.ready);
        assert_eq!(snapshot.engine.message.as_deref(), Some(DRAINING_MESSAGE));
        assert!(!snapshot.engine.agent_control_available);
        assert!(
            manager
                .close_all_authorized(generation, "workspace")
                .await
                .is_err()
        );
        assert_eq!(manager.lock().unwrap().generation, generation);
        // Only a trusted Quit retry can resume reconciliation.
        assert_eq!(manager.drain_for_shutdown().await.unwrap(), generation + 1);
        assert!(manager.require_open().is_err());
    }

    #[tokio::test]
    async fn stale_certificate_teardown_is_rejected_after_waiting_for_operation_ownership() {
        let manager = Arc::new(BrowserManager::default());
        {
            let mut state = manager.lock().unwrap();
            state.scope = Some("workspace".into());
            state.generation = 7;
        }
        let operation = manager.operation.lock().await;
        let queued = manager.clone();
        let attempt =
            tokio::spawn(async move { queued.close_all_authorized(7, "workspace").await.is_err() });
        tokio::task::yield_now().await;
        // A same-workspace controller reload revokes the original authorization.
        manager.lock().unwrap().generation = 8;
        drop(operation);
        assert!(attempt.await.unwrap());
        let state = manager.lock().unwrap();
        // Rejection occurs before advancing lifecycle or planning native effects.
        assert_eq!(state.generation, 8);
        assert_eq!(state.scope.as_deref(), Some("workspace"));
        assert!(state.tabs.is_empty());
    }

    #[tokio::test]
    async fn certificate_teardown_retains_operation_ownership_until_import_finishes() {
        let manager = BrowserManager::default();
        {
            let mut state = manager.lock().unwrap();
            state.scope = Some("workspace".into());
            state.generation = 7;
        }
        let (generation, operation) = manager.close_all_authorized(7, "workspace").await.unwrap();
        assert_eq!(generation, 8);
        assert!(manager.operation.try_lock().is_err());
        drop(operation);
        assert!(manager.operation.try_lock().is_ok());
    }
}
