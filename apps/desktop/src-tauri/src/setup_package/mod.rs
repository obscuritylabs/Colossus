//! Offline, operator-selected Desktop onboarding packages.
mod archive;
mod commands;
mod configuration;
mod types;

pub(crate) use commands::*;
pub(crate) use configuration::validate_saved;
pub(crate) use types::SavedSetupPackage;

#[cfg(test)]
mod tests;
