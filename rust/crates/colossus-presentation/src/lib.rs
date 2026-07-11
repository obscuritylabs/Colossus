//! Event-sourced presentation preferences and pure semantic terminal rendering.

use colossus_contracts::{
    Actor, ContextStatus, EventClassification, ExecutionContext, NewEvent, ProviderEvent, RunEvent,
    RunEventEnvelope, RunPhase, ToolCall, ToolResult, WorkStateSnapshot,
};
pub use colossus_contracts::{
    EventDisplayMode, ReplPreferences, StreamDisplayMode, ThemeName, TranscriptDensity,
};
use colossus_ports::{EventJournal, PresentationRepository, StoreError};
use serde_json::{Value, json};
use std::sync::Arc;
use thiserror::Error;

const PREFERENCES_STREAM: &str = "presentation:repl";
const PREFERENCES_UPDATED: &str = "presentation.preferences.updated.v1";
const HISTORY_STREAM: &str = "presentation:history";
const HISTORY_APPENDED: &str = "presentation.history.appended.v1";
const MAX_HISTORY_ENTRIES: usize = 1_000;
const MAX_HISTORY_ENTRY_BYTES: usize = 1024 * 1024;
const COMPACT_PREVIEW_CHARS: usize = 240;
const VERBOSE_PREVIEW_CHARS: usize = 8 * 1024;

/// Presentation rendering failure.
#[derive(Debug, Error)]
pub enum PresentationError {
    /// Released content could not be rendered safely.
    #[error("presentation rendering failed: {0}")]
    Invalid(String),
}

/// Terminal RGB value shared by Reedline prompts and semantic transcript palettes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RgbColor {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

impl RgbColor {
    const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

#[derive(Clone, Copy)]
struct TextStyle {
    foreground: Option<RgbColor>,
    bold: bool,
    dim: bool,
    italic: bool,
}

impl TextStyle {
    const fn color(foreground: RgbColor) -> Self {
        Self {
            foreground: Some(foreground),
            bold: false,
            dim: false,
            italic: false,
        }
    }

    const fn plain() -> Self {
        Self {
            foreground: None,
            bold: false,
            dim: false,
            italic: false,
        }
    }

    const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    fn paint(self, text: &str, enabled: bool) -> String {
        if !enabled {
            return text.into();
        }
        let mut codes = Vec::with_capacity(4);
        if self.bold {
            codes.push("1".into());
        }
        if self.dim {
            codes.push("2".into());
        }
        if self.italic {
            codes.push("3".into());
        }
        if let Some(color) = self.foreground {
            codes.push(format!("38;2;{};{};{}", color.red, color.green, color.blue));
        }
        if codes.is_empty() {
            text.into()
        } else {
            format!("\x1b[{}m{text}\x1b[0m", codes.join(";"))
        }
    }
}

/// Complete built-in data-only palette for one terminal theme.
#[derive(Clone, Copy)]
pub struct TerminalPalette {
    prompt_left: Option<RgbColor>,
    prompt_right: Option<RgbColor>,
    indicator: Option<RgbColor>,
    continuation: Option<RgbColor>,
    assistant: TextStyle,
    activity: TextStyle,
    thinking: TextStyle,
    tool: TextStyle,
    success: TextStyle,
    warning: TextStyle,
    error: TextStyle,
    meta: TextStyle,
    spinner_frames: &'static [&'static str],
}

