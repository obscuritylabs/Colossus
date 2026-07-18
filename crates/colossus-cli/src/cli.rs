use super::*;

#[derive(Parser)]
#[command(
    name = "colossus",
    version,
    about = "Auditable Colossus workflow runtime"
)]
pub(super) struct Cli {
    /// Fresh Rust YAML configuration path.
    #[arg(long, default_value = ".colossus/config.yaml")]
    pub(super) config: PathBuf,
    /// Handling for policy decisions that require operator approval.
    #[arg(long, value_enum)]
    pub(super) approval_mode: Option<ApprovalMode>,
    /// Output format for structured commands. Auto is human on a terminal and JSON when piped.
    #[arg(long, value_enum, default_value_t = OutputMode::Auto)]
    pub(super) output: OutputMode,
    /// Preserve terminal scrollback by using Ratatui's inline viewport.
    #[arg(long, global = true)]
    pub(super) no_alt_screen: bool,
    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum ApprovalMode {
    /// Fail closed without prompting (default outside the interactive TUI).
    Deny,
    /// Prompt on the terminal for every approval obligation.
    Ask,
    /// Auto-approve only low-risk shell effects after model-assisted review.
    RiskAuto,
    /// Grant approval obligations automatically without expanding policy permissions.
    FullAccess,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum OutputMode {
    /// Render human tables and cards on terminals, JSON when redirected.
    #[default]
    Auto,
    /// Always render human tables, cards, and Markdown.
    Human,
    /// Always emit stable machine-readable JSON.
    Json,
}

pub(super) static OUTPUT_MODE: AtomicU8 = AtomicU8::new(0);
pub(super) static TERMINAL_PREFERENCES: OnceLock<Mutex<TerminalPreferences>> = OnceLock::new();

pub(super) const TERMINAL_HISTORY_CAPACITY: usize = 1_000;
pub(super) const TERMINAL_COMPLETIONS: &[&str] = &[
    "/help",
    "/tui prefs",
    "/tui save",
    "/tui reset",
    "/theme",
    "/theme list",
    "/theme preview",
    "/theme validate",
    "/theme scaffold",
    "/theme reset",
    "/stream on",
    "/stream raw",
    "/stream off",
    "/events compact",
    "/events verbose",
    "/events off",
    "/reasoning on",
    "/reasoning off",
    "/transcript comfortable",
    "/transcript compact",
    "/multiline on",
    "/multiline off",
    "/multiline toggle",
    "/trace",
    "/resume",
    "/sessions",
    "/session show",
    "/session new",
    "/session resume",
    "/work",
    "/tasks",
    "/decisions",
    "/plans",
    "/goals",
    "/goal",
    "/agents",
    "/agents drain",
    "/memories",
    "/memory search",
    "/research",
    "/research list",
    "/telemetry",
    "/telemetry metrics",
    "/skills",
    "/skill active",
    "/skill use",
    "/skill clear",
    "/skill show",
    "/skill resources",
    "/skill read",
    "/packs list",
    "/packs show",
    "/packs verify",
    "/packs install",
    "/packs enable",
    "/packs disable",
    "/packs uninstall",
    "/packs call",
    "/packs trust list",
    "/packs trust add",
    "/collections verify",
    "/collections install",
    "/registry pull",
    "/registry push",
    "/bundle verify",
    "/integrations",
    "/integration show",
    "/integration call",
    "/integration disconnect",
    "/mcp servers",
    "/mcp tools",
    "/mcp call",
    "/context status",
    "/context list",
    "/context compact",
    "/context restore",
    "/workflow list",
    "/workflow status",
    "/workflow schedule list",
    "/workflow schedule show",
    "/workflow schedule enable",
    "/workflow schedule disable",
    "/workflow schedule tick",
    "/workflow webhook list",
    "/workflow webhook show",
    "/workflow webhook enable",
    "/workflow webhook disable",
    "/workflow subscription list",
    "/workflow subscription show",
    "/workflow subscription enable",
    "/workflow subscription disable",
    "/workflow subscription tick",
    "/audit verify",
    "/projection status",
    "/tools",
    "/exit",
];
#[cfg(windows)]
pub(super) const WINDOWS_MAIN_STACK_BYTES: usize = 8 * 1024 * 1024;
