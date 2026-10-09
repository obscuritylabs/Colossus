//! Human-operated browser surface. No agent tools or worker authority live here.

#[cfg(feature = "browser-test-bridge")]
pub(crate) mod acceptance;
pub(crate) mod commands;
pub(crate) mod dto;
mod inspection;
mod manager;
mod registry;
pub(crate) mod viewport;

pub(crate) use manager::BrowserManager;
pub(crate) use viewport::{handle_window_event, start_watchdog};
