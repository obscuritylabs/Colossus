use super::*;

/// Default and hard maximum model turns per run.
pub const DEFAULT_MAX_TURNS: u16 = 24;
/// Absolute bound preventing unbounded model/tool loops.
pub const MAX_TURNS: u16 = 100;
pub(super) const TOOL_ARGUMENT_RECOVERY_LIMIT: u8 = 2;
pub(super) const INVALID_TOOL_ARGUMENTS_CODE: &str = "provider.invalid_tool_arguments";

#[derive(Default)]
pub(super) struct RunScope<'a> {
    pub(super) requested_run_id: Option<&'a str>,
    pub(super) goal_id: Option<&'a str>,
    pub(super) plan_id: Option<&'a str>,
    pub(super) subagent_id: Option<&'a str>,
    pub(super) workflow_id: Option<&'a str>,
    pub(super) workflow_hash: Option<&'a str>,
    pub(super) step_id: Option<&'a str>,
    pub(super) attempt: Option<u32>,
    pub(super) active_skills: &'a [String],
    pub(super) allowed_tools: Option<&'a [String]>,
    pub(super) mode: AgentRunMode,
    pub(super) create_requested_session: bool,
    pub(super) include_provider_response_diagnostics: bool,
}

/// Application-loop failure with terminal states distinguishable by callers.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Configuration or route selection failed.
    #[error("agent configuration failed: {0}")]
    Configuration(String),
    /// Provider failed with a known outcome.
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
    /// Tool policy or execution prevented continuation.
    #[error(transparent)]
    Tool(#[from] ToolError),
    /// Journal durability failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Context preparation could not safely fit or persist model-visible history.
    #[error(transparent)]
    Context(#[from] ContextError),
    /// Malformed tool-call recovery was exhausted without executing a tool.
    #[error("provider tool-call argument recovery exhausted after {attempts} attempts")]
    ToolArgumentRecoveryExhausted {
        /// Number of attempted correction turns.
        attempts: u8,
    },
    /// Model reached the configured turn budget without final output.
    #[error("model turn limit exhausted after {max_turns} turns")]
    MaxTurns {
        /// Configured turn ceiling.
        max_turns: u16,
    },
    /// Normalized turn contained neither visible output nor a tool call.
    #[error("provider returned no visible assistant output or tool calls")]
    EmptyTurn,
    /// Plan Mode exhausted its correction opportunity without persisting its required draft.
    #[error("plan mode completed without the required plan create or update")]
    PlanWriteRequired,
    /// The operator requested a cooperative stop at a safe boundary.
    #[error("agent run cancelled by the operator")]
    Cancelled {
        /// Durable cancellation evidence.
        result: Box<AgentRunCancellation>,
    },
}
