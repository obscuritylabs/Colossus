//! Explicit human terminal authority, independent of agent execution.
use serde::{Deserialize, Serialize};

/// Application scope required for every human terminal operation.
pub const HUMAN_TERMINAL_SCOPE: &str = "terminals:control";
/// Additive authenticated terminal protocol capability.
pub const HUMAN_TERMINAL_CAPABILITY: &str = "human_terminal.v1";

/// One request beneath a native application's terminal grant.
/// The trusted application binds `owner`; a browser host must replace any supplied owner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanTerminalRequest {
    /// Application-local opaque human ownership partition.
    pub owner: String,
    /// Closed terminal operation; never an arbitrary executable or path.
    pub operation: HumanTerminalOperation,
}

/// Closed operations on fixed native shells.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum HumanTerminalOperation {
    Context,
    Open {
        request_id: String,
        controller_id: String,
        rows: u16,
        cols: u16,
    },
    Attach {
        terminal_id: String,
        controller_id: String,
        takeover: bool,
    },
    Read {
        terminal_id: String,
        after_sequence: u64,
        max_bytes: u32,
        wait_ms: u32,
    },
    Write {
        terminal_id: String,
        writer_epoch: u64,
        input_sequence: u64,
        data_base64: String,
    },
    Resize {
        terminal_id: String,
        writer_epoch: u64,
        rows: u16,
        cols: u16,
    },
    Signal {
        terminal_id: String,
        writer_epoch: u64,
        signal: HumanTerminalSignal,
    },
    Close {
        terminal_id: String,
        writer_epoch: u64,
    },
}

/// Native process controls available to the human controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanTerminalSignal {
    /// Interrupt the supervised process group.
    Interrupt,
    /// End the supervised process group.
    Terminate,
}

/// Bounded response; no native paths, process identifiers or credentials.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanTerminalReply {
    /// Both native host opt-in and application authority permit terminals.
    pub available: bool,
    /// Process incarnation; changes whenever the terminal owner restarts.
    pub incarnation: String,
    /// At most eight sessions in this human's application partition.
    pub sessions: Vec<HumanTerminalSession>,
    /// Exact selected session when applicable.
    pub session: Option<HumanTerminalSession>,
    /// Ordered bounded display stream; never input replay.
    pub frames: Vec<HumanTerminalFrame>,
    /// Exclusive cursor for the next read.
    pub next_sequence: u64,
    /// Requested history is absent; a fresh emulator cannot safely consume a suffix.
    pub gap: bool,
    /// Last confirmed input sequence for the current writer epoch.
    pub input_sequence: Option<u64>,
}

/// Safe ownership and lifecycle metadata for one process-local terminal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct HumanTerminalSession {
    pub terminal_id: String,
    pub writer_epoch: u64,
    pub controller_id: String,
    pub rows: u16,
    pub cols: u16,
    pub status: HumanTerminalStatus,
    pub first_sequence: u64,
    /// Last published sequence, usable as an exclusive read cursor.
    pub next_sequence: u64,
    pub complete_history: bool,
    pub exit_code: Option<u32>,
}

/// Process-local lifecycle; terminated sessions are never restarted implicitly.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanTerminalStatus {
    /// Native process is supervised and available for attachment.
    Running,
    /// Confirmed process exit.
    Exited,
    /// Native owner could not confirm normal completion.
    Interrupted,
}

/// One ordered output or size event. Sequences begin at one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanTerminalFrame {
    /// Monotonic session-local display cursor.
    pub sequence: u64,
    /// Bytes or dimensions, applied in this exact order.
    pub kind: HumanTerminalFrameKind,
}

/// Display-only replay data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum HumanTerminalFrameKind {
    Output { data_base64: String },
    Resize { rows: u16, cols: u16 },
}

impl HumanTerminalRequest {
    /// Validate envelopes before native admission or cross-host forwarding.
    pub fn validate(&self) -> bool {
        fn id(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        }
        fn size(rows: u16, cols: u16) -> bool {
            (2..=512).contains(&rows) && (2..=512).contains(&cols)
        }
        if !id(&self.owner) {
            return false;
        }
        match &self.operation {
            HumanTerminalOperation::Context => true,
            HumanTerminalOperation::Open {
                request_id,
                controller_id,
                rows,
                cols,
            } => id(request_id) && id(controller_id) && size(*rows, *cols),
            HumanTerminalOperation::Attach {
                terminal_id,
                controller_id,
                ..
            } => id(terminal_id) && id(controller_id),
            HumanTerminalOperation::Read {
                terminal_id,
                max_bytes,
                wait_ms,
                ..
            } => id(terminal_id) && (16384..=65536).contains(max_bytes) && *wait_ms <= 1000,
            HumanTerminalOperation::Write {
                terminal_id,
                input_sequence,
                data_base64,
                ..
            } => id(terminal_id) && *input_sequence > 0 && data_base64.len() <= 87384,
            HumanTerminalOperation::Resize {
                terminal_id,
                rows,
                cols,
                ..
            } => id(terminal_id) && size(*rows, *cols),
            HumanTerminalOperation::Signal { terminal_id, .. }
            | HumanTerminalOperation::Close { terminal_id, .. } => id(terminal_id),
        }
    }
}

impl HumanTerminalReply {
    /// Check collection, cursor and display bounds at transport consumption boundaries.
    pub fn validate(&self) -> bool {
        fn session(value: &HumanTerminalSession) -> bool {
            !value.terminal_id.is_empty()
                && value.terminal_id.len() <= 128
                && value.controller_id.len() <= 128
                && value.writer_epoch > 0
                && (2..=512).contains(&value.rows)
                && (2..=512).contains(&value.cols)
                && value.first_sequence <= value.next_sequence.saturating_add(1)
                && (!value.complete_history || value.first_sequence == 1)
        }
        if self.incarnation.len() > 128
            || self.sessions.len() > 8
            || self.frames.len() > 256
            || self.sessions.iter().any(|v| !session(v))
            || self.session.as_ref().is_some_and(|v| !session(v))
        {
            return false;
        }
        let mut sequence = None;
        self.frames.iter().all(|frame| {
            if frame.sequence == 0
                || sequence.is_some_and(|old: u64| old.checked_add(1) != Some(frame.sequence))
            {
                return false;
            }
            sequence = Some(frame.sequence);
            match &frame.kind {
                HumanTerminalFrameKind::Output { data_base64 } => data_base64.len() <= 10924,
                HumanTerminalFrameKind::Resize { rows, cols } => {
                    (2..=512).contains(rows) && (2..=512).contains(cols)
                }
            }
        }) && self
            .frames
            .last()
            .is_none_or(|frame| frame.sequence == self.next_sequence)
    }
}
