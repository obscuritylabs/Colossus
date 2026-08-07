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
    /// Direct-execution boundary that still requires this TUI session's acknowledgement.
    pub pending_sandbox_boundary_acknowledgement: Option<SandboxBoundaryMode>,
}

/// Request for one normal provider/tool turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveRunRequest {
    /// Exact durable session.
    pub session_id: String,
    /// Complete user prompt after local skill-mention parsing.
    pub prompt: String,
    /// Trusted execute or plan behavior selected by the terminal reducer.
    pub mode: AgentRunMode,
    /// Explicit skills activated by this prompt.
    pub explicit_skills: Vec<String>,
    /// Sticky skills active in the terminal.
    pub sticky_skills: Vec<String>,
    /// Include explicitly released provider response evidence on a failed turn.
    pub include_provider_response_diagnostics: bool,
}

/// Process-local terminal behavior. This is intentionally not a persisted preference.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InteractiveMode {
    /// Run normal agent turns with the configured execution authority.
    #[default]
    Execute,
    /// Create or refine durable plans with the structurally restricted Plan Mode.
    Plan,
}

impl InteractiveMode {
    /// Stable label used in compact terminal chrome.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Plan => "plan",
        }
    }
}

/// Exact user-facing `/plan` command grammar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanCommand {
    /// Toggle between Execute and Plan modes.
    Toggle,
    /// Enter Plan mode.
    On,
    /// Return to Execute mode without clearing the selected plan.
    Off,
    /// Show process-local mode and selection state.
    Status,
    /// Enter Plan mode and clear the selected plan without discarding it.
    New,
    /// List current-session plans.
    List,
    /// Select one same-session actionable plan.
    Use {
        /// Stable plan identifier supplied by the operator.
        plan_id: String,
    },
    /// Show an explicit plan or the current selection.
    Show {
        /// Optional explicit plan identifier; absent means the current selection.
        plan_id: Option<String>,
    },
    /// Approve the current selected draft.
    Approve,
    /// Discard the current selected draft or approved plan.
    Discard,
    /// Execute the selected approved plan, prompting when no strategy was supplied.
    Execute {
        /// Explicit direct or Goal Mode strategy, or `None` to open the picker.
        strategy: Option<PlanExecutionStrategy>,
    },
}

