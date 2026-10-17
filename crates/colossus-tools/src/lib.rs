//! Strict active tool catalog and shared argument validation.

use colossus_contracts::{ModelToolDefinition, ToolCall, ToolSpec};
use colossus_ports::{ToolError, ToolRegistry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod builtin;
pub use builtin::*;

mod browser;

mod process_limits;
mod process_sessions;
mod workflows;
pub use process_limits::with_process_limits;

mod registry;
pub use registry::*;

mod observation;
pub use observation::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod browser_tests;
