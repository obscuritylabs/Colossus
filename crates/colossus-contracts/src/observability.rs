use serde::{Deserialize, Serialize};

/// Validated, normalized W3C Trace Context safe to persist with encrypted work.
///
/// Baggage is deliberately absent so callers cannot cause arbitrary metadata to be
/// retained or propagated through agent and tool execution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteTraceContext {
    /// Normalized W3C `traceparent` value.
    pub traceparent: String,
    /// Validated W3C `tracestate`, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}
