//! Ratatui terminal interface for the Colossus interactive runtime.
//!
//! This crate owns editing, layout, terminal restoration, overlays, scrolling, and
//! operation scheduling. Product behavior remains behind [`InteractiveHost`].

use async_trait::async_trait;
use colossus_contracts::{
    AgentRunOutcome, ModelMessageRole, ProviderEvent, RunEvent, RunEventEnvelope, SessionMessage,
    SessionMessagePage, TerminalPreferences, ThemeTextStyle,
};
use colossus_ports::RunControl;
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTone, SemanticRenderer,
    StyledDocumentRenderer, TerminalPalette,
};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::{
    collections::{BTreeMap, VecDeque},
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

mod app;
mod contract;
mod render;
mod state;
mod terminal;
mod transcript;

pub use app::run_tui;
pub use contract::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostRunResult, InteractiveCommand,
    InteractiveHost, InteractivePrompt, InteractiveRunRequest, InteractiveSnapshot, LocalCommand,
    OperationResult, PromptResponse, RuntimeCommand, ScreenMode, TranscriptEntry, TranscriptKind,
    TuiError, TuiOptions, parse_interactive_command,
};
pub use state::TuiState;

#[cfg(test)]
use app::*;
use render::*;
use state::*;
use terminal::*;
use transcript::*;

#[cfg(test)]
mod tests;