impl TerminalPalette {
    /// Resolve one strict built-in palette.
    pub const fn for_theme(theme: ThemeName) -> Self {
        match theme {
            ThemeName::Default => Self {
                prompt_left: Some(RgbColor::new(95, 215, 255)),
                prompt_right: Some(RgbColor::new(127, 135, 144)),
                indicator: Some(RgbColor::new(95, 215, 255)),
                continuation: Some(RgbColor::new(127, 135, 144)),
                assistant: TextStyle::color(RgbColor::new(230, 237, 243)),
                activity: TextStyle::color(RgbColor::new(127, 135, 144)).dim(),
                thinking: TextStyle::color(RgbColor::new(95, 215, 255)).italic(),
                tool: TextStyle::color(RgbColor::new(88, 166, 255)).bold(),
                success: TextStyle::color(RgbColor::new(158, 206, 106)),
                warning: TextStyle::color(RgbColor::new(255, 223, 93)).bold(),
                error: TextStyle::color(RgbColor::new(255, 95, 95)).bold(),
                meta: TextStyle::color(RgbColor::new(127, 135, 144)).dim(),
                spinner_frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            },
            ThemeName::Mono => Self {
                prompt_left: None,
                prompt_right: None,
                indicator: None,
                continuation: None,
                assistant: TextStyle::plain(),
                activity: TextStyle::plain().dim(),
                thinking: TextStyle::plain().italic().dim(),
                tool: TextStyle::plain().bold(),
                success: TextStyle::plain(),
                warning: TextStyle::plain().bold(),
                error: TextStyle::plain().bold(),
                meta: TextStyle::plain().dim(),
                spinner_frames: &["-", "\\", "|", "/"],
            },
            ThemeName::HighContrast => Self {
                prompt_left: Some(RgbColor::new(255, 255, 0)),
                prompt_right: Some(RgbColor::new(255, 255, 255)),
                indicator: Some(RgbColor::new(255, 255, 0)),
                continuation: Some(RgbColor::new(255, 255, 0)),
                assistant: TextStyle::color(RgbColor::new(255, 255, 255)),
                activity: TextStyle::color(RgbColor::new(255, 255, 255)).bold(),
                thinking: TextStyle::color(RgbColor::new(255, 255, 0)).bold(),
                tool: TextStyle::color(RgbColor::new(0, 255, 255)).bold(),
                success: TextStyle::color(RgbColor::new(255, 255, 255)).bold(),
                warning: TextStyle::color(RgbColor::new(255, 255, 0)).bold(),
                error: TextStyle::color(RgbColor::new(255, 0, 0)).bold(),
                meta: TextStyle::color(RgbColor::new(255, 255, 255)),
                spinner_frames: &["◜", "◠", "◝", "◞", "◡", "◟"],
            },
            ThemeName::Carrot => Self {
                prompt_left: Some(RgbColor::new(255, 175, 95)),
                prompt_right: Some(RgbColor::new(184, 139, 106)),
                indicator: Some(RgbColor::new(255, 135, 0)),
                continuation: Some(RgbColor::new(215, 135, 95)),
                assistant: TextStyle::color(RgbColor::new(255, 240, 223)),
                activity: TextStyle::color(RgbColor::new(184, 139, 106)).dim(),
                thinking: TextStyle::color(RgbColor::new(255, 175, 95)).italic(),
                tool: TextStyle::color(RgbColor::new(255, 135, 0)).bold(),
                success: TextStyle::color(RgbColor::new(159, 215, 122)),
                warning: TextStyle::color(RgbColor::new(255, 223, 93)).bold(),
                error: TextStyle::color(RgbColor::new(255, 95, 95)).bold(),
                meta: TextStyle::color(RgbColor::new(184, 139, 106)).dim(),
                spinner_frames: &[
                    "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█", "▉", "▊", "▋", "▌", "▍", "▎",
                ],
            },
            ThemeName::Hacker => Self {
                prompt_left: Some(RgbColor::new(0, 255, 102)),
                prompt_right: Some(RgbColor::new(63, 191, 106)),
                indicator: Some(RgbColor::new(0, 255, 102)),
                continuation: Some(RgbColor::new(0, 170, 68)),
                assistant: TextStyle::color(RgbColor::new(215, 255, 215)),
                activity: TextStyle::color(RgbColor::new(63, 191, 106)).dim(),
                thinking: TextStyle::color(RgbColor::new(0, 255, 102)).italic(),
                tool: TextStyle::color(RgbColor::new(0, 215, 255)).bold(),
                success: TextStyle::color(RgbColor::new(0, 255, 102)),
                warning: TextStyle::color(RgbColor::new(255, 215, 95)).bold(),
                error: TextStyle::color(RgbColor::new(255, 95, 95)).bold(),
                meta: TextStyle::color(RgbColor::new(63, 191, 106)).dim(),
                spinner_frames: &["░", "▒", "▓", "█", "▓", "▒"],
            },
        }
    }

    /// Left-prompt color, or reset for the mono palette.
    pub const fn prompt_left(self) -> Option<RgbColor> {
        self.prompt_left
    }

    /// Right-prompt color, or reset for the mono palette.
    pub const fn prompt_right(self) -> Option<RgbColor> {
        self.prompt_right
    }

    /// Primary prompt-indicator color, or reset for the mono palette.
    pub const fn indicator(self) -> Option<RgbColor> {
        self.indicator
    }

    /// Continuation-indicator color, or reset for the mono palette.
    pub const fn continuation(self) -> Option<RgbColor> {
        self.continuation
    }

