//! Metadata-only operational telemetry derived from persisted journal events.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    EventEnvelope, RunTelemetryDetail, RunTelemetrySummary, TelemetryEventRecord, TelemetryMetrics,
};
use colossus_ports::{EventJournal, StoreError};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_SCAN_EVENTS: u64 = 100_000;
const MAX_RUNS: usize = 1_000;
const MAX_DETAIL_EVENTS: usize = 10_000;

mod analysis;
use analysis::*;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
