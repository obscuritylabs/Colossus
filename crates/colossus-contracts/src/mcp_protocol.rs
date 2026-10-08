//! Operator-selected compatibility for remote MCP connections.

use serde::{Deserialize, Serialize};

/// Streamable HTTP lifecycle selected for one configured server.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum McpProtocolVersion {
    /// Discover the current protocol, falling back to legacy initialization.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// Require the sessionless, per-request 2026 protocol.
    #[serde(rename = "2026-07-28")]
    V2026,
    /// Initialize using 2025-11-25; accept supported older negotiation.
    #[serde(rename = "2025-11-25")]
    V2025,
}
