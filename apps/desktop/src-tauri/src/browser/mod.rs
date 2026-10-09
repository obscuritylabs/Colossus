//! Human-operated browser surface. No agent tools or worker authority live here.

#[cfg(feature = "browser-test-bridge")]
pub(crate) mod acceptance;
pub(crate) mod bootstrap;
pub(crate) mod certificates;
pub(crate) mod commands;
pub(crate) mod dto;
mod guest;
mod inspection;
mod manager;
mod presentation;
mod registry;
pub(crate) mod viewport;

pub(crate) use manager::BrowserManager;
pub(crate) use viewport::{handle_window_event, start_watchdog};
