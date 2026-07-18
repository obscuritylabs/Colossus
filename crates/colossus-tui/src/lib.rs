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

/// TUI startup and terminal-ownership failure.
#[derive(Debug, Error)]
pub enum TuiError {
    /// The host failed before or during an interactive operation.
    #[error("interactive host failed: {0}")]
    Host(String),
    /// Crossterm or Ratatui could not own or restore the terminal.
    #[error("terminal operation failed: {0}")]
    Terminal(#[from] io::Error),
    /// TUI launch requires an interactive stdin/stdout pair.
    #[error("the terminal UI requires interactive stdin and stdout")]
    NotInteractive,
}

/// Startup selection for a durable session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BootstrapRequest {
    /// Attach to this exact durable session.
    pub session_id: Option<String>,
    /// Attach to the newest durable session when no exact id was supplied.
    pub resume_latest: bool,
}

/// Cached stable footer data, refreshed only after relevant host mutations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FooterState {
    /// Configured model role.
    pub role: String,
    /// Resolved model/provider label.
    pub route: String,
    /// Current used and maximum provider context tokens.
    pub context: Option<(u64, u64)>,
    /// Canonical visible message count.
    pub message_count: u64,
    /// Short readiness or terminal run status.
    pub status: String,
    /// Active approval mode.
    pub approval_mode: String,
}

/// Fully bounded state needed before terminal ownership begins.
#[derive(Clone, Debug)]
pub struct InteractiveSnapshot {
    /// Exact active durable session.
    pub session_id: String,
    /// Newest bounded canonical transcript page.
    pub transcript: SessionMessagePage,
    /// Persisted rendering and editing preferences.
    pub preferences: TerminalPreferences,
    /// Newest encrypted submitted-input history in chronological order.
    pub history: Vec<String>,
    /// Commands, skills, and theme names eligible for completion.
    pub completions: Vec<String>,
    /// Cached stable footer state.
    pub footer: FooterState,
}

/// Request for one normal provider/tool turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveRunRequest {
    /// Exact durable session.
    pub session_id: String,
    /// Complete user prompt after local skill-mention parsing.
    pub prompt: String,
    /// Explicit skills activated by this prompt.
    pub explicit_skills: Vec<String>,
    /// Sticky skills active in the terminal.
    pub sticky_skills: Vec<String>,
}

/// One parsed terminal command whose behavior belongs to the application host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeCommand {
    /// A known slash command and its bounded original arguments.
    Known {
        /// Stable command name without the slash.
        name: String,
        /// Remaining command text after the name.
        arguments: String,
    },
}

/// One terminal-local command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalCommand {
    /// Close the TUI while idle.
    Exit,
    /// Show stateful interactive help.
    Help,
    /// Show current terminal preferences.
    Preferences,
    /// Persist the current terminal preferences.
    SavePreferences,
    /// Restore and persist default terminal preferences.
    ResetPreferences,
}

/// Result of parsing a submitted interactive line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractiveCommand {
    /// Empty input has no effect.
    Empty,
    /// Interface-only behavior.
    Local(LocalCommand),
    /// Application behavior executed by the host.
    Runtime(RuntimeCommand),
    /// Normal model turn.
    Turn(String),
}

/// Parse terminal input once for both embedded and worker-backed hosts.
pub fn parse_interactive_command(input: &str) -> InteractiveCommand {
    let input = input.trim();
    if input.is_empty() {
        return InteractiveCommand::Empty;
    }
    match input {
        "/exit" | "/quit" => InteractiveCommand::Local(LocalCommand::Exit),
        "/help" => InteractiveCommand::Local(LocalCommand::Help),
        "/tui" | "/tui prefs" => InteractiveCommand::Local(LocalCommand::Preferences),
        "/tui save" => InteractiveCommand::Local(LocalCommand::SavePreferences),
        "/tui reset" => InteractiveCommand::Local(LocalCommand::ResetPreferences),
        command if command.starts_with('/') => {
            let command = command.trim_start_matches('/');
            let (name, arguments) = command.split_once(' ').unwrap_or((command, ""));
            InteractiveCommand::Runtime(RuntimeCommand::Known {
                name: name.to_owned(),
                arguments: arguments.trim().to_owned(),
            })
        }
        prompt => InteractiveCommand::Turn(prompt.to_owned()),
    }
}

/// Result of one host-owned interactive command.
#[derive(Clone, Debug)]
pub struct HostCommandResult {
    /// Human presentation to append to the transcript.
    pub document: PresentationDocument,
    /// New active session and its newest page after a session switch.
    pub session: Option<(String, SessionMessagePage)>,
    /// Updated preferences when the command changed presentation state.
    pub preferences: Option<TerminalPreferences>,
    /// Updated completion catalog when host state changed.
    pub completions: Option<Vec<String>>,
    /// Updated sticky declarative skills when changed by a command.
    pub sticky_skills: Option<Vec<String>>,
    /// Updated cached footer only when relevant state changed.
    pub footer: Option<FooterState>,
    /// Clear visible transcript entries after a local clear command.
    pub clear_transcript: bool,
}

impl HostCommandResult {
    /// Create a transcript-only command result.
    pub fn document(document: PresentationDocument) -> Self {
        Self {
            document,
            session: None,
            preferences: None,
            completions: None,
            sticky_skills: None,
            footer: None,
            clear_transcript: false,
        }
    }
}

/// Controlled run outcome plus its post-run cached footer refresh.
#[derive(Clone, Debug)]
pub struct HostRunResult {
    /// Durable success or cooperative cancellation evidence.
    pub outcome: AgentRunOutcome,
    /// Footer state refreshed after the run reached a terminal state.
    pub footer: FooterState,
}

/// One-use response returned from an approval or `user.ask` overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptResponse {
    /// Operator supplied a bounded answer.
    Answer(String),
    /// Operator cancelled or submitted a blank answer.
    Cancelled,
}

/// Focus-taking prompt sent by the trusted runtime bridge to the TUI.
pub struct InteractivePrompt {
    /// One-use prompt identity bound by the host to the connection and run.
    pub id: String,
    /// Short overlay title.
    pub title: String,
    /// Policy-released prompt details.
    pub document: PresentationDocument,
    /// Optional exact choices.
    pub choices: Vec<String>,
    /// Choice highlighted when the prompt opens; `None` preserves blank-submit cancellation.
    pub initial_choice: Option<usize>,
    /// Whether an answer outside the exact choices is allowed.
    pub allow_free_form: bool,
    /// One-use response channel. Dropping it fails closed.
    pub response: oneshot::Sender<PromptResponse>,
}

/// Typed background event consumed by the sole terminal owner.
pub enum HostEvent {
    /// Ordered policy-released agent runtime event.
    Run(RunEventEnvelope),
    /// A trusted bridge needs focused operator input.
    Prompt(InteractivePrompt),
    /// The current operation reached a terminal result.
    OperationFinished(Box<Result<OperationResult, String>>),
    /// Non-fatal history persistence failed after the requested operation began.
    HistoryWarning(String),
    /// An asynchronously requested older transcript page completed.
    OlderPage(Result<SessionMessagePage, String>),
}

/// Terminal result of one serialized background operation.
pub enum OperationResult {
    /// Application command completed.
    Command(HostCommandResult),
    /// Model run completed or was cooperatively cancelled.
    Run(HostRunResult),
}

/// Embedded and worker-backed application boundary consumed by the TUI.
#[async_trait]
pub trait InteractiveHost: Send + Sync {
    /// Resolve session, transcript, preferences, history, completions, and footer.
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String>;

    /// Execute one typed application command without writing to the terminal.
    async fn execute_command(
        &self,
        command: RuntimeCommand,
        session_id: &str,
        sticky_skills: &[String],
        events: mpsc::Sender<HostEvent>,
    ) -> Result<HostCommandResult, String>;

    /// Execute one controlled model turn and emit ordered policy-released events.
    async fn run_turn(
        &self,
        request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostRunResult, String>;

    /// Persist one submitted input through the encrypted presentation repository.
    async fn append_history(&self, entry: String) -> Result<(), String>;

    /// Persist one exact terminal preference snapshot.
    async fn save_preferences(
        &self,
        preferences: TerminalPreferences,
    ) -> Result<TerminalPreferences, String>;

    /// Load the next older bounded transcript page.
    async fn older_messages(
        &self,
        session_id: &str,
        before_sequence: u64,
    ) -> Result<SessionMessagePage, String>;
}

/// Terminal viewport selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScreenMode {
    /// Full alternate screen with native scrollback protected.
    #[default]
    Alternate,
    /// Ratatui inline viewport, preserving terminal scrollback.
    Inline,
}

/// User-visible TUI startup options.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiOptions {
    /// Durable session selection.
    pub bootstrap: BootstrapRequest,
    /// Explicit screen mode. Zellij automatically selects inline mode.
    pub screen_mode: ScreenMode,
}

/// Semantic transcript provenance used for layout and color selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptKind {
    /// Canonical operator input.
    User,
    /// Canonical assistant output.
    Assistant,
    /// Canonical or live tool activity.
    Tool,
    /// Local or application command output.
    Command,
    /// Recoverable or terminal failure.
    Error,
}

/// Retained semantic transcript entry reflowed on every resize.
#[derive(Clone, Debug)]
pub struct TranscriptEntry {
    /// Canonical sequence when restored from a session.
    pub sequence: Option<u64>,
    /// Semantic provenance.
    pub kind: TranscriptKind,
    /// Original presentation document retained for resize reflow.
    pub document: PresentationDocument,
    /// Whether provider deltas may replace this entry.
    pub temporary: bool,
}

#[derive(Debug, Default)]
struct Composer {
    draft: String,
    cursor: usize,
    history_index: Option<usize>,
    completion_index: Option<usize>,
    completion_hidden: bool,
}

impl Composer {
    fn insert(&mut self, text: &str) {
        self.draft.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.reset_navigation();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = previous_boundary(&self.draft, self.cursor);
        self.draft.drain(previous..self.cursor);
        self.cursor = previous;
        self.reset_navigation();
    }

    fn delete(&mut self) {
        if self.cursor == self.draft.len() {
            return;
        }
        let next = next_boundary(&self.draft, self.cursor);
        self.draft.drain(self.cursor..next);
        self.reset_navigation();
    }

    fn move_left(&mut self) {
        self.cursor = previous_boundary(&self.draft, self.cursor);
        self.completion_index = None;
    }

