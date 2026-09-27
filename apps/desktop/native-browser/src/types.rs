use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Closed navigation operations supported by every guest engine.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationAction {
    Back,
    Forward,
    Reload,
    Stop,
}

/// Native observations. Titles and URLs are untrusted display data.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub loading: bool,
}

/// Bounded engine events; no page content or native paths cross this boundary.
#[derive(Clone, Debug)]
pub enum BrowserEvent {
    Loading(bool),
    Failed,
    Crashed,
    Blocked,
    Download,
    Popup(String),
}

/// The manager owns the sink and validates the tab's lifecycle on delivery.
pub type EventSink = Arc<dyn Fn(BrowserEvent) + Send + Sync>;

/// Categorical engine errors deliberately omit page data and OS error strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserError {
    InvalidAddress,
    Unavailable,
    Closed,
    TimedOut,
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidAddress => "Enter an HTTP or HTTPS address without embedded credentials.",
            Self::Unavailable => "The embedded browser is unavailable on this device.",
            Self::Closed => "This browser tab is no longer available.",
            Self::TimedOut => "The browser did not respond. Close the tab and try again.",
        })
    }
}

impl std::error::Error for BrowserError {}
