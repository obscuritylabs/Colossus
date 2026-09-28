use colossus_native_browser::PageState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserTabDto {
    pub(crate) id: String,
    #[serde(flatten)]
    pub(crate) page: PageState,
    pub(crate) error: Option<String>,
    pub(crate) notice: Option<String>,
    pub(crate) popup_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserSnapshotDto {
    pub(crate) available: bool,
    pub(crate) generation: u64,
    pub(crate) tabs: Vec<BrowserTabDto>,
    pub(crate) selected_tab_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserRequest {
    pub(crate) generation: u64,
    pub(crate) action: BrowserAction,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum BrowserAction {
    New { url: String },
    Navigate { tab_id: String, url: String },
    Select { tab_id: String },
    Close { tab_id: String },
    Back { tab_id: String },
    Forward { tab_id: String },
    Reload { tab_id: String },
    Stop { tab_id: String },
    OpenExternal { tab_id: String },
    OpenPopup { tab_id: String },
    DismissNotice { tab_id: String },
    Clear,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserViewportRequest {
    pub(crate) generation: u64,
    pub(crate) tab_id: Option<String>,
    pub(crate) rect: Option<BrowserRect>,
}