    fn move_right(&mut self) {
        self.cursor = next_boundary(&self.draft, self.cursor);
        self.completion_index = None;
    }

    fn take(&mut self) -> String {
        self.cursor = 0;
        self.history_index = None;
        self.completion_index = None;
        self.completion_hidden = false;
        std::mem::take(&mut self.draft)
    }

    fn clear(&mut self) {
        self.draft.clear();
        self.cursor = 0;
        self.reset_navigation();
    }

    fn set(&mut self, value: String) {
        self.cursor = value.len();
        self.draft = value;
        self.completion_index = None;
        self.completion_hidden = true;
    }

    fn reset_navigation(&mut self) {
        self.history_index = None;
        self.completion_index = None;
        self.completion_hidden = false;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionKind {
    Command,
    Skill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompletionContext<'a> {
    prefix: &'a str,
    kind: CompletionKind,
}

enum Overlay {
    Prompt {
        request: InteractivePrompt,
        input: String,
        selected: Option<usize>,
    },
    HistorySearch {
        query: String,
    },
    QueuePaused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationKind {
    Command,
    Run,
}

/// Pure reducer state used by the terminal loop and `TestBackend` fixtures.
pub struct TuiState {
    /// Exact active durable session.
    pub session_id: String,
    /// Retained visible transcript; system messages are never inserted.
    pub transcript: Vec<TranscriptEntry>,
    /// Whether an older canonical page remains available.
    pub has_more: bool,
    /// Cursor for the next older page.
    pub before_sequence: Option<u64>,
    /// Persisted presentation preferences.
    pub preferences: TerminalPreferences,
    /// Cached stable footer state.
    pub footer: FooterState,
    composer: Composer,
    history: Vec<String>,
    completions: Vec<String>,
    sticky_skills: Vec<String>,
    active_calls: BTreeMap<String, colossus_contracts::ToolCall>,
    queue: VecDeque<String>,
    queue_paused: bool,
    operation: Option<OperationKind>,
    control: Option<RunControl>,
    overlay: Option<Overlay>,
    activity: Option<String>,
    started_at: Option<Instant>,
    scroll_from_bottom: usize,
    new_items: usize,
    transcript_height: usize,
    transcript_width: usize,
    loading_older: bool,
    should_exit: bool,
}

impl TuiState {
    /// Build reducer state from one bounded host snapshot.
    pub fn from_snapshot(snapshot: InteractiveSnapshot) -> Self {
        let transcript = transcript_from_messages(snapshot.transcript.messages);
        Self {
            session_id: snapshot.session_id,
            transcript,
            has_more: snapshot.transcript.has_more,
            before_sequence: snapshot.transcript.before_sequence,
            preferences: snapshot.preferences,
            footer: snapshot.footer,
            composer: Composer::default(),
            history: snapshot.history,
            completions: snapshot.completions,
            sticky_skills: Vec::new(),
            active_calls: BTreeMap::new(),
            queue: VecDeque::new(),
            queue_paused: false,
            operation: None,
            control: None,
            overlay: None,
            activity: None,
            started_at: None,
            scroll_from_bottom: 0,
            new_items: 0,
            transcript_height: 1,
            transcript_width: 80,
            loading_older: false,
            should_exit: false,
        }
    }

    /// Current editable draft, excluding type-ahead ghost text.
    pub fn draft(&self) -> &str {
        &self.composer.draft
    }

    /// UTF-8 byte cursor, always on a character boundary.
    pub const fn cursor(&self) -> usize {
        self.composer.cursor
    }

    /// Number of queued future turns.
    pub fn queued_turns(&self) -> usize {
        self.queue.len()
    }

    /// Whether a serialized command or run is active.
    pub const fn is_busy(&self) -> bool {
        self.operation.is_some()
    }

    /// Append an older page without duplicating or exposing system messages.
    pub fn prepend_page(&mut self, page: SessionMessagePage) {
        let mut older = transcript_from_messages(page.messages);
        older.append(&mut self.transcript);
        self.transcript = older;
        self.has_more = page.has_more;
        self.before_sequence = page.before_sequence;
    }

    /// Scroll the durable transcript upward by one viewport.
    pub fn page_up(&mut self) {
        self.scroll_from_bottom = self
            .scroll_from_bottom
            .saturating_add(self.transcript_height.max(1));
    }

    /// Scroll toward live output by one viewport.
    pub fn page_down(&mut self) {
        self.scroll_from_bottom = self
            .scroll_from_bottom
            .saturating_sub(self.transcript_height.max(1));
        if self.scroll_from_bottom == 0 {
            self.new_items = 0;
        }
    }

    /// Return to live output and clear the new-item badge.
    pub fn end(&mut self) {
        self.scroll_from_bottom = 0;
        self.new_items = 0;
    }

    fn append_entry(&mut self, entry: TranscriptEntry) {
        let old_line_count = if self.scroll_from_bottom > 0 {
            transcript_lines(self, self.transcript_width).len()
        } else {
            0
        };
        if self.scroll_from_bottom > 0 {
            self.new_items = self.new_items.saturating_add(1);
        }
        self.transcript.push(entry);
        if self.scroll_from_bottom > 0 {
            self.preserve_scroll_after_line_change(old_line_count);
        }
    }

    fn preserve_scroll_after_line_change(&mut self, old_line_count: usize) {
        let new_line_count = transcript_lines(self, self.transcript_width).len();
        if new_line_count >= old_line_count {
            self.scroll_from_bottom = self
                .scroll_from_bottom
                .saturating_add(new_line_count - old_line_count);
        } else {
            self.scroll_from_bottom = self
                .scroll_from_bottom
                .saturating_sub(old_line_count - new_line_count);
        }
    }

    fn structured_completion_context(&self) -> Option<CompletionContext<'_>> {
        if self.composer.completion_hidden
            || self.composer.cursor != self.composer.draft.len()
            || self.composer.draft.is_empty()
        {
            return None;
        }
        if self.composer.draft.starts_with('/') {
            return Some(CompletionContext {
                prefix: &self.composer.draft,
                kind: CompletionKind::Command,
            });
        }
        let token_start = self
            .composer
            .draft
            .char_indices()
            .rev()
            .find(|(_, character)| character.is_whitespace())
            .map_or(0, |(index, character)| index + character.len_utf8());
        let token = &self.composer.draft[token_start..];
        token.starts_with('@').then_some(CompletionContext {
            prefix: token,
            kind: CompletionKind::Skill,
        })
    }

    fn completion_menu_candidates(&self) -> Vec<&str> {
        let Some(context) = self.structured_completion_context() else {
            return Vec::new();
        };
        self.completions
            .iter()
            .map(String::as_str)
            .filter(|candidate| {
                candidate.starts_with(context.prefix) && *candidate != context.prefix
            })
            .collect()
    }

    fn completion_candidates(&self) -> Vec<&str> {
        if self.composer.completion_hidden
            || self.composer.cursor != self.composer.draft.len()
            || self.composer.draft.is_empty()
        {
            return Vec::new();
        }
        if self.structured_completion_context().is_some() {
            return self.completion_menu_candidates();
        }
        self.completions
            .iter()
            .map(String::as_str)
            .chain(self.history.iter().rev().map(String::as_str))
            .filter(|candidate| {
                candidate.starts_with(&self.composer.draft)
                    && *candidate != self.composer.draft.as_str()
            })
            .collect()
    }

    fn ghost_text(&self) -> Option<&str> {
        let candidates = self.completion_candidates();
        let index = self.composer.completion_index.unwrap_or(0) % candidates.len().max(1);
        let prefix = self
            .structured_completion_context()
            .map_or(self.composer.draft.as_str(), |context| context.prefix);
        candidates
            .get(index)
            .and_then(|candidate| candidate.strip_prefix(prefix))
    }

    fn advance_completion(&mut self) {
        let count = self.completion_candidates().len();
        if count == 0 {
            self.composer.completion_index = None;
        } else {
            self.composer.completion_index = Some(
                self.composer
                    .completion_index
                    .map_or(0, |index| (index + 1) % count),
            );
        }
    }

    fn previous_completion(&mut self) {
        let count = self.completion_candidates().len();
        if count == 0 {
            self.composer.completion_index = None;
        } else {
            self.composer.completion_index = Some(
                self.composer
                    .completion_index
                    .map_or(count - 1, |index| (index + count - 1) % count),
            );
        }
    }

    fn accept_completion(&mut self) -> bool {
        let kind = self
            .structured_completion_context()
            .map(|context| context.kind);
        let Some(suffix) = self.ghost_text().map(str::to_owned) else {
            return false;
        };
        self.composer.insert(&suffix);
        if kind == Some(CompletionKind::Skill)
            && !self
                .composer
                .draft
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            self.composer.insert(" ");
        }
        true
    }

    fn hide_completion(&mut self) -> bool {
        if self.completion_menu_candidates().is_empty() {
            return false;
        }
        self.composer.completion_index = None;
        self.composer.completion_hidden = true;
        true
    }

    fn previous_history(&mut self) {
        if self.composer.cursor != 0 || self.history.is_empty() {
            return;
        }
        let index = self
            .composer
            .history_index
            .map_or(self.history.len() - 1, |index| index.saturating_sub(1));
        self.composer.history_index = Some(index);
        self.composer.set(self.history[index].clone());
        self.composer.history_index = Some(index);
    }

    fn next_history(&mut self) {
        if self.composer.cursor != self.composer.draft.len() {
            return;
        }
        let Some(index) = self.composer.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.composer.set(self.history[index + 1].clone());
            self.composer.history_index = Some(index + 1);
        } else {
            self.composer.clear();
        }
    }

    fn remember_history(&mut self, entry: &str) {
        if self.history.last().is_some_and(|last| last == entry) {
            return;
        }
        self.history.push(entry.to_owned());
        if self.history.len() > 1_000 {
            let excess = self.history.len() - 1_000;
            self.history.drain(..excess);
        }
    }

    fn cancel_focus(&mut self) -> bool {
        if let Some(overlay) = self.overlay.take() {
            if let Overlay::Prompt { request, .. } = overlay {
                let _ = request.response.send(PromptResponse::Cancelled);
            }
            return true;
        }
        if !self.composer.draft.is_empty() {
            self.composer.clear();
            return true;
        }
        if let Some(control) = &self.control {
            control.cancel();
            self.activity = Some("cancelling after the current effect settles".into());
            return true;
        }
        false
    }
}

