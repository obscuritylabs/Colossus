//! Canonical event-sourced research runs, sources, claims, and cited reports.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, ModelMessage, ModelMessageRole,
    NewEvent, ResearchClaim, ResearchDepth, ResearchLane, ResearchLaneStatus, ResearchPhase,
    ResearchProgress, ResearchProgressStatus, ResearchRun, ResearchSource, ResearchSourceKind,
    ResearchStatus,
};
use colossus_ports::{
    EventJournal, ResearchRepository, SessionRepository, StoreError, collect_stream_ids,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, collections::BTreeSet, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

mod repository;
pub use repository::*;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
