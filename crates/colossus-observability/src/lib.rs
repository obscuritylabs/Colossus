//! Live OpenTelemetry instrumentation and host-owned exporters.
//!
//! Durable historical analytics remain in `colossus-telemetry`. This crate emits
//! best-effort live signals and decorates the journal without changing canonical
//! storage behavior.

#![allow(clippy::missing_errors_doc)]

mod config;
mod conventions;
mod journal;
mod metrics;
#[cfg(feature = "host-exporters")]
mod sdk;
mod trace_context;

pub use config::*;
pub use conventions::*;
pub use journal::*;
pub use metrics::*;
#[cfg(feature = "host-exporters")]
pub use sdk::*;
pub use trace_context::*;
