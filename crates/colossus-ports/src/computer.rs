//! Supervised native desktop boundary. Drivers neither authorize effects nor release pixels.

use crate::RunControl;
use async_trait::async_trait;
use colossus_contracts::{
    ComputerBinding, ComputerCapabilities, ComputerInput, ComputerLimits, ComputerObservation,
    ComputerObservationId, ComputerScreenshotDescriptor, ComputerSessionId, ComputerSessionSummary,
    ComputerValidationError, ComputerWindowId, ComputerWindowSummary,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One already-authorized owner-bound native dispatch. Model inputs never nominate its binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerDriverRequest {
    /// Authenticated immutable native owner, including current registered run.
    pub binding: ComputerBinding,
    /// Closed reviewed operation.
    pub command: ComputerCommand,
}

/// Closed native protocol operations; raw handles, shell, clipboard, and arbitrary code are absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerCommand {
    /// Enumerate bounded choices, retaining opaque IDs in this run's native registry.
    Windows {
        /// Positive number, no greater than the fixed protocol ceiling.
        max_windows: u16,
    },
    /// Acquire the physical writer lease for one previously observed native choice.
    Open {
        /// Exact observed opaque native window.
        window_id: ComputerWindowId,
        /// Runtime-derived envelope intersected with verified helper capabilities.
        limits: ComputerLimits,
    },
    /// Read current owner-bound epoch, including native human pause or resume.
    Status {
        /// Current native owned session.
        session_id: ComputerSessionId,
    },
    /// Obtain bounded semantics for the selected root or positively owned same-app modal.
    Snapshot {
        /// Exact owned native session.
        session_id: ComputerSessionId,
        /// Current positive native writer generation.
        control_generation: u64,
        /// Positive requested node bound, within the admitted session envelope.
        max_nodes: u16,
    },
    /// Capture target-only pixels into private native custody.
    Screenshot {
        /// Exact owned native session.
        session_id: ComputerSessionId,
        /// Current positive native writer generation.
        control_generation: u64,
        /// Positive transfer bound, within the admitted session envelope.
        max_bytes: u32,
    },
    /// Consume a fresh observation and execute exactly one bounded balanced input action.
    Input {
        /// Exact owned native session.
        session_id: ComputerSessionId,
        /// Current positive native writer generation.
        control_generation: u64,
        /// Fresh released native observation, consumed even if dispatch later fails.
        observation_id: ComputerObservationId,
        /// Reviewed typed input.
        input: ComputerInput,
    },
    /// Release automation and the writer lease, leaving the user's application running.
    Close {
        /// Exact owned native session.
        session_id: ComputerSessionId,
        /// Current positive native writer generation.
        control_generation: u64,
    },
}

impl ComputerDriverRequest {
    /// Recheck bounded protocol syntax before native dispatch; this grants no authority.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        self.binding.validate()?;
        self.command.validate()
    }
}

impl ComputerCommand {
    /// Validate fixed ceilings. Native custody separately enforces the admitted session limits.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        let valid = match self {
            Self::Windows { max_windows } => {
                (1..=ComputerLimits::HARD_MAX.max_windows).contains(max_windows)
            }
            Self::Open { limits, .. } => return limits.validate(),
            Self::Status { .. } => true,
            Self::Snapshot {
                control_generation,
                max_nodes,
                ..
            } => {
                *control_generation > 0
                    && (1..=ComputerLimits::HARD_MAX.max_snapshot_nodes).contains(max_nodes)
            }
            Self::Screenshot {
                control_generation,
                max_bytes,
                ..
            } => {
                *control_generation > 0
                    && (1..=ComputerLimits::HARD_MAX.max_screenshot_bytes).contains(max_bytes)
            }
            Self::Input {
                control_generation,
                input,
                ..
            } => *control_generation > 0 && input.validate().is_ok(),
            Self::Close {
                control_generation, ..
            } => *control_generation > 0,
        };
        if valid {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

/// Private native image custody. Deliberately has no serde implementation or byte-printing Debug.
#[derive(Clone, Eq, PartialEq)]
pub struct ComputerScreenshotCapture {
    /// Native-proved selected session root, independently of the actual captured owned modal.
    /// Private provenance only; never a model argument or public screenshot descriptor field.
    pub root_window_id: ComputerWindowId,
    /// Bounded metadata independently verified before release.
    pub descriptor: ComputerScreenshotDescriptor,
    /// Exact PNG frame, never serialized into model arguments or ordinary diagnostics.
    pub png: Vec<u8>,
}

impl std::fmt::Debug for ComputerScreenshotCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComputerScreenshotCapture")
            .field("root_window_id", &self.root_window_id)
            .field("descriptor", &self.descriptor)
            .field("png_bytes", &self.png.len())
            .finish()
    }
}