/// Launch the terminal UI and retain exclusive ownership of all terminal writes.
pub async fn run_tui(
    host: Arc<dyn InteractiveHost>,
    mut options: TuiOptions,
) -> Result<(), TuiError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(TuiError::NotInteractive);
    }
    if std::env::var_os("ZELLIJ").is_some() {
        options.screen_mode = ScreenMode::Inline;
    }
    let snapshot = host
        .bootstrap(options.bootstrap)
        .await
        .map_err(TuiError::Host)?;
    let mut state = TuiState::from_snapshot(snapshot);
    let (event_tx, mut event_rx) = mpsc::channel::<HostEvent>(256);
    let mut terminal = OwnedTerminal::new(options.screen_mode)?;

    loop {
        terminal.draw(&mut state)?;
        while let Ok(host_event) = event_rx.try_recv() {
            handle_host_event(&mut state, host_event);
        }
        if !state.is_busy()
            && !state.queue_paused
            && state.overlay.is_none()
            && let Some(line) = state.queue.pop_front()
        {
            start_line(&mut state, line, Arc::clone(&host), event_tx.clone());
        }
        if state.should_exit {
            break;
        }
        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(&mut state, key, Arc::clone(&host), event_tx.clone());
                }
                Event::Paste(text) => insert_active_text(&mut state, &text),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

fn handle_key(
    state: &mut TuiState,
    key: KeyEvent,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.cancel_focus();
        return;
    }
    if state.overlay.is_some() {
        handle_overlay_key(state, key);
        return;
    }
    match key.code {
        KeyCode::Esc => {
            state.hide_completion();
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !state.is_busy() && state.composer.draft.is_empty() {
                state.should_exit = true;
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.overlay = Some(Overlay::HistorySearch {
                query: String::new(),
            });
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.composer.cursor = 0;
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.composer.cursor = state.composer.draft.len();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            state.composer.insert(&character.to_string());
        }
        KeyCode::Backspace => state.composer.backspace(),
        KeyCode::Delete => state.composer.delete(),
        KeyCode::Left => state.composer.move_left(),
        KeyCode::Right => {
            if state.composer.cursor == state.composer.draft.len() && state.accept_completion() {
                return;
            }
            state.composer.move_right();
        }
        KeyCode::Home => state.composer.cursor = 0,
        KeyCode::End => {
            if state.composer.cursor == state.composer.draft.len() {
                state.end();
            } else {
                state.composer.cursor = state.composer.draft.len();
            }
        }
        KeyCode::Up if !state.completion_menu_candidates().is_empty() => {
            state.previous_completion();
        }
        KeyCode::Down if !state.completion_menu_candidates().is_empty() => {
            state.advance_completion();
        }
        KeyCode::Up => state.previous_history(),
        KeyCode::Down => state.next_history(),
        KeyCode::Tab => state.advance_completion(),
        KeyCode::BackTab => state.previous_completion(),
        KeyCode::PageUp => {
            state.page_up();
            request_older_page(state, host, event_tx);
        }
        KeyCode::PageDown => state.page_down(),
        KeyCode::Enter => {
            if state.composer.completion_index.is_some() && state.accept_completion() {
                return;
            }
            let submit = !state.preferences.multiline
                || key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL);
            if submit {
                let line = state.composer.take();
                submit_line(state, line, host, event_tx);
            } else {
                state.composer.insert("\n");
            }
        }
        _ => {}
    }
}

fn handle_overlay_key(state: &mut TuiState, key: KeyEvent) {
    if key.code == KeyCode::Esc {
        state.cancel_focus();
        return;
    }
    let Some(overlay) = state.overlay.as_mut() else {
        return;
    };
    match overlay {
        Overlay::Prompt {
            request,
            input,
            selected,
        } => match key.code {
            KeyCode::Enter => {
                let overlay = state.overlay.take();
                if let Some(Overlay::Prompt {
                    request,
                    input,
                    selected,
                }) = overlay
                {
                    let answer = input.trim();
                    let response = if answer.is_empty() {
                        selected
                            .and_then(|index| request.choices.get(index))
                            .cloned()
                            .map(PromptResponse::Answer)
                            .unwrap_or(PromptResponse::Cancelled)
                    } else if let Ok(index) = answer.parse::<usize>() {
                        request
                            .choices
                            .get(index.saturating_sub(1))
                            .cloned()
                            .map(PromptResponse::Answer)
                            .unwrap_or(PromptResponse::Cancelled)
                    } else if request.allow_free_form
                        || request.choices.iter().any(|choice| choice == answer)
                    {
                        PromptResponse::Answer(answer.to_owned())
                    } else {
                        PromptResponse::Cancelled
                    };
                    let _ = request.response.send(response);
                }
            }
            KeyCode::Up | KeyCode::BackTab if !request.choices.is_empty() => {
                let current = selected.unwrap_or(0);
                *selected = Some(if current == 0 {
                    request.choices.len() - 1
                } else {
                    current - 1
                });
            }
            KeyCode::Down | KeyCode::Tab if !request.choices.is_empty() => {
                *selected =
                    Some(selected.map_or(0, |current| (current + 1) % request.choices.len()));
            }
            KeyCode::Home if !request.choices.is_empty() => {
                *selected = Some(0);
            }
            KeyCode::End if !request.choices.is_empty() => {
                *selected = Some(request.choices.len() - 1);
            }
            KeyCode::PageUp if !request.choices.is_empty() => {
                *selected = Some(selected.unwrap_or(0).saturating_sub(5));
            }
            KeyCode::PageDown if !request.choices.is_empty() => {
                *selected = Some(
                    selected
                        .unwrap_or(0)
                        .saturating_add(5)
                        .min(request.choices.len() - 1),
                );
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                input.push(character);
            }
            _ => {}
        },
        Overlay::HistorySearch { query } => match key.code {
            KeyCode::Enter => {
                let selected = state
                    .history
                    .iter()
                    .rev()
                    .find(|entry| entry.contains(query.as_str()))
                    .cloned();
                state.overlay = None;
                if let Some(selected) = selected {
                    state.composer.set(selected);
                }
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                query.push(character);
            }
            _ => {}
        },
        Overlay::QueuePaused => match key.code {
            KeyCode::Char('r' | 'R') | KeyCode::Enter => {
                state.queue_paused = false;
                state.overlay = None;
            }
            KeyCode::Char('c' | 'C') => {
                state.queue.clear();
                state.queue_paused = false;
                state.overlay = None;
            }
            _ => {}
        },
    }
}

fn request_older_page(
    state: &mut TuiState,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    if state.loading_older || !state.has_more {
        return;
    }
    let Some(before_sequence) = state.before_sequence else {
        return;
    };
    state.loading_older = true;
    let session_id = state.session_id.clone();
    tokio::spawn(async move {
        let result = host.older_messages(&session_id, before_sequence).await;
        let _ = event_tx.send(HostEvent::OlderPage(result)).await;
    });
}

fn insert_active_text(state: &mut TuiState, text: &str) {
    let text = sanitize_input(text);
    if let Some(overlay) = state.overlay.as_mut() {
        match overlay {
            Overlay::Prompt { input, .. } | Overlay::HistorySearch { query: input } => {
                input.push_str(&text);
            }
            Overlay::QueuePaused => {}
        }
    } else {
        state.composer.insert(&text);
    }
}

fn submit_line(
    state: &mut TuiState,
    line: String,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    let line = line.trim().to_owned();
    if line.is_empty() {
        return;
    }
    state.remember_history(&line);
    let history_host = Arc::clone(&host);
    let history_tx = event_tx.clone();
    let history_line = line.clone();
    tokio::spawn(async move {
        if let Err(error) = history_host.append_history(history_line).await {
            let _ = history_tx.send(HostEvent::HistoryWarning(error)).await;
        }
    });

    if state.is_busy() {
        if matches!(
            parse_interactive_command(&line),
            InteractiveCommand::Local(LocalCommand::Help | LocalCommand::Preferences)
        ) {
            start_line(state, line, host, event_tx);
            return;
        }
        if state.queue.len() < MAX_QUEUED_TURNS {
            state.queue.push_back(line);
        } else {
            state.append_entry(error_entry("The future-turn queue is full (8 entries)."));
        }
        return;
    }
    start_line(state, line, host, event_tx);
}

fn start_line(
    state: &mut TuiState,
    line: String,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    match parse_interactive_command(&line) {
        InteractiveCommand::Empty => {}
        InteractiveCommand::Local(command) => handle_local_command(state, command, host, event_tx),
        InteractiveCommand::Runtime(command) => {
            state.append_entry(user_entry(&line, TranscriptKind::Command));
            state.operation = Some(OperationKind::Command);
            state.started_at = Some(Instant::now());
            state.activity = Some(format!("running /{}", runtime_command_name(&command)));
            let session_id = state.session_id.clone();
            let sticky_skills = state.sticky_skills.clone();
            let task_tx = event_tx.clone();
            tokio::spawn(async move {
                let result = host
                    .execute_command(command, &session_id, &sticky_skills, task_tx.clone())
                    .await
                    .map(OperationResult::Command);
                let _ = task_tx
                    .send(HostEvent::OperationFinished(Box::new(result)))
                    .await;
            });
        }
        InteractiveCommand::Turn(prompt) => {
            state.append_entry(user_entry(&prompt, TranscriptKind::User));
            state.operation = Some(OperationKind::Run);
            state.started_at = Some(Instant::now());
            state.activity = Some("waiting for model".into());
            let control = RunControl::default();
            state.control = Some(control.clone());
            let request = InteractiveRunRequest {
                session_id: state.session_id.clone(),
                prompt,
                explicit_skills: Vec::new(),
                sticky_skills: state.sticky_skills.clone(),
            };
            let task_tx = event_tx.clone();
            tokio::spawn(async move {
                let result = host
                    .run_turn(request, task_tx.clone(), control)
                    .await
                    .map(OperationResult::Run);
                let _ = task_tx
                    .send(HostEvent::OperationFinished(Box::new(result)))
                    .await;
            });
        }
    }
}

