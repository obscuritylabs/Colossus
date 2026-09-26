use colossus_native_browser::{
    self as engine, BrowserEvent, EventSink, NavigationAction, NavigationPolicy, PageState,
};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{
    AppHandle, Manager as _, Webview, WebviewUrl,
    webview::{DownloadEvent, NewWindowResponse, WebviewBuilder},
};

use super::{
    dto::{BrowserAction, BrowserSnapshotDto, BrowserTabDto},
    registry::{MAX_TABS, Registry, Tab},
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

impl BrowserManager {
    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Registry>, CommandErrorDto> {
        self.data
            .lock()
            .map_err(|_| error("The browser session is unavailable."))
    }

    pub(crate) fn selection_changed(&self, scope: Option<String>) {
        if let Ok(mut state) = self.data.lock()
            && state.scope != scope
        {
            for tab in &mut state.tabs {
                let _ = tab.view.hide();
                tab.heartbeat = None;
            }
            state.generation = state.generation.wrapping_add(1);
            state.scope = scope;
        }
    }

    pub(crate) fn hide_all(&self) {
        if let Ok(mut state) = self.data.lock() {
            for tab in &mut state.tabs {
                let _ = tab.view.hide();
                tab.heartbeat = None;
            }
        }
    }

    pub(crate) fn controller_loading(&self) {
        self.hide_all();
        if let Ok(mut state) = self.data.lock() {
            state.generation = state.generation.wrapping_add(1);
        }
    }

    pub(crate) fn validate(&self, generation: u64) -> Result<String, CommandErrorDto> {
        let state = self.lock()?;
        if !state.snapshot().available {
            return Err(error("Browser preview is not enabled in this build."));
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
    ) -> Result<(Webview, NavigationPolicy), CommandErrorDto> {
        self.lock()?
            .tabs
            .iter()
            .find(|t| t.dto.id == id && t.scope == scope)
            .map(|t| (t.view.clone(), t.policy.clone()))
            .ok_or_else(|| error("This tab is no longer available in the selected workspace."))
    }

    pub(crate) async fn snapshot(&self) -> Result<BrowserSnapshotDto, CommandErrorDto> {
        let views: Vec<_> = {
            let state = self.lock()?;
            state
                .tabs
                .iter()
                .filter(|t| Some(&t.scope) == state.scope.as_ref())
                .map(|t| (t.dto.id.clone(), t.view.clone()))
                .collect()
        };
        for (id, view) in views {
            if let Ok(mut page) = engine::inspect(&view).await {
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
                let (view, _) = self.tab(&tab_id, &scope)?;
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
                engine::control(&view, action).await.map_err(engine_error)?;
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
                .find(|t| t.scope == scope)
                .map(|t| t.view.clone()),
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
            if let Some(data) = weak.upgrade()
                && let Ok(mut state) = data.lock()
            {
                state.event(&event_id, event);
            }
        });
        let popup_sink = sink.clone();
        let download_sink = sink.clone();
        let navigation = policy.clone();
        let mut builder = WebviewBuilder::new(
            &id,
            WebviewUrl::External(
                "about:blank"
                    .parse()
                    .map_err(|_| error("Browser initialization failed."))?,
            ),
        )
        .incognito(true)
        .data_directory(directory)
        .focused(false)
        .devtools(false)
        .on_navigation(move |url| navigation.allows(url.as_str()))
        .on_new_window(move |url, _| {
            popup_sink(BrowserEvent::Popup(url.to_string()));
            NewWindowResponse::Deny
        })
        .on_download(move |_, event| {
            if matches!(event, DownloadEvent::Requested { .. }) {
                download_sink(BrowserEvent::Download);
            }
            false
        });
        if let Some(source) = source {
            builder = engine::share_session(builder, &source)
                .await
                .map_err(engine_error)?;
        }
        let window = app
            .get_window("main")
            .ok_or_else(|| error("The Desktop window has closed."))?;
        let view = window
            .add_child(
                builder,
                tauri::LogicalPosition::new(-10_000.0, -10_000.0),
                tauri::LogicalSize::new(16.0, 16.0),
            )
            .map_err(|_| error("The browser engine could not start."))?;
        let _ = view.hide();
        if let Err(failure) = engine::harden(&view, policy.clone(), sink).await {
            let _ = engine::release(&view).await;
            let _ = view.close();
            return Err(engine_error(failure));
        }
        self.hide_all();
        {
            let mut state = self.lock()?;
            state.selected.insert(scope.to_owned(), id.clone());
            state.tabs.push(Tab {
                scope: scope.to_owned(),
                view: view.clone(),
                policy,
                heartbeat: None,
                dto: BrowserTabDto {
                    id: id.clone(),
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
        let _ = engine::release(&view).await;
        view.close()
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
        }
        Ok(())
    }
}