    /// Render the theme's bounded spinner frame for one elapsed duration.
    pub fn activity_frame(self, elapsed_seconds: f64, color: bool) -> String {
        let index = ((elapsed_seconds.max(0.0) * 10.0) as usize) % self.spinner_frames.len();
        self.activity.paint(self.spinner_frames[index], color)
    }
}

fn validate_preferences(preferences: &ReplPreferences) -> Result<(), StoreError> {
    if preferences.schema_version != 1 {
        return Err(StoreError::Adapter("schema_version must be 1".into()));
    }
    Ok(())
}

/// Immutable-journal implementation of the presentation preference port.
pub struct EventSourcedPresentationRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedPresentationRepository {
    /// Bind the global REPL presentation profile to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl PresentationRepository for EventSourcedPresentationRepository {
    fn load(&self) -> Result<ReplPreferences, StoreError> {
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        let Some(event) = events.last() else {
            return Ok(ReplPreferences::default());
        };
        if event.event_type != PREFERENCES_UPDATED {
            return Err(StoreError::Verification(
                "presentation stream contains an unknown event".into(),
            ));
        }
        let payload = self.journal.decrypt_payload(event)?;
        let preferences: ReplPreferences = serde_json::from_value(
            payload
                .get("preferences")
                .cloned()
                .ok_or_else(|| StoreError::Verification("preferences payload is absent".into()))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;
        validate_preferences(&preferences)?;
        Ok(preferences)
    }

    fn save(
        &self,
        preferences: ReplPreferences,
        actor: Actor,
    ) -> Result<ReplPreferences, StoreError> {
        validate_preferences(&preferences)?;
        let expected_stream_version =
            u64::try_from(self.journal.read_stream(PREFERENCES_STREAM)?.len())
                .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: PREFERENCES_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: PREFERENCES_UPDATED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: PREFERENCES_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"preferences": &preferences}),
        })?;
        Ok(preferences)
    }

    fn list_history(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        if !(1..=MAX_HISTORY_ENTRIES).contains(&limit) {
            return Err(StoreError::Adapter(format!(
                "history limit must be between 1 and {MAX_HISTORY_ENTRIES}"
            )));
        }
        let events = self.journal.read_stream(HISTORY_STREAM)?;
        let skip = events.len().saturating_sub(limit);
        events
            .iter()
            .skip(skip)
            .map(|event| {
                if event.event_type != HISTORY_APPENDED {
                    return Err(StoreError::Verification(
                        "presentation history contains an unknown event".into(),
                    ));
                }
                let payload = self.journal.decrypt_payload(event)?;
                let entry = payload
                    .get("entry")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StoreError::Verification("presentation history entry is absent".into())
                    })?;
                validate_history_entry(entry)?;
                Ok(entry.into())
            })
            .collect()
    }

    fn append_history(&self, entry: String, actor: Actor) -> Result<String, StoreError> {
        validate_history_entry(&entry)?;
        let events = self.journal.read_stream(HISTORY_STREAM)?;
        if let Some(event) = events.last() {
            if event.event_type != HISTORY_APPENDED {
                return Err(StoreError::Verification(
                    "presentation history contains an unknown event".into(),
                ));
            }
            let payload = self.journal.decrypt_payload(event)?;
            if payload.get("entry").and_then(Value::as_str) == Some(entry.as_str()) {
                return Ok(entry);
            }
        }
        let expected_stream_version =
            u64::try_from(events.len()).map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: HISTORY_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: HISTORY_APPENDED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: HISTORY_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"entry": &entry}),
        })?;
        Ok(entry)
    }
}