fn handle_local_command(
    state: &mut TuiState,
    command: LocalCommand,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    match command {
        LocalCommand::Exit => state.should_exit = true,
        LocalCommand::Help => state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: help_document(),
            temporary: false,
        }),
        LocalCommand::Preferences => state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: preferences_document(&state.preferences),
            temporary: false,
        }),
        LocalCommand::SavePreferences | LocalCommand::ResetPreferences => {
            let preferences = if command == LocalCommand::ResetPreferences {
                TerminalPreferences::default()
            } else {
                state.preferences.clone()
            };
            state.operation = Some(OperationKind::Command);
            state.activity = Some("saving terminal preferences".into());
            let task_tx = event_tx.clone();
            tokio::spawn(async move {
                let result = host.save_preferences(preferences).await.map(|preferences| {
                    OperationResult::Command(HostCommandResult {
                        document: preferences_document(&preferences),
                        session: None,
                        preferences: Some(preferences),
                        completions: None,
                        sticky_skills: None,
                        footer: None,
                        clear_transcript: false,
                    })
                });
                let _ = task_tx
                    .send(HostEvent::OperationFinished(Box::new(result)))
                    .await;
            });
        }
    }
}

fn handle_host_event(state: &mut TuiState, event: HostEvent) {
    match event {
        HostEvent::Run(envelope) => handle_run_event(state, envelope),
        HostEvent::Prompt(request) => {
            if state.overlay.is_some() {
                let _ = request.response.send(PromptResponse::Cancelled);
            } else {
                let selected = request
                    .initial_choice
                    .filter(|index| *index < request.choices.len());
                state.overlay = Some(Overlay::Prompt {
                    request,
                    input: String::new(),
                    selected,
                });
            }
        }
        HostEvent::HistoryWarning(error) => {
            state.append_entry(error_entry(&format!("History was not persisted: {error}")))
        }
        HostEvent::OlderPage(result) => {
            state.loading_older = false;
            match result {
                Ok(page) => state.prepend_page(page),
                Err(error) => state.append_entry(error_entry(&format!(
                    "Older transcript messages could not be loaded: {error}"
                ))),
            }
        }
        HostEvent::OperationFinished(result) => {
            let result = *result;
            state.operation = None;
            state.control = None;
            state.activity = None;
            state.started_at = None;
            let successful = match result {
                Ok(OperationResult::Command(result)) => {
                    apply_command_result(state, result);
                    true
                }
                Ok(OperationResult::Run(HostRunResult {
                    outcome: AgentRunOutcome::Completed { result },
                    footer,
                })) => {
                    finalize_assistant(state, &result.output);
                    state.footer = footer;
                    true
                }
                Ok(OperationResult::Run(HostRunResult {
                    outcome: AgentRunOutcome::Cancelled { .. },
                    footer,
                })) => {
                    state.footer = footer;
                    state.append_entry(TranscriptEntry {
                        sequence: None,
                        kind: TranscriptKind::Command,
                        document: PresentationDocument::from_block(PresentationBlock::Card {
                            title: "Run cancelled".into(),
                            tone: PresentationTone::Warning,
                            body: vec![PresentationBlock::Text(
                                "No new effect will start. Any active effect settled first.".into(),
                            )],
                        }),
                        temporary: false,
                    });
                    false
                }
                Err(error) => {
                    state.footer.status = "error".into();
                    state.append_entry(error_entry(&format!("Operation failed: {error}")));
                    false
                }
            };
            if successful && !state.queue_paused {
                // The event loop starts this on its next key/tick iteration through a
                // synthetic queue drain in `draw`; this avoids re-entrant host spawning.
            } else if !state.queue.is_empty() {
                state.queue_paused = true;
                state.overlay = Some(Overlay::QueuePaused);
            }
        }
    }
}

fn handle_run_event(state: &mut TuiState, envelope: RunEventEnvelope) {
    let event = envelope.event;
    match &event {
        RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text },
        } if state.preferences.stream_mode != colossus_contracts::StreamDisplayMode::Off => {
            update_streaming_assistant(state, text);
            return;
        }
        RunEvent::Provider {
            event: ProviderEvent::FinalOutput { text },
        } => {
            finalize_assistant(state, text);
            return;
        }
        RunEvent::Phase {
            phase,
            action,
            elapsed_seconds,
            ..
        } => {
            state.activity = Some(action.clone().unwrap_or_else(|| {
                format!(
                    "{} ({elapsed_seconds:.1}s)",
                    format!("{phase:?}").to_lowercase()
                )
            }));
            return;
        }
        RunEvent::ToolStarted { call, .. } => {
            state.activity = Some(format!("running {}", call.name));
            state
                .active_calls
                .insert(call.call_id.clone(), call.clone());
            return;
        }
        _ => {}
    }

    let (kind, call) = match &event {
        RunEvent::ToolCompleted { result, .. } => (
            TranscriptKind::Tool,
            state.active_calls.remove(&result.call_id),
        ),
        RunEvent::ToolCancelled { call, .. } => {
            state.active_calls.remove(&call.call_id);
            (TranscriptKind::Tool, None)
        }
        RunEvent::Error { .. } => (TranscriptKind::Error, None),
        RunEvent::Provider {
            event: ProviderEvent::ReasoningSummary { .. },
        } => (TranscriptKind::Assistant, None),
        RunEvent::Provider {
            event: ProviderEvent::Usage { .. },
        } => (TranscriptKind::Command, None),
        RunEvent::Provider { .. } => return,
        RunEvent::Phase { .. } | RunEvent::ToolStarted { .. } => return,
    };
    if let Some(document) =
        SemanticRenderer::new(state.preferences.clone()).run_event_document(&event, call.as_ref())
    {
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind,
            document,
            temporary: false,
        });
    }
}

fn apply_command_result(state: &mut TuiState, result: HostCommandResult) {
    if result.clear_transcript {
        state.transcript.clear();
        state.end();
    }
    if !result.document.is_empty() {
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: result.document,
            temporary: false,
        });
    }
    if let Some((session_id, page)) = result.session {
        state.session_id = session_id;
        state.transcript = transcript_from_messages(page.messages);
        state.before_sequence = page.before_sequence;
        state.has_more = page.has_more;
        state.end();
    }
    if let Some(preferences) = result.preferences {
        state.preferences = preferences;
    }
    if let Some(completions) = result.completions {
        state.completions = completions;
    }
    if let Some(sticky_skills) = result.sticky_skills {
        state.sticky_skills = sticky_skills;
    }
    if let Some(footer) = result.footer {
        state.footer = footer;
    }
}

fn update_streaming_assistant(state: &mut TuiState, delta: &str) {
    let old_line_count = if state.scroll_from_bottom > 0 {
        transcript_lines(state, state.transcript_width).len()
    } else {
        0
    };
    if let Some(entry) = state.transcript.last_mut()
        && entry.temporary
        && entry.kind == TranscriptKind::Assistant
        && let Some(PresentationBlock::Text(text)) = entry.document.blocks.first_mut()
    {
        text.push_str(delta);
        if state.scroll_from_bottom > 0 {
            state.preserve_scroll_after_line_change(old_line_count);
        }
        return;
    }
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Assistant,
        document: PresentationDocument::from_block(PresentationBlock::Text(delta.into())),
        temporary: true,
    });
}

fn finalize_assistant(state: &mut TuiState, output: &str) {
    let old_line_count = if state.scroll_from_bottom > 0 {
        transcript_lines(state, state.transcript_width).len()
    } else {
        0
    };
    if let Some(entry) = state.transcript.last_mut()
        && entry.temporary
        && entry.kind == TranscriptKind::Assistant
    {
        entry.document =
            if state.preferences.stream_mode == colossus_contracts::StreamDisplayMode::Raw {
                PresentationDocument::from_block(PresentationBlock::Text(output.into()))
            } else {
                PresentationDocument::from_block(PresentationBlock::Markdown(output.into()))
            };
        entry.temporary = false;
        if state.scroll_from_bottom > 0 {
            state.preserve_scroll_after_line_change(old_line_count);
        }
        return;
    }
    let completed = PresentationDocument::from_block(PresentationBlock::Markdown(output.into()));
    if state.transcript.last().is_some_and(|entry| {
        !entry.temporary && entry.kind == TranscriptKind::Assistant && entry.document == completed
    }) {
        return;
    }
    if !output.is_empty() {
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Assistant,
            document: completed,
            temporary: false,
        });
    }
}

struct OwnedTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _guard: TerminalGuard,
}

impl OwnedTerminal {
    fn new(mode: ScreenMode) -> Result<Self, io::Error> {
        let guard = TerminalGuard::enter(mode)?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = match mode {
            ScreenMode::Alternate => Terminal::new(backend)?,
            ScreenMode::Inline => Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Inline(20),
                },
            )?,
        };
        Ok(Self {
            terminal,
            _guard: guard,
        })
    }

    fn draw(&mut self, state: &mut TuiState) -> Result<(), io::Error> {
        self.terminal.draw(|frame| render(frame, state))?;
        Ok(())
    }
}

struct TerminalGuard {
    mode: ScreenMode,
}

impl TerminalGuard {
    fn enter(mode: ScreenMode) -> Result<Self, io::Error> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, Hide, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        if mode == ScreenMode::Alternate
            && let Err(error) = execute!(stdout, EnterAlternateScreen)
        {
            let _ = execute!(stdout, Show, DisableBracketedPaste);
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self { mode })
    }

    fn restore(&self) {
        let mut stdout = io::stdout();
        if self.mode == ScreenMode::Alternate {
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
        let _ = execute!(stdout, Show, DisableBracketedPaste);
        let _ = stdout.flush();
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn render(frame: &mut Frame<'_>, state: &mut TuiState) {
    let area = frame.area();
    if area.width < MINIMUM_TERMINAL_WIDTH || area.height < MINIMUM_TERMINAL_HEIGHT {
        let notice = Paragraph::new(format!(
            "Colossus needs at least {MINIMUM_TERMINAL_WIDTH}x{MINIMUM_TERMINAL_HEIGHT}. Current: {}x{}.\nYour draft and transcript are preserved.",
            area.width, area.height
        ))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Resize terminal"))
        .wrap(Wrap { trim: true });
        frame.render_widget(notice, area);
        return;
    }

    let composer_height = composer_height(state, area.width);
    let activity_height = u16::from(state.operation.is_some());
    let completion_height =
        completion_menu_height(state, area.height, composer_height, activity_height);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(activity_height),
            Constraint::Length(completion_height),
            Constraint::Length(composer_height),
            Constraint::Length(1),
        ])
        .split(area);
    state.transcript_height = usize::from(
        rows[0]
            .height
            .saturating_sub(u16::from(state.new_items > 0)),
    );
    state.transcript_width = usize::from(rows[0].width).max(20);
    render_transcript(frame, state, rows[0]);
    if activity_height > 0 {
        render_activity(frame, state, rows[1]);
    }
    if completion_height > 0 {
        render_completion_menu(frame, state, rows[2]);
    }
    render_composer(frame, state, rows[3]);
    render_footer(frame, state, rows[4]);
    if state.overlay.is_some() {
        render_overlay(frame, state, area);
    }
}

