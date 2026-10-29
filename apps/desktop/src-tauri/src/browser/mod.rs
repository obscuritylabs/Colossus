//! Human-operated browser surface. No agent tools or worker authority live here.

#[cfg(feature = "browser-test-bridge")]
pub(crate) mod acceptance;
pub(crate) mod bootstrap;
pub(crate) mod certificates;
#[cfg(all(windows, feature = "client-identity-test-bridge"))]
#[path = "acceptance/client_identity.rs"]
pub(crate) mod client_identity_acceptance;
#[cfg(all(
    windows,
    any(
        feature = "windows-browser-development",
        feature = "windows-browser-functional-preview"
    )
))]
pub(crate) mod client_identity_broker;
pub(crate) mod commands;
#[cfg(any(
    feature = "embedded-chromium-preview",
    any(
        feature = "macos-contained-browser-diagnostic",
        any(
            feature = "windows-browser-development",
            feature = "windows-browser-functional-preview"
        )
    )
))]
pub(crate) mod contained;
mod diagnostics;
pub(crate) mod dto;
mod guest;
mod inspection;
mod manager;
pub(crate) mod preferences;
mod presentation;
mod profile;
mod registry;
pub(crate) mod viewport;
#[cfg(all(windows, feature = "embedded-chromium-preview"))]
mod windows_entry;
#[cfg(any(
    feature = "windows-browser-development",
    feature = "windows-browser-functional-preview"
))]
mod windows_viewer;

pub(crate) use manager::BrowserManager;
pub(crate) use viewport::{handle_window_event, start_watchdog};
