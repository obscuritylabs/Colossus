//! Bounded human terminal operations, independent from durable agent commands.
use colossus_sdk::{ApiError, HumanTerminalReply};
use serde::{Deserialize, Serialize};

/// Advertised only when the dedicated native application can open human terminals.
pub const TERMINAL_CAPABILITY: &str = "human_terminal.v1";
/// Encoded request/response budget, including base64 and bounded session metadata.
pub const MAX_TERMINAL_PAYLOAD_BYTES: usize = 128 * 1024;
/// Independent long-poll admission; cannot consume interactive control slots.
pub const MAX_TERMINAL_READ_REQUESTS: usize = 8;
/// Independent interactive admission, including interrupt and close.
pub const MAX_TERMINAL_CONTROL_REQUESTS: usize = 8;
/// Dedicated queue; bulk terminal traffic cannot occupy run/control queue capacity.
pub const MAX_TERMINAL_QUEUED_FRAMES: usize =
    MAX_TERMINAL_READ_REQUESTS + MAX_TERMINAL_CONTROL_REQUESTS;

/// Explicit public result; unknown mutation outcomes are never silently retried.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalReply {
    /// Caller-owned terminal display or lifecycle metadata.
    Result {
        /// Exact caller-owned public SDK terminal projection.
        reply: HumanTerminalReply,
    },
    /// Bounded SDK error with effect certainty.
    Failed {
        /// Categorical public error; contains no private transport diagnostics.
        error: ApiError,
    },
}

/// Connection-local random identifier, never a terminal authority token.
pub fn valid_terminal_request_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