/// Quarantined native evidence. Runtime policy must authorize any observation or pixels released.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComputerDriverReply {
    /// Bounded observed top-level choices.
    Windows(Vec<ComputerWindowSummary>),
    /// Acknowledged physical writer lease.
    Opened(ComputerSessionSummary),
    /// Current owner-bound native control state.
    Status(ComputerSessionSummary),
    /// Bounded semantics and a fresh native observation.
    Snapshot(ComputerObservation),
    /// Binary target-only PNG in private custody.
    Screenshot(ComputerScreenshotCapture),
    /// Acknowledged balanced input action.
    Applied(ComputerSessionSummary),
    /// Acknowledged automation release; the user's app remains running.
    Closed,
}

/// Categorical native errors never disclose private provider content or operating-system identities.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerDriverError {
    /// Verified ordinary-user interactive host is absent.
    #[error("computer control unavailable")]
    Unavailable,
    /// Another owner holds the interactive-session writer lease.
    #[error("computer control busy")]
    Busy,
    /// Native authority, protected target, or human control rejected the action.
    #[error("computer native boundary denied operation")]
    Denied,
    /// The native operator stopped this exact owned run before dispatch.
    /// This is never inferred from a stale observation, pause, or generic denial.
    #[error("computer control stopped by user")]
    UserStopped,
    /// Native identity, control generation, geometry, focus, or observation changed.
    #[error(
        "computer target or observation is stale; check computer.status and take a fresh snapshot; if the selected observation.window_id has foreground=false in observation.windows, use computer.focus for that observed window, then take a new snapshot before input"
    )]
    Stale,
    /// Closed typed payload failed bounded validation.
    #[error("invalid computer input")]
    InvalidInput,
    /// Failure with a known no-effect outcome.
    #[error("computer native host failed")]
    Failed,
    /// Native mutation or cleanup may have happened; never retry automatically.
    #[error("computer outcome unknown")]
    OutcomeUnknown,
}

/// Owner-scoped supervised normal-user desktop adapter, injected only by trusted composition.
#[async_trait]
pub trait ComputerDriver: Send + Sync {
    /// Verified installed-host availability and absolute native ceilings.
    fn capabilities(&self) -> ComputerCapabilities;

    /// Execute one bounded request using the exact control registered for its immutable run binding.
    /// Trusted callers retain that control across calls and never replace it after cancellation.
    /// Check cancellation under native owner-registry admission before the first insertion, then
    /// before dispatch and while awaiting replies, so cancellation cannot race a new helper owner.
    async fn execute(
        &self,
        request: ComputerDriverRequest,
        control: &RunControl,
    ) -> Result<ComputerDriverReply, ComputerDriverError>;

    /// Synchronously and permanently fence an admitted run before asynchronous cleanup.
    /// The caller first cancels the same registered control supplied to `execute`. A never-dispatched
    /// run needs no native tombstone: cancelled-control admission must fail under the registry lock.
    /// Existing admitted owners remain permanently fenced even after acknowledged helper cleanup.
    fn revoke_run(&self, binding: &ComputerBinding);

    /// Independently acknowledge idempotent helper cleanup, including enumeration-only runs.
    async fn close_run(&self, binding: &ComputerBinding) -> Result<(), ComputerDriverError>;
}
