//! Ratatui terminal interface for the Colossus interactive runtime.
//!
//! This crate owns editing, layout, terminal restoration, overlays, scrolling, and
//! operation scheduling. Product behavior remains behind [`InteractiveHost`].

use async_trait::async_trait;
use colossus_contracts::{
    AgentRunMode, AgentRunOutcome, ModelContent, ModelContentPart, ModelImageReference,
    ModelMessageRole, PlanDraftTarget, PlanExecutionStrategy, PlanRecord, PlanStatus,
    ProviderEvent, RunEvent, RunEventEnvelope, SandboxBoundaryMode, SecurityPostureReport,
    SessionMessage, SessionMessagePage, SessionSummary, TerminalPreferences, ThemeTextStyle,
};
use colossus_ports::RunControl;
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTone, SemanticRenderer,
    StyledDocumentRenderer, TerminalPalette,
};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, ClearType, CrosstermBackend},
    buffer::{Buffer, Cell},
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, StatefulWidget, Widget, Wrap},
};
use ratatui_image::{
    Resize, StatefulImage,
    picker::{Picker, ProtocolType},
    thread::{ResizeRequest, ResizeResponse, ThreadProtocol},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{self, IsTerminal as _, Stdout, Write as _},
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Maximum number of future turns accepted while one operation is running.
pub const MAX_QUEUED_TURNS: usize = 8;
/// Maximum number of canonical messages loaded in one transcript page.
pub const MAX_TRANSCRIPT_PAGE_MESSAGES: usize = 100;
/// Maximum decoded bytes loaded in one transcript page.
pub const MAX_TRANSCRIPT_PAGE_BYTES: usize = 2 * 1024 * 1024;
/// Smallest viewport that can safely show transcript, composer, and footer.
pub const MINIMUM_TERMINAL_WIDTH: u16 = 40;
/// Smallest viewport that can safely show transcript, composer, and footer.
pub const MINIMUM_TERMINAL_HEIGHT: u16 = 12;
/// Most completion rows shown before the suggestion menu scrolls.
const MAX_COMPLETION_MENU_ROWS: usize = 6;
/// Transcript rows retained above completion chrome so the menu remains renderable.
const MINIMUM_COMPLETION_TRANSCRIPT_ROWS: u16 = 3;
/// Most rows occupied by the bottom-docked effect approval surface.
const MAX_APPROVAL_DOCK_ROWS: u16 = 10;
/// Fewest rows that keep approval summary, navigation, and decisions usable.
const MIN_APPROVAL_DOCK_ROWS: u16 = 8;
/// Transcript rows retained above a bottom-docked approval.
const MINIMUM_APPROVAL_TRANSCRIPT_ROWS: u16 = 3;
/// Most rows occupied by the contextual plan-execution decision dock.
const MAX_PLAN_EXECUTION_DOCK_ROWS: u16 = 11;
/// Fewest rows that keep plan context, choices, and confirmation usable.
const MIN_PLAN_EXECUTION_DOCK_ROWS: u16 = 11;
/// Number of transcript lines moved by one terminal mouse-wheel event.
const MOUSE_SCROLL_LINES: usize = 3;
/// Smallest inline viewport: the composer and status footer, with no reserved transcript gap.
const MINIMUM_INLINE_VIEWPORT_HEIGHT: u16 = 4;
/// Maximum older durable pages eagerly restored into native terminal scrollback.
const MAX_NATIVE_HISTORY_PAGES: usize = 10;
/// Maximum durable messages restored into native scrollback, including the bootstrap page.
const MAX_NATIVE_HISTORY_MESSAGES: usize =
    (MAX_NATIVE_HISTORY_PAGES + 1) * MAX_TRANSCRIPT_PAGE_MESSAGES;
/// Maximum rendered rows inserted into terminal scrollback in one operation.
const HISTORY_INSERT_CHUNK_LINES: usize = 1_024;

mod app;
pub use app::{sandbox_boundary_acknowledgement_choice, sandbox_boundary_prompt};
mod completion;
mod contract;
mod plan_execution;
mod preview;
mod render;
mod session_browser;
mod state;
mod terminal;
mod theme_picker;
mod transcript;

pub use app::run_tui;
pub use contract::{
    AttachmentDetach, BackgroundNoticeProvider, BootstrapRequest, FooterState, HostCommandResult,
    HostEvent, HostPlanExecutionOutcome, HostPlanExecutionResult, HostRunResult,
    InteractiveApprovalMode, InteractiveCommand, InteractiveHost, InteractiveMode,
    InteractivePlanExecutionRequest, InteractivePrompt, InteractivePromptKind,
    InteractiveRunRequest, InteractiveSessionBrowser, InteractiveSessionBrowserEntry,
    InteractiveSessionBrowserMessage, InteractiveSnapshot, InteractiveThemePicker,
    InteractiveThemePickerEntry, LocalCommand, OperationResult, PlanCommand, PlanHostCommand,
    PlanSelectionUpdate, PromptResponse, ResearchCommand, RuntimeCommand, ScreenMode,
    TranscriptEntry, TranscriptKind, TuiError, TuiOptions, parse_interactive_command,
};
pub use state::TuiState;

use completion::*;
use plan_execution::*;
use preview::*;
use session_browser::*;
use theme_picker::*;

#[cfg(test)]
use app::*;
use render::*;
use state::*;
use terminal::*;
use transcript::*;

#[cfg(test)]
mod tests;
