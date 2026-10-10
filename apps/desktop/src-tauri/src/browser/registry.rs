use super::dto::{BrowserEngineDto, BrowserSnapshotDto, BrowserTabDto};
use colossus_native_browser::{BrowserEvent, BrowserView, NavigationPolicy};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

pub(super) const MAX_TABS: usize = 8;
pub(super) const DRAINING_MESSAGE: &str = "The browser is closing for application shutdown.";

pub(super) struct Tab {
    pub(super) scope: String,
    pub(super) view: BrowserView,
    pub(super) policy: NavigationPolicy,
    pub(super) dto: BrowserTabDto,
    pub(super) heartbeat: Option<Instant>,
    pub(super) presentation_epoch: Arc<AtomicU64>,
}

#[derive(Default)]
pub(super) struct Registry {
    pub(super) draining: bool,
    pub(super) generation: u64,
    pub(super) scope: Option<String>,
    pub(super) tabs: Vec<Tab>,
    pub(super) selected: HashMap<String, String>,
    pub(super) profiles: HashMap<String, tempfile::TempDir>,
    pub(super) sessions: HashMap<String, String>,
}

impl Registry {
    pub(super) fn snapshot(&self) -> BrowserSnapshotDto {
        let mut engine = BrowserEngineDto::current();
        if self.tabs.iter().any(|tab| {
            Some(&tab.scope) == self.scope.as_ref() && tab.view.contained_session_id().is_some()
        }) {
            engine.preview = false;
            engine.ready = true;
            engine.agent_control_available = true;
            engine.message = None;
        }
        if self.draining {
            engine.ready = false;
            engine.message = Some(DRAINING_MESSAGE.into());
        }
        BrowserSnapshotDto {
            available: engine.ready,
            engine,
            generation: self.generation,
            tabs: self
                .tabs
                .iter()
                .filter(|t| Some(&t.scope) == self.scope.as_ref())
                .map(|t| {
                    let mut dto = t.dto.clone();
                    if !t.view.human_control_available() && dto.control == "human" {
                        dto.control = "paused";
                    }
                    dto
                })
                .collect(),
            selected_tab_id: self
                .scope
                .as_ref()
                .and_then(|scope| self.selected.get(scope))
                .cloned(),
        }
    }

    pub(super) fn event(&mut self, id: &str, event: BrowserEvent) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.dto.id == id) else {
            return;
        };
        match event {
            BrowserEvent::Loading(loading) => {
                tab.dto.page.loading = loading;
                if loading {
                    tab.dto.error = None;
                }
            }
            BrowserEvent::Failed => {
                tab.dto.page.loading = false;
                tab.dto.error = Some(
                    "This page could not load. Check its address and connection, then retry."
                        .into(),
                );
            }
            BrowserEvent::Crashed => {
                tab.dto.page.loading = false;
                tab.dto.error = Some(
                    "This browser tab stopped responding. Close it and open a new tab.".into(),
                );
                tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
                tab.heartbeat = None;
            }
            BrowserEvent::Blocked => {
                tab.dto.notice = Some("This page requested access that is unavailable in the embedded browser. You can open it in your system browser.".into());
            }
            BrowserEvent::Download => {
                tab.dto.page.loading = false;
                tab.dto.notice = Some("To download this file, open the page in your system browser. You may need to sign in there.".into());
            }
            BrowserEvent::Popup(url) => {
                // Keep the first pending request; popup storms cannot replace a
                // destination while the user is deciding whether to open it.
                if tab.dto.popup_url.is_none()
                    && colossus_native_browser::parse_address(&url).is_ok()
                {
                    tab.dto.popup_url = Some(url);
                    tab.dto.notice = Some("This page wants to open another tab.".into());
                }
            }
            BrowserEvent::TlsFailed => {
                tab.dto.page.loading = false;
                tab.dto.error = Some("The website's certificate could not be verified. Check its hostname, expiry, and native CA trust configuration. TLS verification cannot be bypassed.".into());
            }
            BrowserEvent::AuthenticationRequired => {
                tab.dto.page.loading = false;
                tab.dto.notice = Some("This website requires a client certificate. A native identity must be provisioned and reviewed for this exact HTTPS origin.".into());
            }
        }
    }
}
