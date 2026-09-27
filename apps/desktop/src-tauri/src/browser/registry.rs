use super::dto::{BrowserSnapshotDto, BrowserTabDto};
use colossus_native_browser::{BrowserEvent, NavigationPolicy};
use std::{collections::HashMap, time::Instant};
use tauri::Webview;

pub(super) const MAX_TABS: usize = 8;

pub(super) struct Tab {
    pub(super) scope: String,
    pub(super) view: Webview,
    pub(super) policy: NavigationPolicy,
    pub(super) dto: BrowserTabDto,
    pub(super) heartbeat: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Registry {
    pub(super) generation: u64,
    pub(super) scope: Option<String>,
    pub(super) tabs: Vec<Tab>,
    pub(super) selected: HashMap<String, String>,
    pub(super) profiles: HashMap<String, tempfile::TempDir>,
}

impl Registry {
    pub(super) fn snapshot(&self) -> BrowserSnapshotDto {
        BrowserSnapshotDto {
            available: cfg!(feature = "browser-preview") && cfg!(any(windows, target_os = "macos")),
            generation: self.generation,
            tabs: self
                .tabs
                .iter()
                .filter(|t| Some(&t.scope) == self.scope.as_ref())
                .map(|t| t.dto.clone())
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
                let _ = tab.view.hide();
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
        }
    }
}