fn render_transcript(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let (badge_area, transcript_area) = if state.new_items > 0 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);
        (Some(rows[0]), rows[1])
    } else {
        (None, area)
    };
    if let Some(badge_area) = badge_area {
        let palette = TerminalPalette::for_preferences(&state.preferences);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" ↑ {} new · End returns live ", state.new_items),
                ratatui_style(palette.warning_style()),
            ))
            .alignment(Alignment::Right),
            badge_area,
        );
    }
    let width = usize::from(transcript_area.width).max(20);
    let lines = transcript_lines(state, width);
    let visible = usize::from(transcript_area.height);
    let live_top = lines.len().saturating_sub(visible);
    let top = live_top.saturating_sub(state.scroll_from_bottom);
    let paragraph = Paragraph::new(lines)
        .scroll((u16::try_from(top).unwrap_or(u16::MAX), 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, transcript_area);
}

fn transcript_lines<'a>(state: &'a TuiState, width: usize) -> Vec<Line<'a>> {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let mut lines = Vec::new();
    for (index, entry) in state.transcript.iter().enumerate() {
        if index > 0
            && state.preferences.transcript_density
                == colossus_contracts::TranscriptDensity::Comfortable
        {
            lines.push(Line::default());
        }
        let (marker, label) = match entry.kind {
            TranscriptKind::User => ("›", "You"),
            TranscriptKind::Assistant => ("●", "Colossus"),
            TranscriptKind::Tool => ("◆", "Tool"),
            TranscriptKind::Command => ("›", "Command"),
            TranscriptKind::Error => ("!", "Error"),
        };
        let label_style = match entry.kind {
            TranscriptKind::User => palette.user_style(),
            TranscriptKind::Command => palette.meta_style(),
            TranscriptKind::Assistant => palette.assistant_style(),
            TranscriptKind::Tool => palette.tool_style(),
            TranscriptKind::Error => palette.error_style(),
        };
        let has_semantic_heading = entry
            .document
            .blocks
            .first()
            .is_some_and(|block| matches!(block, PresentationBlock::Card { .. }));
        let show_label = state.preferences.transcript_density
            == colossus_contracts::TranscriptDensity::Comfortable
            && !(has_semantic_heading
                && matches!(
                    entry.kind,
                    TranscriptKind::Tool | TranscriptKind::Command | TranscriptKind::Error
                ));
        if show_label {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    ratatui_style(label_style).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    label,
                    ratatui_style(label_style).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        let rendered = StyledDocumentRenderer::for_transcript(state.preferences.clone(), width)
            .render(&entry.document);
        lines.extend(rendered.into_iter().map(|mut line| {
            if show_label && !line.spans.is_empty() {
                line.spans.insert(
                    0,
                    colossus_presentation::StyledSpan {
                        content: "  ".into(),
                        style: palette.meta_style(),
                    },
                );
            }
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, ratatui_style(span.style)))
                    .collect::<Vec<_>>(),
            )
        }));
    }
    lines
}

fn render_activity(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let elapsed = state
        .started_at
        .map_or(0.0, |started| started.elapsed().as_secs_f64());
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let frame_text = palette.activity_frame(elapsed, false);
    let activity = state.activity.as_deref().unwrap_or("working");
    let queued = if state.queue.is_empty() {
        String::new()
    } else {
        format!(" · {} queued", state.queue.len())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {frame_text} "),
                ratatui_style(palette.activity_style()),
            ),
            Span::styled(
                format!("{activity} · {elapsed:.1}s{queued}"),
                ratatui_style(palette.activity_style()),
            ),
        ])),
        area,
    );
}

fn render_completion_menu(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let candidates = state.completion_menu_candidates();
    let Some(context) = state.structured_completion_context() else {
        return;
    };
    if candidates.is_empty() || area.height < 3 {
        return;
    }
    let selected = state.composer.completion_index.unwrap_or(0) % candidates.len();
    let visible_rows = usize::from(area.height.saturating_sub(2));
    let first = selected
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(candidates.len().saturating_sub(visible_rows));
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let candidate_style = match context.kind {
        CompletionKind::Command => palette.assistant_style(),
        CompletionKind::Skill => palette.tool_style(),
    };
    let menu_width = area.width.saturating_sub(2).min(80);
    let menu_area = Rect::new(area.x.saturating_add(1), area.y, menu_width, area.height);
    let content_width = usize::from(menu_width.saturating_sub(5)).max(1);
    let lines = candidates
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_rows)
        .map(|(index, candidate)| {
            let is_selected = index == selected;
            let marker = if is_selected { "› " } else { "  " };
            let style = if is_selected {
                ratatui_style(candidate_style)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                ratatui_style(candidate_style)
            };
            Line::from(Span::styled(
                format!("{marker}{}", truncate_width(candidate, content_width)),
                style,
            ))
        })
        .collect::<Vec<_>>();
    let label = match context.kind {
        CompletionKind::Command => "Commands",
        CompletionKind::Skill => "Skills",
    };
    frame.render_widget(Clear, menu_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {label} · {} matches · Tab/↓ ", candidates.len())),
        ),
        menu_area,
    );
}

fn render_composer(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let inner_width = usize::from(area.width.saturating_sub(4)).max(1);
    let (before, after) = state.composer.draft.split_at(state.composer.cursor);
    let ghost = state.ghost_text().unwrap_or("");
    let mut text = Vec::new();
    let logical_lines = format!("{before}{after}{ghost}")
        .split('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // Render the common single-line case with a distinct ghost span. Multiline retains
    // newlines and uses the real terminal cursor for exact position.
    if !state.composer.draft.contains('\n') {
        let mut ghost_style = palette.meta_style();
        ghost_style.dim = true;
        text.push(Line::from(vec![
            Span::raw(before.to_owned()),
            Span::raw(after.to_owned()),
            Span::styled(ghost.to_owned(), ratatui_style(ghost_style)),
        ]));
    } else {
        text.extend(logical_lines.into_iter().map(Line::from));
    }
    let title = if state.preferences.multiline {
        " Message · Ctrl/Alt+Enter sends "
    } else {
        " Message · Enter sends "
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
    let (cursor_row, cursor_column) = composer_cursor_position(before, inner_width);
    let x = area
        .x
        .saturating_add(1)
        .saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX));
    let y = area
        .y
        .saturating_add(1)
        .saturating_add(u16::try_from(cursor_row).unwrap_or(u16::MAX));
    if x < area.right().saturating_sub(1) && y < area.bottom().saturating_sub(1) {
        frame.set_cursor_position((x, y));
    }
}

fn render_footer(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let width = usize::from(area.width);
    let short_session = state.session_id.chars().take(8).collect::<String>();
    let mut segments = vec![format!(" Colossus {short_session}")];
    if width >= 60 {
        segments.push(format!("{}:{}", state.footer.role, state.footer.route));
    }
    if width >= 90
        && let Some((used, maximum)) = state.footer.context
    {
        segments.push(format!("ctx={used}/{maximum}"));
    }
    if width >= 110 {
        segments.push(format!("msgs={}", state.footer.message_count));
        segments.push(format!("approval={}", state.footer.approval_mode));
    }
    segments.push(format!("status={}", state.footer.status));
    let mut footer = segments.join(" · ");
    if UnicodeWidthStr::width(footer.as_str()) > width {
        footer = truncate_width(&footer, width);
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    frame.render_widget(
        Paragraph::new(Span::styled(footer, ratatui_style(palette.meta_style()))),
        area,
    );
}

fn render_overlay(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let overlay_area = match state.overlay.as_ref() {
        Some(Overlay::Prompt { request, .. })
            if request.document.is_empty() && !request.choices.is_empty() =>
        {
            picker_rect(area, &request.choices)
        }
        _ => centered_rect(80, 60, area),
    };
    frame.render_widget(Clear, overlay_area);
    let (title, mut lines) = match state.overlay.as_ref() {
        Some(Overlay::Prompt {
            request,
            input,
            selected,
        }) => {
            let inner_width = usize::from(overlay_area.width.saturating_sub(2)).max(1);
            let inner_height = usize::from(overlay_area.height.saturating_sub(2)).max(1);
            let lines = prompt_lines(
                request,
                input,
                *selected,
                &state.preferences,
                inner_width,
                inner_height,
            );
            let title = selected.map_or_else(
                || request.title.clone(),
                |selected| {
                    format!(
                        "{} · {}/{}",
                        request.title,
                        selected + 1,
                        request.choices.len()
                    )
                },
            );
            (title, lines)
        }
        Some(Overlay::HistorySearch { query }) => (
            "History search".into(),
            vec![
                Line::from(format!("> {query}")),
                Line::default(),
                Line::from(
                    state
                        .history
                        .iter()
                        .rev()
                        .find(|entry| entry.contains(query.as_str()))
                        .map_or("No match", String::as_str)
                        .to_owned(),
                ),
            ],
        ),
        Some(Overlay::QueuePaused) => (
            "Queued turns paused".into(),
            vec![
                Line::from("The prior run failed or was cancelled."),
                Line::from(format!("{} queued turn(s) remain.", state.queue.len())),
                Line::default(),
                Line::from("Enter/R: resume queue · C: clear queue · Esc: keep paused"),
            ],
        ),
        None => return,
    };
    if lines.is_empty() {
        lines.push(Line::default());
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .style(Style::default().bg(Color::Reset)),
            )
            .wrap(Wrap { trim: false }),
        overlay_area,
    );
}

fn picker_rect(area: Rect, choices: &[String]) -> Rect {
    let width = if area.width <= 60 {
        area.width
    } else {
        area.width.saturating_sub(8).min(96)
    };
    let choice_rows = choices
        .iter()
        .map(|choice| choice.lines().count().max(1))
        .sum::<usize>();
    let desired_height = u16::try_from(choice_rows.min(14).saturating_add(3))
        .unwrap_or(u16::MAX)
        .min(area.height);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(desired_height) / 2),
        width,
        desired_height,
    )
}

