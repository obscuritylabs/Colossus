use super::*;

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
    /// Include explicitly released provider response evidence on a failed turn.
    pub include_provider_response_diagnostics: bool,
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
    /// Enable or disable in-run provider response diagnostics for this TUI process.
    ProviderDiagnostics(bool),
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
        "/provider diagnostics on" => {
            InteractiveCommand::Local(LocalCommand::ProviderDiagnostics(true))
        }
        "/provider diagnostics off" => {
            InteractiveCommand::Local(LocalCommand::ProviderDiagnostics(false))
        }
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
    /// Policy-released informational notice that does not take focus.
    Notice(PresentationDocument),
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