fn validate_history_entry(entry: &str) -> Result<(), StoreError> {
    if entry.trim().is_empty() || entry.len() > MAX_HISTORY_ENTRY_BYTES {
        return Err(StoreError::Adapter(format!(
            "history entry must be nonempty and at most {MAX_HISTORY_ENTRY_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Pure semantic renderer over already released contracts.
pub struct SemanticRenderer {
    preferences: ReplPreferences,
    color: bool,
}

impl SemanticRenderer {
    /// Create a renderer for one immutable preference snapshot.
    pub fn new(preferences: ReplPreferences) -> Self {
        Self {
            preferences,
            color: false,
        }
    }

    /// Enable or disable ANSI styling without changing semantic content.
    pub const fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Apply the active assistant palette to released model text.
    pub fn assistant_text(&self, text: &str) -> String {
        TerminalPalette::for_theme(self.preferences.theme)
            .assistant
            .paint(text, self.color)
    }

    fn label(&self, name: &str) -> String {
        let text = match self.preferences.theme {
            ThemeName::Default | ThemeName::Carrot | ThemeName::Hacker => format!("[{name}]"),
            ThemeName::Mono => format!("{name}:"),
            ThemeName::HighContrast => format!("{}:", name.to_ascii_uppercase()),
        };
        self.label_style(name).paint(&text, self.color)
    }

    fn label_style(&self, name: &str) -> TextStyle {
        let palette = TerminalPalette::for_theme(self.preferences.theme);
        match name {
            "activity" => palette.activity,
            "thinking" => palette.thinking,
            "usage" => palette.meta,
            "approval" => palette.warning,
            "risk" | "error" => palette.error,
            "done" => palette.success,
            _ => palette.tool,
        }
    }

    /// Render current session work without exposing repository internals.
    pub fn work_state(&self, state: &WorkStateSnapshot) -> String {
        let summary = format!(
            "{} session={} tasks={}/{} decisions={} plans={} goals={} agents={}",
            self.label("work"),
            state.session_id,
            state.open_task_count,
            state.tasks.len(),
            state.active_decisions.len(),
            state.actionable_plans.len(),
            state.current_goals.len(),
            state.current_subagents.len()
        );
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            return summary;
        }
        let mut lines = vec![summary];
        lines.extend(
            state
                .tasks
                .iter()
                .filter(|task| {
                    !matches!(
                        task.status,
                        colossus_contracts::TaskStatus::Completed
                            | colossus_contracts::TaskStatus::Cancelled
                    )
                })
                .map(|task| format!("  task [{}] {}", task.id, task.title)),
        );
        lines.extend(
            state
                .current_goals
                .iter()
                .map(|goal| format!("  goal [{}] {}", goal.id, goal.objective)),
        );
        lines.join("\n")
    }

    /// Render context budget and compaction state.
    pub fn context_status(&self, status: &ContextStatus) -> String {
        format!(
            "{} session={} messages={} tokens={}/{} compacted={} snapshot={}",
            self.label("context"),
            status.session_id,
            status.message_count,
            status.token_estimate,
            status.context_window_tokens,
            status.compacted,
            status.active_snapshot_id.as_deref().unwrap_or("none")
        )
    }

    /// Render one already policy-released provider event.
    ///
    /// Visible model deltas are streamed separately and final output is not repeated. Safe
    /// reasoning summaries remain independently configurable from tool/activity events.
    pub fn provider_event(
        &self,
        event: &ProviderEvent,
    ) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        let rendered = match event {
            ProviderEvent::ModelDelta { .. } | ProviderEvent::FinalOutput { .. } => None,
            ProviderEvent::ReasoningSummary { summary } if self.preferences.show_reasoning => {
                Some(format!("{} {summary}", self.label("thinking")))
            }
            ProviderEvent::ReasoningSummary { .. } => None,
            ProviderEvent::ToolCallRequested { .. } => None,
            ProviderEvent::Usage { usage } => match self.preferences.events_mode {
                EventDisplayMode::Verbose => Some(format!(
                    "{} input={} output={} total={} cached={} reasoning={}",
                    self.label("usage"),
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.total_tokens,
                    usage
                        .cached_input_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string()),
                    usage
                        .reasoning_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string())
                )),
                EventDisplayMode::Compact | EventDisplayMode::Off => None,
            },
        };
        Ok(rendered)
    }

    /// Render one ordered application-level run event.
    pub fn run_event(&self, event: &RunEvent) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        match event {
            RunEvent::Provider { event } => self.provider_event(event),
            RunEvent::Phase {
                phase,
                turn,
                action,
                elapsed_seconds,
            } => Ok(self.render_phase(*phase, *turn, action.as_deref(), *elapsed_seconds)),
            RunEvent::ToolStarted {
                turn,
                call,
                elapsed_seconds,
            } => self.render_tool_started(*turn, call, *elapsed_seconds),
            RunEvent::ToolCompleted {
                turn,
                result,
                duration_seconds,
                elapsed_seconds,
            } => self.render_tool_completed(*turn, result, *duration_seconds, *elapsed_seconds),
            RunEvent::Error {
                code,
                message,
                recoverable,
                turn,
                elapsed_seconds,
            } => Ok(Some(self.with_detail(
                format!(
                    "{} code={} recoverable={} turn={} elapsed={:.2}s",
                    self.label("error"),
                    code,
                    if *recoverable { "yes" } else { "no" },
                    turn.map_or_else(|| "none".into(), |value| value.to_string()),
                    elapsed_seconds,
                ),
                Some(bounded_text(message, COMPACT_PREVIEW_CHARS)),
            ))),
        }
    }

    /// Render a correlated run event, including bounded provenance in verbose mode.
    pub fn run_event_envelope(
        &self,
        envelope: &RunEventEnvelope,
    ) -> Result<Option<String>, PresentationError> {
        let Some(rendered) = self.run_event(&envelope.event)? else {
            return Ok(None);
        };
        if self.preferences.events_mode == EventDisplayMode::Verbose {
            Ok(Some(format!(
                "run={} session={} {rendered}",
                envelope.run_id, envelope.session_id
            )))
        } else {
            Ok(Some(rendered))
        }
    }

    fn render_phase(
        &self,
        phase: RunPhase,
        turn: Option<u16>,
        action: Option<&str>,
        elapsed_seconds: f64,
    ) -> Option<String> {
        if phase == RunPhase::Completed && self.preferences.events_mode == EventDisplayMode::Off {
            return None;
        }
        let phase_name = match phase {
            RunPhase::Preparing => "preparing",
            RunPhase::WaitingForModel => "waiting_for_model",
            RunPhase::Responding => "responding",
            RunPhase::Completed => "completed",
        };
        match self.preferences.events_mode {
            EventDisplayMode::Verbose => Some(format!(
                "{} phase={phase_name} turn={} action={} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                turn.map_or_else(|| "none".into(), |value| value.to_string()),
                action.unwrap_or("none")
            )),
            EventDisplayMode::Compact | EventDisplayMode::Off => Some(format!(
                "{} {phase_name}{} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                action.map_or_else(String::new, |value| format!(" {value}"))
            )),
        }
    }

    fn render_tool_started(
        &self,
        turn: u16,
        call: &ToolCall,
        elapsed_seconds: f64,
    ) -> Result<Option<String>, PresentationError> {
        if self.preferences.events_mode == EventDisplayMode::Off {
            return Ok(Some(format!(
                "{} using {} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                call.name
            )));
        }
        let family = ToolFamily::from_name(&call.name);
        let detail = summarize_value(&call.arguments, family.keys());
        let rendered = match self.preferences.events_mode {
            EventDisplayMode::Compact => self.with_detail(
                format!(
                    "{} start {} elapsed={elapsed_seconds:.2}s",
                    self.label(family.label()),
                    call.name,
                ),
                detail,
            ),
            EventDisplayMode::Verbose => format!(
                "{} start name={} call_id={} turn={} elapsed={elapsed_seconds:.2}s arguments={}",
                self.label(family.label()),
                call.name,
                call.call_id,
                turn,
                bounded_json(&call.arguments, VERBOSE_PREVIEW_CHARS)?
            ),
            EventDisplayMode::Off => unreachable!("handled above"),
        };
        Ok(Some(rendered))
    }

    fn render_tool_completed(
        &self,
        turn: u16,
        result: &ToolResult,
        duration_seconds: f64,
        elapsed_seconds: f64,
    ) -> Result<Option<String>, PresentationError> {
        let parsed = serde_json::from_str::<Value>(&result.output)
            .unwrap_or_else(|_| Value::String(result.output.clone()));
        let family = ToolFamily::from_name(&result.name);
        let recoverable = parsed
            .pointer("/error/recoverable")
            .and_then(Value::as_bool);
        let failed = result.exit_code != 0 || parsed.get("error").is_some();
        if self.preferences.events_mode == EventDisplayMode::Off && !failed {
            return Ok(None);
        }
        let status = if failed {
            if recoverable == Some(true) {
                "recoverable_error"
            } else {
                "failed"
            }
        } else {
            "ok"
        };
        let detail = if failed {
            parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(|message| bounded_text(message, COMPACT_PREVIEW_CHARS))
        } else {
            summarize_value(&parsed, family.keys())
        };
        let rendered = match self.preferences.events_mode {
            EventDisplayMode::Verbose => format!(
                "{} complete name={} call_id={} turn={} status={} exit={} duration={duration_seconds:.2}s elapsed={elapsed_seconds:.2}s output={}",
                self.label(family.label()),
                result.name,
                result.call_id,
                turn,
                status,
                result.exit_code,
                bounded_json(&parsed, VERBOSE_PREVIEW_CHARS)?
            ),
            EventDisplayMode::Compact | EventDisplayMode::Off => self.with_detail(
                format!(
                    "{} complete {} status={} exit={} duration={duration_seconds:.2}s",
                    self.label(family.label()),
                    result.name,
                    status,
                    result.exit_code,
                ),
                detail,
            ),
        };
        Ok(Some(rendered))
    }

    fn with_detail(&self, summary: String, detail: Option<String>) -> String {
        let Some(detail) = detail else {
            return summary;
        };
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            format!("{summary} {detail}")
        } else {
            format!("{summary}\n  {detail}")
        }
    }

    /// Render generic released structured output according to transcript density.
    pub fn structured(&self, value: &Value) -> Result<String, PresentationError> {
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            serde_json::to_string(value)
        } else {
            serde_json::to_string_pretty(value)
        }
        .map_err(|error| PresentationError::Invalid(error.to_string()))
    }
}