fn prompt_lines(
    request: &InteractivePrompt,
    input: &str,
    selected: Option<usize>,
    preferences: &TerminalPreferences,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let palette = TerminalPalette::for_preferences(preferences);
    let document = StyledDocumentRenderer::new(preferences.clone(), width)
        .render(&request.document)
        .into_iter()
        .map(|line| {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, ratatui_style(span.style)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let choice_rows = request
        .choices
        .iter()
        .map(|choice| choice.lines().count().max(1))
        .sum::<usize>();
    let footer_rows = usize::from(height > 0);
    let minimum_document_rows = if document.is_empty() {
        0
    } else {
        document.len().min(3)
    };
    let choice_budget = choice_rows.min(
        height
            .saturating_sub(footer_rows)
            .saturating_sub(minimum_document_rows),
    );
    let document_budget = height
        .saturating_sub(footer_rows)
        .saturating_sub(choice_budget);
    let mut lines = document
        .into_iter()
        .take(document_budget)
        .collect::<Vec<_>>();

    if choice_budget > 0 {
        let focus = selected
            .unwrap_or(0)
            .min(request.choices.len().saturating_sub(1));
        let (start, end) = visible_choice_range(&request.choices, focus, choice_budget);
        let mut remaining = choice_budget;
        for (index, choice) in request
            .choices
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            let is_selected = selected == Some(index);
            let marker = if is_selected { "› " } else { "  " };
            let prefix = format!("{marker}{}. ", index + 1);
            let continuation = " ".repeat(prefix.chars().count());
            for (line_index, content) in choice.lines().enumerate() {
                if remaining == 0 {
                    break;
                }
                let prefix = if line_index == 0 {
                    prefix.as_str()
                } else {
                    continuation.as_str()
                };
                let available = width.saturating_sub(UnicodeWidthStr::width(prefix));
                let base_style = if line_index == 0 {
                    palette.assistant_style()
                } else {
                    palette.meta_style()
                };
                let style = if is_selected {
                    ratatui_style(base_style)
                        .add_modifier(Modifier::BOLD)
                        .add_modifier(Modifier::REVERSED)
                } else {
                    ratatui_style(base_style)
                };
                lines.push(Line::from(Span::styled(
                    format!("{prefix}{}", truncate_width(content, available)),
                    style,
                )));
                remaining -= 1;
            }
        }
    }

    if footer_rows > 0 {
        let hint = if !input.is_empty() {
            format!("Choice: {input} · Enter submit · Esc cancel")
        } else if selected.is_some() && !request.choices.is_empty() {
            "↑/↓ move · Enter select · Esc cancel".into()
        } else if !request.choices.is_empty() {
            "↑/↓ move · type a number · Esc cancel".into()
        } else if request.allow_free_form {
            "Type an answer · Enter submit · Esc cancel".into()
        } else {
            "Enter submit · Esc cancel".into()
        };
        lines.push(Line::from(Span::styled(
            truncate_width(&hint, width),
            ratatui_style(palette.warning_style()),
        )));
    }
    lines
}

fn visible_choice_range(choices: &[String], selected: usize, row_budget: usize) -> (usize, usize) {
    if choices.is_empty() || row_budget == 0 {
        return (0, 0);
    }
    let selected = selected.min(choices.len() - 1);
    let row_count = |choice: &String| choice.lines().count().max(1);
    let mut start = 0;
    let mut used = choices[..=selected].iter().map(row_count).sum::<usize>();
    while start < selected && used > row_budget {
        used = used.saturating_sub(row_count(&choices[start]));
        start += 1;
    }
    let mut end = selected + 1;
    while end < choices.len() {
        let next = row_count(&choices[end]);
        if used.saturating_add(next) > row_budget {
            break;
        }
        used += next;
        end += 1;
    }
    while start > 0 {
        let previous = row_count(&choices[start - 1]);
        if used.saturating_add(previous) > row_budget {
            break;
        }
        start -= 1;
        used += previous;
    }
    (start, end)
}

fn transcript_from_messages(messages: Vec<SessionMessage>) -> Vec<TranscriptEntry> {
    let mut entries = Vec::new();
    let mut tool_names = BTreeMap::<String, String>::new();
    for record in messages {
        let (kind, document) = match record.message.role {
            ModelMessageRole::System => continue,
            ModelMessageRole::User => (
                TranscriptKind::User,
                PresentationDocument::from_block(PresentationBlock::Markdown(
                    record.message.content,
                )),
            ),
            ModelMessageRole::Assistant => {
                let mut document = PresentationDocument::new();
                if !record.message.content.is_empty() {
                    document.push(PresentationBlock::Markdown(record.message.content));
                }
                for call in record.message.tool_calls {
                    tool_names.insert(call.call_id.clone(), call.name.clone());
                    document.push(PresentationBlock::Card {
                        title: format!("Requested {}", call.name),
                        tone: PresentationTone::Tool,
                        body: vec![PresentationBlock::Code {
                            language: Some("arguments".into()),
                            content: call.arguments.to_string(),
                        }],
                    });
                }
                (TranscriptKind::Assistant, document)
            }
            ModelMessageRole::Tool => {
                let title = record.message.tool_call_id.as_ref().map_or_else(
                    || "Tool result".into(),
                    |id| {
                        tool_names.get(id).map_or_else(
                            || format!("Tool result {id}"),
                            |name| format!("Completed {name}"),
                        )
                    },
                );
                (
                    TranscriptKind::Tool,
                    PresentationDocument::from_block(PresentationBlock::Card {
                        title,
                        tone: PresentationTone::Tool,
                        body: vec![PresentationBlock::Code {
                            language: None,
                            content: record.message.content,
                        }],
                    }),
                )
            }
        };
        if !document.is_empty() {
            entries.push(TranscriptEntry {
                sequence: Some(record.sequence),
                kind,
                document,
                temporary: false,
            });
        }
    }
    entries
}

fn user_entry(content: &str, kind: TranscriptKind) -> TranscriptEntry {
    TranscriptEntry {
        sequence: None,
        kind,
        document: PresentationDocument::from_block(PresentationBlock::Markdown(content.into())),
        temporary: false,
    }
}

fn error_entry(message: &str) -> TranscriptEntry {
    TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Error,
        document: PresentationDocument::from_block(PresentationBlock::Card {
            title: "Error".into(),
            tone: PresentationTone::Error,
            body: vec![PresentationBlock::Text(message.into())],
        }),
        temporary: false,
    }
}

fn help_document() -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Colossus terminal".into(),
        tone: PresentationTone::Neutral,
        body: vec![
            PresentationBlock::Text(
                "Type a message to run the agent. Slash commands operate durable state.".into(),
            ),
            PresentationBlock::KeyValue(vec![
                (
                    "Send".into(),
                    "Enter; Ctrl/Alt+Enter in multiline mode".into(),
                ),
                ("Scroll".into(), "PageUp/PageDown; End returns live".into()),
                (
                    "Complete".into(),
                    "Type / or @ for suggestions; Tab/Arrows select; Right accepts".into(),
                ),
                (
                    "History".into(),
                    "Up/Down at boundaries; Ctrl-R searches".into(),
                ),
                (
                    "Cancel".into(),
                    "Ctrl-C clears draft, modal, or active run".into(),
                ),
                ("Preferences".into(), "/tui prefs|save|reset".into()),
                ("Exit".into(), "Ctrl-D while idle or /exit".into()),
            ]),
        ],
    })
}

fn preferences_document(preferences: &TerminalPreferences) -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::KeyValue(vec![
        ("Theme".into(), preferences.theme_name().into()),
        (
            "Streaming".into(),
            format!("{:?}", preferences.stream_mode).to_lowercase(),
        ),
        (
            "Events".into(),
            format!("{:?}", preferences.events_mode).to_lowercase(),
        ),
        (
            "Reasoning summaries".into(),
            if preferences.show_reasoning {
                "on"
            } else {
                "off"
            }
            .into(),
        ),
        (
            "Transcript".into(),
            preferences.transcript_density.as_str().into(),
        ),
        (
            "Multiline".into(),
            if preferences.multiline { "on" } else { "off" }.into(),
        ),
    ]))
}

fn runtime_command_name(command: &RuntimeCommand) -> &str {
    match command {
        RuntimeCommand::Known { name, .. } => name,
    }
}

fn ratatui_style(style: ThemeTextStyle) -> Style {
    let mut rendered = Style::default();
    if let Some(color) = style.foreground {
        rendered = rendered.fg(Color::Rgb(color.red, color.green, color.blue));
    }
    if style.bold {
        rendered = rendered.add_modifier(Modifier::BOLD);
    }
    if style.dim {
        rendered = rendered.add_modifier(Modifier::DIM);
    }
    if style.italic {
        rendered = rendered.add_modifier(Modifier::ITALIC);
    }
    rendered
}

fn composer_height(state: &TuiState, width: u16) -> u16 {
    let inner_width = usize::from(width.saturating_sub(4)).max(1);
    let rows = state
        .composer
        .draft
        .split('\n')
        .map(|line| UnicodeWidthStr::width(line).div_ceil(inner_width).max(1))
        .sum::<usize>();
    u16::try_from(rows.clamp(1, 6) + 2).unwrap_or(8)
}

fn completion_menu_height(
    state: &TuiState,
    total_height: u16,
    composer_height: u16,
    activity_height: u16,
) -> u16 {
    let candidate_rows = state
        .completion_menu_candidates()
        .len()
        .min(MAX_COMPLETION_MENU_ROWS);
    if candidate_rows == 0 {
        return 0;
    }
    let available = total_height
        .saturating_sub(3)
        .saturating_sub(activity_height)
        .saturating_sub(composer_height)
        .saturating_sub(1);
    if available < 3 {
        return 0;
    }
    u16::try_from(candidate_rows + 2)
        .unwrap_or(u16::MAX)
        .min(available)
}

