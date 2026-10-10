//! Human-operated browser surface. No agent tools or worker authority live here.

#[cfg(feature = "browser-test-bridge")]
pub(crate) mod acceptance;
pub(crate) mod bootstrap;
pub(crate) mod certificates;
pub(crate) mod commands;
#[cfg(feature = "embedded-chromium-preview")]
pub(crate) mod contained;
pub(crate) mod dto;
mod guest;
mod inspection;
mod manager;
mod presentation;
mod registry;
pub(crate) mod viewport;
#[cfg(all(windows, feature = "embedded-chromium-preview"))]
mod windows_entry;

pub(crate) use manager::BrowserManager;
pub(crate) use viewport::{handle_window_event, start_watchdog};
