//! Versioned serializable contracts crossing Colossus boundaries.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

mod agent;
mod decisions;
mod distribution;
mod integrations;
mod journal;
mod memory;
mod observability;
mod presentation;
mod research;
mod sandbox;
mod security;
mod session;
mod skills;
mod work;
mod workflow;

pub use agent::*;
pub use decisions::*;
pub use distribution::*;
pub use integrations::*;
pub use journal::*;
pub use memory::*;
pub use observability::*;
pub use presentation::*;
pub use research::*;
pub use sandbox::*;
pub use security::*;
pub use session::*;
pub use skills::*;
pub use work::*;
pub use workflow::*;

#[cfg(test)]
mod tests;
