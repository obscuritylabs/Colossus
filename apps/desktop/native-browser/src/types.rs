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
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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
    /// Native main-document response status; never response headers or body.
    HttpStatus(u16),
    /// A redirect occurred, without retaining a potentially secret URL.
    Redirected,
    /// Native engine error category code, without page data or credentials.
    NavigationFailure(i32),
    Failed,
    Crashed,
    /// The supervised context reached its original fixed authority deadline.
    Expired,
    Blocked,
    Download,
    Popup(String),
    /// Chromium rejected a certificate; no TLS-error override is available.
    TlsFailed,
    /// No reviewed, exact-origin native client identity was selected.
    AuthenticationRequired,
}

/// The manager owns the sink and validates the tab's lifecycle on delivery.
pub type EventSink = Arc<dyn Fn(BrowserEvent) + Send + Sync>;

/// Categorical engine errors deliberately omit page data and OS error strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserError {
    InvalidAddress,
    Unavailable,
    Closed,
    /// The original supervised network/session authority expired.
    Expired,
    /// The native owner is changing the document; hide pixels until a fresh target is admitted.
    Stale,
    TimedOut,
    ComponentMissing,
    BootstrapRequired,
    NativeAcceptanceRequired,
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidAddress => "Enter an HTTP or HTTPS address without embedded credentials.",
            Self::Unavailable => "The embedded browser is unavailable on this device.",
            Self::Closed => "This browser tab is no longer available.",
            Self::Expired => "This browser session expired. Continue in a new session to browse again.",
            Self::Stale => "The browser page is changing. Waiting for its current document.",
            Self::TimedOut => "The browser did not respond. Close the tab and try again.",
            Self::ComponentMissing => "The verified Chromium component is not included in this build.",
            Self::BootstrapRequired => "Embedded Chromium requires the supported sandboxed native bootstrap on this platform.",
            Self::NativeAcceptanceRequired => "Embedded Chromium has not passed native acceptance on this platform.",
        })
    }
}

impl std::error::Error for BrowserError {}
