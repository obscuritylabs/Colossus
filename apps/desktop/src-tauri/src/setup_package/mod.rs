//! Offline, operator-selected Desktop onboarding packages.
mod archive;
mod catalog;
mod commands;
mod configuration;
mod export;
mod global_catalog;
mod globals;
mod provider_presentation;
mod types;
pub(crate) use provider_presentation::{
    ProviderPresentation, provider_presentation, provider_presentations,
};

pub(crate) use catalog::{migrate_catalog, sync_configured_credentials};
pub(crate) use commands::*;
pub(crate) use configuration::validate_saved;
pub(crate) use types::SavedSetupPackage;

#[cfg(test)]
mod global_tests;
#[cfg(test)]
mod tests;
