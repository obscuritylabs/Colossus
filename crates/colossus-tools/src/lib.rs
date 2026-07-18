//! Strict active tool catalog and shared argument validation.

use colossus_contracts::{ModelToolDefinition, ToolCall, ToolSpec};
use colossus_ports::{ToolError, ToolRegistry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod builtin;
pub use builtin::*;

mod registry;
pub use registry::*;

#[cfg(test)]
mod tests;
