//! Offline, operator-selected Desktop onboarding packages.
mod archive;
mod catalog;
mod commands;
mod configuration;
mod types;

pub(crate) use catalog::{migrate_catalog, sync_configured_credentials};
pub(crate) use commands::*;
pub(crate) use configuration::validate_saved;
pub(crate) use types::SavedSetupPackage;

#[cfg(test)]
mod tests;