/// Fully resolved lifecycle operation passed to the application host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanHostCommand {
    /// List current-session plans.
    List,
    /// Validate and select one exact plan.
    Use {
        /// Stable plan identifier supplied by the operator.
        plan_id: String,
    },
    /// Show one exact plan.
    Show {
        /// Stable plan identifier resolved by the terminal.
        plan_id: String,
    },
    /// Approve one exact optimistic draft revision.
    Approve {
        /// Stable selected plan identifier.
        plan_id: String,
        /// Expected canonical draft revision.
        revision: u64,
    },
    /// Discard one exact optimistic draft revision.
    Discard {
        /// Stable selected plan identifier.
        plan_id: String,
        /// Expected canonical draft revision.
        revision: u64,
    },
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
    /// Typed plan lifecycle operation whose policy and persistence remain host-owned.
    Plan(PlanHostCommand),
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
    /// Typed terminal plan workflow command.
    Plan(PlanCommand),
    /// Known command with invalid bounded syntax.
    Invalid(String),
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
        command
            if command.strip_prefix("/plan").is_some_and(|rest| {
                rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace)
            }) =>
        {
            parse_plan_command(command)
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

pub(crate) const DEFAULT_GOAL_ITERATIONS: u16 = 5;
const MAX_GOAL_ITERATIONS: u16 = 50;

fn parse_plan_command(input: &str) -> InteractiveCommand {
    let mut words = input.split_whitespace();
    debug_assert_eq!(words.next(), Some("/plan"));
    let subcommand = words.next();
    let parsed = match subcommand {
        None => Some(PlanCommand::Toggle),
        Some("on") if words.next().is_none() => Some(PlanCommand::On),
        Some("off") if words.next().is_none() => Some(PlanCommand::Off),
        Some("status") if words.next().is_none() => Some(PlanCommand::Status),
        Some("new") if words.next().is_none() => Some(PlanCommand::New),
        Some("list") if words.next().is_none() => Some(PlanCommand::List),
        Some("use") => {
            let plan_id = words.next();
            if let Some(plan_id) = plan_id
                && words.next().is_none()
            {
                Some(PlanCommand::Use {
                    plan_id: plan_id.to_owned(),
                })
            } else {
                return InteractiveCommand::Invalid("Usage: /plan use PLAN_ID".into());
            }
        }
        Some("show") => {
            let plan_id = words.next().map(str::to_owned);
            if words.next().is_none() {
                Some(PlanCommand::Show { plan_id })
            } else {
                return InteractiveCommand::Invalid("Usage: /plan show [PLAN_ID]".into());
            }
        }
        Some("approve") if words.next().is_none() => Some(PlanCommand::Approve),
        Some("discard") if words.next().is_none() => Some(PlanCommand::Discard),
        Some("execute") => match words.next() {
            None => Some(PlanCommand::Execute { strategy: None }),
            Some("direct") if words.next().is_none() => Some(PlanCommand::Execute {
                strategy: Some(PlanExecutionStrategy::Direct),
            }),
            Some("goal") => {
                let iterations = match words.next() {
                    None => DEFAULT_GOAL_ITERATIONS,
                    Some(value) => match value.parse::<u16>() {
                        Ok(value @ 1..=MAX_GOAL_ITERATIONS) => value,
                        _ => {
                            return InteractiveCommand::Invalid(
                                "Goal iterations must be between 1 and 50.".into(),
                            );
                        }
                    },
                };
                if words.next().is_some() {
                    return InteractiveCommand::Invalid(
                        "Usage: /plan execute [direct|goal [ITERATIONS]]".into(),
                    );
                }
                Some(PlanCommand::Execute {
                    strategy: Some(PlanExecutionStrategy::Goal {
                        max_iterations: iterations,
                    }),
                })
            }
            _ => {
                return InteractiveCommand::Invalid(
                    "Usage: /plan execute [direct|goal [ITERATIONS]]".into(),
                );
            }
        },
        _ => None,
    };
    parsed.map_or_else(
        || {
            InteractiveCommand::Invalid(
                "Usage: /plan [on|off|status|new|list|use|show|approve|discard|execute]".into(),
            )
        },
        InteractiveCommand::Plan,
    )
}

/// Process-local selected-plan change returned by a trusted host operation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PlanSelectionUpdate {
    /// Leave the current selected plan unchanged.
    #[default]
    Unchanged,
    /// Replace the selection with this canonical plan record.
    Set(Box<PlanRecord>),
    /// Select this canonical plan and enter Plan mode after a successful `/plan use`.
    Use(Box<PlanRecord>),
    /// Clear the selection.
    Clear,
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
    /// Canonical selected-plan update produced by the host operation.
    pub plan_selection: PlanSelectionUpdate,
    /// Whether the FIFO may continue after applying this command result.
    pub continue_queue: bool,
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
            plan_selection: PlanSelectionUpdate::Unchanged,
            continue_queue: true,
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
    /// Canonical plan created or refined by this run, when any.
    pub plan_selection: PlanSelectionUpdate,
}

/// Controlled request to consume one selected approved plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractivePlanExecutionRequest {
    /// Exact durable session.
    pub session_id: String,
    /// Stable selected plan identity.
    pub plan_id: String,
    /// Exact optimistic lifecycle revision selected by the operator.
    pub revision: u64,
    /// Explicit direct or bounded Goal Mode handoff.
    pub strategy: PlanExecutionStrategy,
}

