//! Canonical event-sourced tasks and future-facing key decisions.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    Actor, DecisionPriority, DecisionSource, DecisionStatus, EventClassification, ExecutionContext,
    GoalRecord, GoalStatus, KeyDecision, NewEvent, PlanRecord, PlanStatus, PlanStep, SubagentJob,
    SubagentStatus, TaskRecord, TaskStatus,
};
use colossus_ports::{EventJournal, SessionRepository, StoreError, WorkRepository};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

mod validation;
use validation::*;

mod repository;
pub use repository::*;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