fn composer_cursor_position(before: &str, width: usize) -> (usize, usize) {
    let mut row = 0;
    let mut column = 0;
    for character in before.chars() {
        if character == '\n' {
            row += 1;
            column = 0;
            continue;
        }
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if column + character_width > width {
            row += 1;
            column = 0;
        }
        column += character_width;
        if column == width {
            row += 1;
            column = 0;
        }
    }
    (row.min(5), column)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(value: &str, cursor: usize) -> usize {
    value[cursor..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(index, _)| cursor + index)
}

fn sanitize_input(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .take(1024 * 1024)
        .collect()
}

fn truncate_width(value: &str, maximum: usize) -> String {
    let mut width = 0;
    value
        .chars()
        .take_while(|character| {
            let next = width + UnicodeWidthChar::width(*character).unwrap_or(0);
            if next > maximum {
                false
            } else {
                width = next;
                true
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{
        CustomTheme, EventDisplayMode, ModelMessage, ModelToolCall, SessionMessage,
        StreamDisplayMode, ThemeColor, ThemeSpinner, ThemeTextStyle, TranscriptDensity,
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn snapshot() -> InteractiveSnapshot {
        InteractiveSnapshot {
            session_id: "019f-test".into(),
            transcript: SessionMessagePage {
                messages: vec![SessionMessage {
                    session_id: "019f-test".into(),
                    run_id: "run".into(),
                    sequence: 1,
                    message: ModelMessage {
                        role: ModelMessageRole::Assistant,
                        content: "durable row marker".into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    created_at: "2026-07-15T00:00:00Z".into(),
                }],
                before_sequence: Some(1),
                has_more: true,
            },
            preferences: TerminalPreferences::default(),
            history: vec!["older prompt".into()],
            completions: vec!["/tools".into(), "/tui prefs".into()],
            footer: FooterState {
                role: "primary".into(),
                route: "echo@echo".into(),
                context: Some((1, 32_768)),
                message_count: 1,
                status: "ready".into(),
                approval_mode: "ask".into(),
            },
        }
    }

    fn custom_theme() -> CustomTheme {
        let primary = ThemeTextStyle {
            foreground: Some(ThemeColor {
                red: 64,
                green: 200,
                blue: 255,
            }),
            bold: false,
            dim: false,
            italic: false,
        };
        let muted = ThemeTextStyle {
            foreground: Some(ThemeColor {
                red: 100,
                green: 110,
                blue: 120,
            }),
            bold: false,
            dim: true,
            italic: true,
        };
        CustomTheme {
            schema_version: 1,
            name: "test_ocean".into(),
            source_hash: "a".repeat(64),
            base: colossus_contracts::ThemeName::Default,
            title: "Ocean".into(),
            caret: "›".into(),
            continuation: "…".into(),
            prompt_left: primary.foreground,
            prompt_right: muted.foreground,
            indicator: primary.foreground,
            continuation_color: muted.foreground,
            assistant: primary,
            activity: primary,
            thinking: muted,
            tool: primary,
            success: primary,
            warning: primary,
            error: primary,
            meta: muted,
            spinner: ThemeSpinner::Line,
        }
    }

    #[test]
    fn parser_handles_tui_commands_without_a_repl_alias() {
        assert_eq!(
            parse_interactive_command("/tui prefs"),
            InteractiveCommand::Local(LocalCommand::Preferences)
        );
        assert_eq!(
            parse_interactive_command("/tui reset"),
            InteractiveCommand::Local(LocalCommand::ResetPreferences)
        );
        assert_eq!(
            parse_interactive_command("/repl reset"),
            InteractiveCommand::Runtime(RuntimeCommand::Known {
                name: "repl".into(),
                arguments: "reset".into(),
            })
        );
        assert_eq!(
            parse_interactive_command("/plans"),
            InteractiveCommand::Runtime(RuntimeCommand::Known {
                name: "plans".into(),
                arguments: String::new(),
            })
        );
    }

    #[test]
    fn unicode_editing_never_splits_a_character() {
        let mut composer = Composer::default();
        composer.insert("a🦀界");
        composer.move_left();
        composer.backspace();
        assert_eq!(composer.draft, "a界");
        assert!(composer.draft.is_char_boundary(composer.cursor));
        composer.delete();
        assert_eq!(composer.draft, "a");
    }

    #[test]
    fn multiline_history_search_and_boundary_navigation_preserve_the_draft() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.preferences.multiline = true;
        state.composer.insert("first");
        state.composer.insert("\n");
        state.composer.insert("界");
        assert_eq!(state.draft(), "first\n界");
        state.composer.cursor = 0;
        state.previous_history();
        assert_eq!(state.draft(), "older prompt");
        state.next_history();
        assert!(state.draft().is_empty());
        state.composer.insert("unsent draft");
        state.overlay = Some(Overlay::HistorySearch {
            query: "older".into(),
        });
        handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(state.draft(), "unsent draft");
    }

    #[test]
    fn completion_ghost_is_separate_and_right_accepts_it() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.composer.insert("/to");
        assert_eq!(state.ghost_text(), Some("ols"));
        assert!(state.accept_completion());
        assert_eq!(state.draft(), "/tools");
    }

    #[test]
    fn structured_completion_tracks_slash_commands_and_skill_tokens() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.completions.extend([
            "@coding".into(),
            "@offline-dev".into(),
            "@security-review".into(),
        ]);

        state.composer.insert("/");
        assert_eq!(
            state.structured_completion_context(),
            Some(CompletionContext {
                prefix: "/",
                kind: CompletionKind::Command,
            })
        );
        assert_eq!(
            state.completion_menu_candidates(),
            vec!["/tools", "/tui prefs"]
        );

        state.composer.clear();
        state.composer.insert("please @off");
        assert_eq!(
            state.structured_completion_context(),
            Some(CompletionContext {
                prefix: "@off",
                kind: CompletionKind::Skill,
            })
        );
        assert_eq!(state.ghost_text(), Some("line-dev"));
        assert!(state.accept_completion());
        assert_eq!(state.draft(), "please @offline-dev ");
    }

    #[test]
    fn completion_selection_moves_in_both_directions_and_can_be_dismissed() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.composer.insert("/");
        state.advance_completion();
        assert_eq!(state.composer.completion_index, Some(0));
        state.advance_completion();
        assert_eq!(state.composer.completion_index, Some(1));
        state.previous_completion();
        assert_eq!(state.composer.completion_index, Some(0));
        assert!(state.hide_completion());
        assert!(state.completion_menu_candidates().is_empty());
        state.composer.insert("to");
        assert_eq!(state.completion_menu_candidates(), vec!["/tools"]);
    }

    #[test]
    fn visible_completion_menu_is_adaptive_at_minimum_size() {
        for (width, height) in [(40, 12), (60, 16)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state = TuiState::from_snapshot(snapshot());
            state.composer.insert("/");
            terminal
                .draw(|frame| render(frame, &mut state))
                .expect("draw completion menu");
            let rendered = terminal.backend().to_string();
            assert!(
                rendered.contains("Commands"),
                "{width}x{height}: {rendered}"
            );
            assert!(rendered.contains("/tools"), "{width}x{height}: {rendered}");
            assert!(
                rendered.contains("/tui prefs"),
                "{width}x{height}: {rendered}"
            );
            assert!(
                rendered.contains("durable row marker"),
                "{width}x{height}: {rendered}"
            );
            assert!(
                rendered.contains("Message · Enter sends"),
                "{width}x{height}: {rendered}"
            );
            assert!(
                rendered.contains("Colossus"),
                "{width}x{height}: {rendered}"
            );
        }
    }

    #[test]
    fn completion_ghost_uses_a_distinct_low_emphasis_style() {
        let backend = TestBackend::new(40, 3);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = TuiState::from_snapshot(snapshot());
        state.completions = vec!["candy".into()];
        state.composer.insert("can");
        terminal
            .draw(|frame| render_composer(frame, &state, frame.area()))
            .expect("draw composer");
        let buffer = terminal.backend().buffer();
        let typed = buffer.cell((1, 1)).expect("typed cell").style();
        let ghost = buffer.cell((4, 1)).expect("ghost cell").style();
        assert_ne!(typed, ghost);
        assert!(ghost.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn canonical_system_messages_are_excluded() {
        let mut source = snapshot();
        source.transcript.messages.insert(
            0,
            SessionMessage {
                session_id: "019f-test".into(),
                run_id: "run".into(),
                sequence: 0,
                message: ModelMessage {
                    role: ModelMessageRole::System,
                    content: "hidden instructions".into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                created_at: "2026-07-15T00:00:00Z".into(),
            },
        );
        let state = TuiState::from_snapshot(source);
        assert_eq!(state.transcript.len(), 1);
        assert!(
            !transcript_lines(&state, 80)
                .iter()
                .any(|line| line.to_string().contains("hidden instructions"))
        );
    }

    #[test]
    fn historical_tool_results_are_correlated_with_assistant_calls() {
        let mut source = snapshot();
        source.transcript.messages = vec![
            SessionMessage {
                session_id: "019f-test".into(),
                run_id: "run".into(),
                sequence: 1,
                message: ModelMessage {
                    role: ModelMessageRole::Assistant,
                    content: String::new(),
                    tool_call_id: None,
                    tool_calls: vec![ModelToolCall {
                        call_id: "call-1".into(),
                        name: "filesystem.search".into(),
                        arguments: serde_json::json!({"query": "needle"}),
                    }],
                },
                created_at: "2026-07-15T00:00:00Z".into(),
            },
            SessionMessage {
                session_id: "019f-test".into(),
                run_id: "run".into(),
                sequence: 2,
                message: ModelMessage {
                    role: ModelMessageRole::Tool,
                    content: "found".into(),
                    tool_call_id: Some("call-1".into()),
                    tool_calls: Vec::new(),
                },
                created_at: "2026-07-15T00:00:00Z".into(),
            },
        ];
        let state = TuiState::from_snapshot(source);
        let rendered = transcript_lines(&state, 80)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("Completed filesystem.search"),
            "{rendered}"
        );
    }

    #[test]
    fn session_switch_replaces_transcript_and_resets_live_scroll_state() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.page_up();
        state.new_items = 3;
        apply_command_result(
            &mut state,
            HostCommandResult {
                document: PresentationDocument::new(),
                session: Some((
                    "019f-other".into(),
                    SessionMessagePage {
                        messages: vec![SessionMessage {
                            session_id: "019f-other".into(),
                            run_id: "other-run".into(),
                            sequence: 1,
                            message: ModelMessage {
                                role: ModelMessageRole::Assistant,
                                content: "other transcript".into(),
                                tool_call_id: None,
                                tool_calls: Vec::new(),
                            },
                            created_at: "2026-07-15T00:00:00Z".into(),
                        }],
                        before_sequence: Some(1),
                        has_more: true,
                    },
                )),
                preferences: None,
                completions: None,
                sticky_skills: None,
                footer: None,
                clear_transcript: false,
            },
        );
        assert_eq!(state.session_id, "019f-other");
        assert_eq!(state.transcript.len(), 1);
        assert_eq!(state.scroll_from_bottom, 0);
        assert_eq!(state.new_items, 0);
        assert!(
            transcript_lines(&state, 80)
                .iter()
                .any(|line| line.to_string().contains("other transcript"))
        );
    }

    #[test]
    fn on_raw_and_off_stream_modes_keep_their_distinct_transcript_contracts() {
        let envelope = |event| RunEventEnvelope {
            schema_version: 1,
            run_id: "run-stream".into(),
            session_id: "019f-test".into(),
            event,
        };
        for mode in [
            StreamDisplayMode::On,
            StreamDisplayMode::Raw,
            StreamDisplayMode::Off,
        ] {
            let mut state = TuiState::from_snapshot(snapshot());
            state.preferences.stream_mode = mode;
            let starting = state.transcript.len();
            handle_run_event(
                &mut state,
                envelope(RunEvent::Provider {
                    event: ProviderEvent::ModelDelta {
                        text: "**partial**".into(),
                    },
                }),
            );
            assert_eq!(
                state.transcript.len(),
                starting + usize::from(mode != StreamDisplayMode::Off)
            );
            handle_run_event(
                &mut state,
                envelope(RunEvent::Provider {
                    event: ProviderEvent::FinalOutput {
                        text: "**complete**".into(),
                    },
                }),
            );
            let block = state
                .transcript
                .last()
                .and_then(|entry| entry.document.blocks.first())
                .expect("final block");
            if mode == StreamDisplayMode::Raw {
                assert!(matches!(block, PresentationBlock::Text(_)));
            } else {
                assert!(matches!(block, PresentationBlock::Markdown(_)));
            }
        }
    }

    #[test]
    fn prompt_cancel_is_one_use_and_preserves_the_composer_draft() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.composer.insert("draft stays here");
        let (response, mut received) = oneshot::channel();
        handle_host_event(
            &mut state,
            HostEvent::Prompt(InteractivePrompt {
                id: "prompt-1".into(),
                title: "Approval".into(),
                document: PresentationDocument::from_block(PresentationBlock::Text(
                    "Allow?".into(),
                )),
                choices: vec!["allow".into(), "deny".into()],
                initial_choice: None,
                allow_free_form: false,
                response,
            }),
        );
        handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
        assert_eq!(state.draft(), "draft stays here");
        assert!(state.overlay.is_none());
    }

    #[test]
    fn prompt_keyboard_selection_returns_the_highlighted_choice() {
        let mut state = TuiState::from_snapshot(snapshot());
        let (response, mut received) = oneshot::channel();
        handle_host_event(
            &mut state,
            HostEvent::Prompt(InteractivePrompt {
                id: "session-picker".into(),
                title: "Resume session".into(),
                document: PresentationDocument::new(),
                choices: vec![
                    "First session\nFirst preview".into(),
                    "Second session\nSecond preview".into(),
                ],
                initial_choice: Some(0),
                allow_free_form: false,
                response,
            }),
        );
        handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(
            received.try_recv(),
            Ok(PromptResponse::Answer(
                "Second session\nSecond preview".into()
            ))
        );
        assert!(state.overlay.is_none());
    }

    #[test]
    fn blank_approval_submission_still_fails_closed() {
        let mut state = TuiState::from_snapshot(snapshot());
        let (response, mut received) = oneshot::channel();
        handle_host_event(
            &mut state,
            HostEvent::Prompt(InteractivePrompt {
                id: "approval".into(),
                title: "Approval".into(),
                document: PresentationDocument::from_block(PresentationBlock::Text(
                    "Allow?".into(),
                )),
                choices: vec!["Allow once".into(), "Deny".into()],
                initial_choice: None,
                allow_free_form: false,
                response,
            }),
        );
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
    }

    #[test]
    fn resume_picker_is_responsive_and_keeps_the_selected_preview_visible() {
        for (width, height) in [(40, 12), (80, 24)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state = TuiState::from_snapshot(snapshot());
            let choices = (0..10)
                .map(|index| {
                    let message_count = index + 1;
                    format!(
                        "Session {index} · {message_count} msgs · 2026-07-18 01:4{index} · 019f72e{index}\nPrior user message {index}"
                    )
                })
                .collect::<Vec<_>>();
            let (response, _received) = oneshot::channel();
            handle_host_event(
                &mut state,
                HostEvent::Prompt(InteractivePrompt {
                    id: "session-picker".into(),
                    title: "Resume session".into(),
                    document: PresentationDocument::new(),
                    choices,
                    initial_choice: Some(7),
                    allow_free_form: false,
                    response,
                }),
            );
            terminal
                .draw(|frame| render(frame, &mut state))
                .expect("draw resume picker");
            let rendered = terminal.backend().to_string();
            assert!(
                rendered.contains("Resume session · 8/10"),
                "{width}x{height}: {rendered}"
            );
            assert!(
                rendered.contains("Prior user message 7"),
                "{width}x{height}: {rendered}"
            );
            assert!(
                rendered.contains("Enter select"),
                "{width}x{height}: {rendered}"
            );
            assert!(!rendered.contains("Message count"), "{rendered}");
            assert!(!rendered.contains("Created at"), "{rendered}");
            assert!(!rendered.contains("Prior user message 0"), "{rendered}");
        }
    }

    #[test]
    fn scrolled_up_state_counts_new_items_without_losing_position() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.transcript_height = 4;
        state.transcript_width = 80;
        for index in 0..8 {
            state.append_entry(user_entry(
                &format!("old row {index}"),
                TranscriptKind::User,
            ));
        }
        state.page_up();
        let before_lines = transcript_lines(&state, state.transcript_width).len();
        let before_top = before_lines
            .saturating_sub(state.transcript_height)
            .saturating_sub(state.scroll_from_bottom);
        state.append_entry(user_entry("new row", TranscriptKind::User));
        let after_lines = transcript_lines(&state, state.transcript_width).len();
        let after_top = after_lines
            .saturating_sub(state.transcript_height)
            .saturating_sub(state.scroll_from_bottom);
        assert_eq!(after_top, before_top);
        assert_eq!(state.new_items, 1);
        state.end();
        assert_eq!(state.scroll_from_bottom, 0);
        assert_eq!(state.new_items, 0);
    }

    #[test]
    fn queue_is_bounded_to_eight_future_turns() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.operation = Some(OperationKind::Run);
        for index in 0..10 {
            if state.queue.len() < MAX_QUEUED_TURNS {
                state.queue.push_back(format!("turn {index}"));
            }
        }
        assert_eq!(state.queue.len(), MAX_QUEUED_TURNS);
    }

    #[test]
    fn failed_or_cancelled_runs_pause_the_queue_and_cancellation_is_cooperative() {
        let mut state = TuiState::from_snapshot(snapshot());
        let control = RunControl::default();
        state.operation = Some(OperationKind::Run);
        state.control = Some(control.clone());
        state.queue.push_back("next turn".into());
        assert!(state.cancel_focus());
        assert!(control.is_cancelled());
        handle_host_event(
            &mut state,
            HostEvent::OperationFinished(Box::new(Err("cancelled".into()))),
        );
        assert!(state.queue_paused);
        assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
    }

    #[test]
    fn hostile_controls_are_removed_and_minimum_size_preserves_state() {
        assert_eq!(
            sanitize_input("safe\u{1b}]8;;evil\u{7}text\r\n"),
            "safe]8;;eviltext\n"
        );
        let backend = TestBackend::new(39, 11);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = TuiState::from_snapshot(snapshot());
        state.composer.insert("preserved draft");
        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Resize terminal"));
        assert_eq!(state.draft(), "preserved draft");
        assert_eq!(state.transcript.len(), 1);
    }

    #[test]
    fn every_theme_keeps_transcript_and_composer_at_all_required_sizes() {
        let themes = [
            colossus_contracts::ThemeName::Default,
            colossus_contracts::ThemeName::Mono,
            colossus_contracts::ThemeName::HighContrast,
            colossus_contracts::ThemeName::Carrot,
            colossus_contracts::ThemeName::Hacker,
        ];
        for custom in [false, true] {
            for theme in themes {
                for (width, height) in [(40, 12), (60, 20), (80, 24), (120, 40), (160, 50)] {
                    let mut source = snapshot();
                    source.preferences = TerminalPreferences {
                        theme,
                        custom_theme: custom.then(custom_theme),
                        stream_mode: StreamDisplayMode::On,
                        events_mode: EventDisplayMode::Compact,
                        transcript_density: TranscriptDensity::Comfortable,
                        ..TerminalPreferences::default()
                    };
                    let backend = TestBackend::new(width, height);
                    let mut terminal = Terminal::new(backend).expect("test terminal");
                    let mut state = TuiState::from_snapshot(source);
                    state.composer.insert("draft marker");
                    terminal
                        .draw(|frame| render(frame, &mut state))
                        .expect("draw");
                    let rendered = terminal.backend().to_string();
                    assert!(rendered.contains("durable row marker"), "{width}x{height}");
                    assert!(rendered.contains("draft marker"), "{width}x{height}");
                    assert!(rendered.contains("Colossus"), "{width}x{height}");
                }
            }
        }
    }

    #[test]
    fn transcript_is_borderless_and_uses_distinct_speaker_and_semantic_cues() {
        let mut state = TuiState::from_snapshot(snapshot());
        state.append_entry(user_entry("question", TranscriptKind::User));
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: help_document(),
            temporary: false,
        });
        let lines = transcript_lines(&state, 80);
        let rendered = lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("● Colossus"), "{rendered}");
        assert!(rendered.contains("› You"), "{rendered}");
        assert!(rendered.contains("◆ Colossus terminal"), "{rendered}");
        assert!(!rendered.contains("Command\n"), "{rendered}");
        assert!(!rendered.contains("┌─Colossus terminal"), "{rendered}");
        assert!(!rendered.contains("│ Field"), "{rendered}");

        let assistant = lines
            .iter()
            .find(|line| line.to_string().contains("● Colossus"))
            .expect("assistant label");
        let user = lines
            .iter()
            .find(|line| line.to_string().contains("› You"))
            .expect("user label");
        assert_ne!(assistant.spans[0].style, user.spans[0].style);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let screen = terminal.backend().to_string();
        assert!(!screen.contains("┌─Transcript"), "{screen}");
        assert!(screen.contains("Message · Enter sends"), "{screen}");
    }
}