/// Terminal disposition of one controlled approved-plan handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostPlanExecutionOutcome {
    /// Cancellation won before consumption; Plan mode and selection remain active.
    CancelledBeforeStart,
    /// The operation failed and durable readback proved the approved plan was not consumed.
    FailedBeforeConsumption(String),
    /// The selected execution strategy completed normally.
    Completed,
    /// The operator cooperatively cancelled after consumption.
    CancelledAfterConsumption,
    /// Execution failed after consumption; the bounded message is safe to display.
    FailedAfterConsumption(String),
    /// Consumption is durable, but the connection closed before terminal evidence arrived.
    ConsumedOutcomeUnknown(String),
    /// The connection or durable readback failed, so consumption could not be determined.
    OutcomeUnknown(String),
}

/// Controlled plan execution result with explicit pre/post-consumption semantics.
#[derive(Clone, Debug)]
pub struct HostPlanExecutionResult {
    /// Canonical approved or consumed plan returned by the application service.
    pub plan: PlanRecord,
    /// Human presentation for the direct run or Goal Mode result.
    pub document: PresentationDocument,
    /// Terminal completion, cancellation, or post-consumption failure.
    pub outcome: HostPlanExecutionOutcome,
    /// Footer state refreshed after the operation reached a terminal state.
    pub footer: FooterState,
    /// Canonical selection update (`Set` before consumption, otherwise `Clear`).
    pub plan_selection: PlanSelectionUpdate,
}

/// One-use response returned from an approval or `user.ask` overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptResponse {
    /// Operator supplied a bounded answer.
    Answer(String),
    /// Operator cancelled or submitted a blank answer.
    Cancelled,
}

/// Presentation and interaction class for one focus-taking prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractivePromptKind {
    /// Policy requires a request-bound effect approval.
    Approval,
    /// A tool needs bounded operator input but grants no effect authority.
    UserInput,
    /// A local interface picker such as session or plan selection.
    Choice,
}

/// Focus-taking prompt sent by the trusted runtime bridge to the TUI.
pub struct InteractivePrompt {
    /// One-use prompt identity bound by the host to the connection and run.
    pub id: String,
    /// Typed presentation and interaction behavior.
    pub kind: InteractivePromptKind,
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
    /// Startup direct-execution acknowledgement reached a terminal result.
    SandboxBoundaryAcknowledgement(Result<Option<SandboxBoundaryMode>, String>),
}

/// Terminal result of one serialized background operation.
pub enum OperationResult {
    /// Application command completed.
    Command(HostCommandResult),
    /// Model run completed or was cooperatively cancelled.
    Run(HostRunResult),
    /// Approved-plan execution reached a post-consumption terminal state.
    PlanExecution(HostPlanExecutionResult),
}

/// Embedded and worker-backed application boundary consumed by the TUI.
#[async_trait]
pub trait InteractiveHost: Send + Sync {
    /// Resolve session, transcript, preferences, history, completions, and footer.
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String>;

    /// Acknowledge one configured direct-execution boundary for the active TUI session.
    async fn acknowledge_sandbox_boundary(
        &self,
        session_id: &str,
        mode: SandboxBoundaryMode,
    ) -> Result<(), String>;

    /// Execute one typed application command without writing to the terminal.
    async fn execute_command(
        &self,
        command: RuntimeCommand,
        session_id: &str,
        sticky_skills: &[String],
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostCommandResult, String>;

    /// Execute one controlled model turn and emit ordered policy-released events.
    async fn run_turn(
        &self,
        request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostRunResult, String>;

    /// Consume and execute one selected approved plan under a controlled channel.
    async fn run_plan_execution(
        &self,
        request: InteractivePlanExecutionRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostPlanExecutionResult, String>;

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
    /// Dynamic inline viewport with finalized output in native terminal scrollback.
    #[default]
    Inline,
    /// Full alternate screen with an application-owned transcript viewport.
    Alternate,
}

/// User-visible TUI startup options.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiOptions {
    /// Durable session selection.
    pub bootstrap: BootstrapRequest,
    /// Explicit screen mode.
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
