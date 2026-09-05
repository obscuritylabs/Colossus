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
mod plugin_management;
mod plugin_mcp;
mod plugin_reads;
mod plugin_selection;
mod plugins;
mod presentation;
mod research;
mod sandbox;
mod security;
mod session;
mod work;
mod workflow;

pub use agent::*;
pub use decisions::*;
pub use distribution::*;
pub use integrations::*;
pub use journal::*;
pub use memory::*;
pub use observability::*;
pub use plugin_management::*;
pub use plugin_mcp::*;
pub use plugin_reads::*;
pub use plugin_selection::*;
pub use plugins::*;
pub use presentation::*;
pub use research::*;
pub use sandbox::*;
pub use security::*;
pub use session::*;
pub use work::*;
pub use workflow::*;

#[cfg(test)]
mod tests;