#[derive(Clone, Copy)]
enum ToolFamily {
    Files,
    Shell,
    Git,
    Work,
    Context,
    Repository,
    Skills,
    Web,
    Mcp,
    Trace,
    Integrations,
    Packs,
    Generic,
}

impl ToolFamily {
    fn from_name(name: &str) -> Self {
        let prefix = name.split('.').next().unwrap_or(name);
        match prefix {
            "filesystem" | "patch" => Self::Files,
            "shell" | "process" => Self::Shell,
            "git" => Self::Git,
            "task" | "decision" | "plan" | "goal" | "agent" | "memory" => Self::Work,
            "context" => Self::Context,
            "repo" => Self::Repository,
            "skill" => Self::Skills,
            "web" | "docs" | "network" => Self::Web,
            "mcp" => Self::Mcp,
            "trace" | "telemetry" | "audit" => Self::Trace,
            "integration" => Self::Integrations,
            "pack" | "bundle" => Self::Packs,
            _ => Self::Generic,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Files => "file",
            Self::Shell => "shell",
            Self::Git => "git",
            Self::Work => "work",
            Self::Context => "context",
            Self::Repository => "repo",
            Self::Skills => "skill",
            Self::Web => "web",
            Self::Mcp => "mcp",
            Self::Trace => "trace",
            Self::Integrations => "integration",
            Self::Packs => "pack",
            Self::Generic => "tool",
        }
    }

