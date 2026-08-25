//! Deterministic, restartable projections over the authoritative event journal.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    EventEnvelope, ExternalWorkRetryState, ProjectionBatch, ProjectionMutation, ProjectionStatus,
    ProjectionWorkItem,
};
use colossus_ports::{
    AggregateRepository, EventJournal, ExternalWorkQueue, ProjectionStore, StoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use time::{Duration, OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

mod external_work;
pub use external_work::*;

mod worker;
pub use worker::*;

mod handlers;
pub use handlers::*;

mod repositories;
pub use repositories::*;

mod activity;
pub use activity::*;

#[cfg(test)]
mod tests;
