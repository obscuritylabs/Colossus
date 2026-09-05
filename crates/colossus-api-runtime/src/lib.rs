//! Runtime adapter for durable public Colossus application resources.
//!
//! This crate coordinates the transport-neutral public API with the real runtime. It
//! persists run updates before waking watchers and keeps execution independent from any
//! client connection.

#![allow(clippy::missing_errors_doc)]

mod admission;
mod feed;
mod interactions;
#[cfg(test)]
mod plugin_tests;
mod plugins;
mod service;
#[cfg(test)]
mod service_tests;
mod writer;

pub use admission::{RunAdmissionConfig, RunAdmissionConfigError};
pub use interactions::{PublicApprovalMode, PublicApprovalModeProvider, PublicInteractionRouter};
pub use plugins::RuntimeExtensionApi;
pub use service::RuntimeAgentRunApi;