    const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Files => &[
                "path",
                "bytes",
                "matches",
                "changed",
                "line_start",
                "line_end",
            ],
            Self::Shell => &["executable", "exit_code", "stdout", "stderr", "truncated"],
            Self::Git => &["branch", "commit", "path", "status", "summary", "stdout"],
            Self::Work => &["id", "status", "title", "objective", "open_task_count"],
            Self::Context => &[
                "session_id",
                "message_count",
                "token_estimate",
                "snapshot_id",
                "compacted",
            ],
            Self::Repository => &["path", "symbol", "matches", "files", "summary"],
            Self::Skills => &["name", "path", "status", "sha256"],
            Self::Web => &["url", "status", "title", "media_type", "bytes"],
            Self::Mcp => &["server", "tool", "status", "content"],
            Self::Trace => &["run_id", "event_count", "path", "status"],
            Self::Integrations => &["name", "tool", "status", "connected"],
            Self::Packs => &["name", "version", "trusted", "status", "publisher"],
            Self::Generic => &["id", "name", "status", "message"],
        }
    }
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut rendered = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        rendered.push('…');
    }
    rendered.replace(['\n', '\r'], " ")
}

fn bounded_json(value: &Value, max_chars: usize) -> Result<String, PresentationError> {
    serde_json::to_string(value)
        .map(|encoded| bounded_text(&encoded, max_chars))
        .map_err(|error| PresentationError::Invalid(error.to_string()))
}

fn summarize_value(value: &Value, keys: &[&str]) -> Option<String> {
    let parts = keys
        .iter()
        .filter_map(|key| {
            find_key(value, key, 0).map(|value| {
                format!(
                    "{key}={}",
                    bounded_text(&scalar_summary(value), COMPACT_PREVIEW_CHARS / 2)
                )
            })
        })
        .take(4)
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn find_key<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 2 {
        return None;
    }
    let object = value.as_object()?;
    if let Some(value) = object.get(key) {
        return Some(value);
    }
    object
        .values()
        .find_map(|value| find_key(value, key, depth.saturating_add(1)))
}

