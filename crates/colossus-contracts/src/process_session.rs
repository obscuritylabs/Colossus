//! Managed process status and released-output contracts.
use super::*;

/// Authority lifetime requested explicitly by the caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessLifetime {
    /// Terminate unfinished work when its agent run ends.
    #[default]
    Run,
    /// Remain owned by the workspace runtime across conversation turns.
    Workspace,
}

/// Observed state; only terminal states describe a completed invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSessionStatus {
    /// Waiting for authorization or launch.
    Starting,
    /// Supervisor has confirmed launch.
    Running,
    /// Stop requested; termination is not yet confirmed.
    Stopping,
    /// Process and descendants have been reaped.
    Exited,
    /// Explicit stop and cleanup confirmed.
    Stopped,
    /// Original execution deadline reached and cleanup confirmed.
    TimedOut,
    /// Known launch or execution failure.
    Failed,
    /// Runtime restarted; operating-system handles are never adopted.
    Interrupted,
    /// Effect or cleanup outcome could not be proven.
    OutcomeUnknown,
}
impl ProcessSessionStatus {
    /// Whether this invocation may still be executing.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}

/// Safe metadata for one invocation. No environment or operating-system handles.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSessionSummary {
    /// Opaque process-session identifier.
    pub id: String,
    /// Owning conversation.
    pub session_id: String,
    /// Initiating agent run.
    pub run_id: String,
    /// Authenticated initiating actor.
    pub owner: Actor,
    /// Exact delegated lineage, if any.
    pub subagent_id: Option<String>,
    /// Run or workspace lifetime; never changes after launch.
    pub lifetime: ProcessLifetime,
    /// Current observed lifecycle.
    pub status: ProcessSessionStatus,
    /// Credential-redacted command display.
    pub command: String,
    /// Sanitized working-directory display.
    pub cwd: String,
    /// Accepted time, Unix milliseconds.
    pub created_at_ms: u64,
    /// Actual execution deadline after launch, Unix milliseconds.
    pub deadline_ms: Option<u64>,
    /// Exit code only when confirmed by the supervisor.
    pub exit_code: Option<i32>,
    /// Bounded terminal explanation.
    pub reason: Option<String>,
    /// Output was dropped because of an authorized or retention bound.
    pub truncated: bool,
    /// Latest released output sequence; zero before any output.
    pub output_sequence: u64,
}

/// A bounded, released, display-safe output chunk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessOutputChunk {
    /// Monotonically increasing output cursor within this invocation.
    pub sequence: u64,
    /// Standard output decoded incrementally as UTF-8.
    pub stdout: String,
    /// Standard error decoded incrementally as UTF-8.
    pub stderr: String,
}

/// Cursor-based snapshot; reading or waiting never extends execution authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSessionSnapshot {
    /// Current lifecycle and safe metadata.
    pub session: ProcessSessionSummary,
    /// New output after the supplied exclusive cursor.
    pub chunks: Vec<ProcessOutputChunk>,
    /// Exclusive cursor for the next read.
    pub next_sequence: u64,
    /// Requested output is older than retained logs, including after restart.
    pub gap: bool,
}

/// Bounded session discovery, ordered by opaque identifier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSessionPage {
    /// Up to one hundred authorized summaries.
    pub sessions: Vec<ProcessSessionSummary>,
    /// Exclusive identifier for the next page.
    pub next_cursor: Option<String>,
}