fn scalar_summary(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => format!("{} items", values.len()),
        Value::Object(values) => format!("{} fields", values.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EventDisplayMode, EventSourcedPresentationRepository, ReplPreferences, SemanticRenderer,
        StreamDisplayMode, TerminalPalette, ThemeName, TranscriptDensity,
    };
    use colossus_contracts::{
        Actor, ActorType, ProviderEvent, ProviderUsage, RunEvent, RunEventEnvelope, RunPhase,
        ToolCall, ToolResult, WorkStateSnapshot,
    };
    use colossus_ports::{EventJournal, PresentationRepository};
    use colossus_testkit::{InMemoryEventJournal, assert_presentation_repository_conformance};
    use std::sync::Arc;

    #[test]
    fn preferences_reconstruct_from_immutable_events_and_validate_schema() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(
            repository.load().expect("defaults"),
            ReplPreferences::default()
        );
        let preferences = ReplPreferences {
            theme: ThemeName::HighContrast,
            multiline: true,
            stream_mode: StreamDisplayMode::Off,
            events_mode: EventDisplayMode::Verbose,
            show_reasoning: false,
            transcript_density: TranscriptDensity::Compact,
            ..ReplPreferences::default()
        };
        repository
            .save(
                preferences.clone(),
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .expect("save");
        let restarted = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(restarted.load().expect("load"), preferences);
        let events = journal.read_stream("presentation:repl").expect("events");
        assert_eq!(events[0].event_type, "presentation.preferences.updated.v1");
        let invalid = ReplPreferences {
            schema_version: 2,
            ..ReplPreferences::default()
        };
        assert!(
            restarted
                .save(
                    invalid,
                    Actor {
                        actor_type: ActorType::User,
                        id: "terminal-user".into(),
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn event_sourced_repository_passes_shared_conformance() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(journal);
        assert_presentation_repository_conformance(&repository);
    }

    #[test]
    fn work_renderer_has_compact_and_comfortable_semantics() {
        let state = WorkStateSnapshot {
            session_id: "session-1".into(),
            tasks: Vec::new(),
            open_task_count: 0,
            active_decisions: Vec::new(),
            actionable_plans: Vec::new(),
            current_goals: Vec::new(),
            current_subagents: Vec::new(),
        };
        let compact = SemanticRenderer::new(ReplPreferences {
            transcript_density: TranscriptDensity::Compact,
            ..ReplPreferences::default()
        });
        assert_eq!(
            compact.work_state(&state),
            "[work] session=session-1 tasks=0/0 decisions=0 plans=0 goals=0 agents=0"
        );
        let comfortable = SemanticRenderer::new(ReplPreferences::default());
        assert!(
            comfortable
                .work_state(&state)
                .starts_with("[work] session=session-1")
        );
    }

    #[test]
    fn provider_events_respect_reasoning_events_and_theme_independently() {
        let renderer = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::HighContrast,
            events_mode: EventDisplayMode::Off,
            show_reasoning: true,
            ..ReplPreferences::default()
        });
        assert_eq!(
            renderer
                .provider_event(&ProviderEvent::ReasoningSummary {
                    summary: "safe summary".into(),
                })
                .expect("reasoning"),
            Some("THINKING: safe summary".into())
        );
        assert_eq!(
            renderer
                .provider_event(&ProviderEvent::ToolCallRequested {
                    call_id: "call-1".into(),
                    name: "filesystem.read".into(),
                    arguments: serde_json::json!({"path": "README.md"}),
                })
                .expect("tool"),
            None
        );

        let verbose = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::Mono,
            events_mode: EventDisplayMode::Verbose,
            ..ReplPreferences::default()
        });
        assert_eq!(
            verbose
                .provider_event(&ProviderEvent::Usage {
                    usage: ProviderUsage {
                        input_tokens: 4,
                        output_tokens: 2,
                        total_tokens: 6,
                        cached_input_tokens: Some(1),
                        reasoning_tokens: None,
                    },
                })
                .expect("usage"),
            Some("usage: input=4 output=2 total=6 cached=1 reasoning=unknown".into())
        );
        let correlated = verbose
            .run_event_envelope(&RunEventEnvelope {
                schema_version: 1,
                run_id: "run-1".into(),
                session_id: "session-1".into(),
                event: RunEvent::Phase {
                    phase: RunPhase::Preparing,
                    turn: Some(1),
                    action: None,
                    elapsed_seconds: 0.1,
                },
            })
            .expect("correlated")
            .expect("visible");
        assert!(correlated.starts_with("run=run-1 session=session-1"));
    }

    #[test]
    fn semantic_tool_families_errors_and_elapsed_phases_are_distinct() {
        let renderer = SemanticRenderer::new(ReplPreferences::default());
        for (name, label) in [
            ("filesystem.read", "[file]"),
            ("shell.run", "[shell]"),
            ("git.status", "[git]"),
            ("task.list", "[work]"),
            ("context.show", "[context]"),
            ("repo.map", "[repo]"),
            ("skill.read", "[skill]"),
            ("web.fetch", "[web]"),
            ("mcp.call", "[mcp]"),
            ("trace.export", "[trace]"),
            ("integration.invoke", "[integration]"),
            ("pack.verify", "[pack]"),
            ("echo", "[tool]"),
        ] {
            let rendered = renderer
                .run_event(&RunEvent::ToolStarted {
                    turn: 1,
                    call: ToolCall {
                        call_id: format!("call-{name}"),
                        name: name.into(),
                        arguments: serde_json::json!({"path": "README.md", "name": "demo"}),
                    },
                    elapsed_seconds: 0.25,
                })
                .expect("render")
                .expect("visible");
            assert!(rendered.starts_with(label), "{name}: {rendered}");
        }

        let completed = renderer
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-file".into(),
                    name: "filesystem.read".into(),
                    output: serde_json::json!({"path": "README.md", "bytes": 42}).to_string(),
                    exit_code: 0,
                },
                duration_seconds: 1.25,
                elapsed_seconds: 2.0,
            })
            .expect("render")
            .expect("visible");
        assert!(completed.contains("duration=1.25s"));
        assert!(completed.contains("path=README.md"));

        let quiet = SemanticRenderer::new(ReplPreferences {
            events_mode: EventDisplayMode::Off,
            ..ReplPreferences::default()
        });
        assert!(
            quiet
                .run_event(&RunEvent::ToolCompleted {
                    turn: 1,
                    result: ToolResult {
                        call_id: "call-ok".into(),
                        name: "echo".into(),
                        output: "ok".into(),
                        exit_code: 0,
                    },
                    duration_seconds: 0.1,
                    elapsed_seconds: 0.2,
                })
                .expect("quiet")
                .is_none()
        );
        let recoverable = quiet
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-error".into(),
                    name: "filesystem.read".into(),
                    output: serde_json::json!({
                        "error": {"message": "missing", "recoverable": true}
                    })
                    .to_string(),
                    exit_code: 1,
                },
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            })
            .expect("error")
            .expect("visible error");
        assert!(recoverable.contains("status=recoverable_error"));
        let phase = quiet
            .run_event(&RunEvent::Phase {
                phase: RunPhase::WaitingForModel,
                turn: Some(2),
                action: Some("model-x".into()),
                elapsed_seconds: 3.5,
            })
            .expect("phase")
            .expect("activity remains visible");
        assert!(phase.contains("waiting_for_model model-x elapsed=3.50s"));
    }

    #[test]
    fn every_builtin_palette_styles_tty_output_without_touching_redirected_text() {
        for theme in [
            ThemeName::Default,
            ThemeName::Mono,
            ThemeName::HighContrast,
            ThemeName::Carrot,
            ThemeName::Hacker,
        ] {
            let preferences = ReplPreferences {
                theme,
                ..ReplPreferences::default()
            };
            let event = RunEvent::Phase {
                phase: RunPhase::Preparing,
                turn: Some(1),
                action: None,
                elapsed_seconds: 0.5,
            };
            let redirected = SemanticRenderer::new(preferences.clone())
                .run_event(&event)
                .expect("redirected render")
                .expect("visible");
            assert!(!redirected.contains("\x1b["), "{}", theme.as_str());
            assert!(
                !SemanticRenderer::new(preferences.clone())
                    .assistant_text("connected")
                    .contains("\x1b[")
            );
            let terminal = SemanticRenderer::new(preferences)
                .with_color(true)
                .run_event(&event)
                .expect("terminal render")
                .expect("visible");
            assert!(terminal.contains("\x1b["), "{}", theme.as_str());
            let palette = TerminalPalette::for_theme(theme);
            assert_ne!(
                palette.activity_frame(0.0, false),
                palette.activity_frame(0.1, false),
                "{}",
                theme.as_str()
            );
        }
        let assistant = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::Hacker,
            ..ReplPreferences::default()
        })
        .with_color(true)
        .assistant_text("connected");
        assert!(assistant.contains("\x1b["));
        assert!(assistant.contains("connected"));
    }
}
