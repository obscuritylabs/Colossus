use crate::{PresentationBlock, PresentationDocument, PresentationTone};
use serde_json::Value;

/// Maximum Unicode scalar count in a shared human activity row.
pub const TOOL_ACTIVITY_SUMMARY_CHARS: usize = 320;
/// Maximum structured output inspected or retained in one detail document.
pub const TOOL_ACTIVITY_DETAIL_BYTES: usize = 64 * 1024;
/// Maximum logical lines in one detail document before terminal reflow.
pub const TOOL_ACTIVITY_DETAIL_LINES: usize = 160;
/// Maximum rows in one detail table.
pub const TOOL_ACTIVITY_DETAIL_ROWS: usize = 20;
/// Maximum columns in one detail table.
pub const TOOL_ACTIVITY_DETAIL_COLUMNS: usize = 8;

/// Display category derived from a registered tool identity, never its payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolActivityKind {
    /// Reading released file content.
    FileRead,
    /// Listing filesystem entries.
    FileList,
    /// Searching filesystem content.
    FileSearch,
    /// Writing file content.
    FileWrite,
    /// Applying a patch.
    Patch,
    /// Mapping repository files.
    RepositoryMap,
    /// Finding symbols or references.
    RepositorySearch,
    /// Summarizing repository source.
    RepositorySummary,
    /// Git inspection or mutation.
    Git,
    /// Shell or managed process activity.
    Process,
    /// A named MCP server/tool call.
    Mcp,
    /// Web or documentation retrieval.
    Web,
    /// Tasks, plans, goals, memory or delegated work.
    Work,
    /// Context budget or compaction activity.
    Context,
    /// Plugin activity.
    Plugin,
    /// Native computer activity.
    Computer,
    /// Browser activity.
    Browser,
    /// Audit or trace activity.
    Trace,
    /// External integration activity.
    Integration,
    /// Bundle activity.
    Bundle,
    /// A human clarification request.
    UserInput,
    /// An unrecognized tool with a bounded identity fallback.
    Generic,
}

/// Presentation lifecycle; the caller retains the authoritative execution state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolActivityPhase {
    /// A call was requested; execution input is withheld.
    Requested,
    /// A call is waiting for an authorization decision.
    WaitingApproval,
    /// The interface released a validated start event.
    Started,
    /// The caller reports known successful completion.
    Completed,
    /// The caller reports known failure.
    Failed,
    /// The call was settled without starting execution.
    Cancelled,
    /// The execution outcome is not trustworthy.
    OutcomeUnknown,
    /// Retained history lacks live outcome metadata.
    Recorded,
}

impl ToolActivityPhase {
    /// Human lifecycle label without any effect arguments.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::WaitingApproval => "waiting for approval",
            Self::Started => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::OutcomeUnknown => "outcome unknown",
            Self::Recorded => "recorded",
        }
    }
}

/// A bounded, interface-neutral projection of already released tool activity.
/// This is display data, not a persisted event or evidence of execution authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolActivityPresentation {
    /// Registered tool display category.
    pub kind: ToolActivityKind,
    /// Caller-supplied lifecycle, distinct from authority.
    pub phase: ToolActivityPhase,
    /// Bounded canonical machine identity.
    pub tool_name: String,
    /// Human operation label.
    pub title: String,
    /// Bounded released target or MCP identity.
    pub context: Option<String>,
    /// At most four bounded recognized metadata facts.
    pub facts: Vec<(String, String)>,
    /// The released preview or detail projection is incomplete.
    pub truncated: bool,
}

impl ToolActivityPresentation {
    /// One canonical row shared by terminal and public application interfaces.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.heading()];
        let facts = self
            .facts
            .iter()
            .filter(|(key, _)| key == "Reason")
            .chain(self.facts.iter().filter(|(key, _)| key != "Reason"))
            .take(4);
        parts.extend(facts.map(|(key, value)| format!("{}: {}", clean(key, 32), clean(value, 80))));
        let phase = self.phase.label();
        let body = clean(
            &parts.join(" · "),
            TOOL_ACTIVITY_SUMMARY_CHARS.saturating_sub(phase.chars().count() + 3),
        );
        format!("{body} · {phase}")
    }

    /// Human operation and its bounded released target, without lifecycle duplication.
    pub fn heading(&self) -> String {
        let title = clean(&self.title, 80);
        self.context.as_deref().map_or(title.clone(), |context| {
            format!("{title} · {}", clean(context, 120))
        })
    }

    /// Add trusted caller metadata without retaining arbitrary arguments or output.
    pub fn add_fact(&mut self, label: &str, value: impl AsRef<str>) {
        if self.facts.len() < 4 {
            self.facts
                .push((clean(label, 32), clean(value.as_ref(), 80)));
        }
    }
}

/// Project only released inputs/previews supplied by the interface's release boundary.
/// Requested/cancelled calls ignore input and output. Shell input is always withheld.
/// Oversize or incomplete JSON never yields invented full-output counts.
pub fn tool_activity_presentation(
    name: &str,
    phase: ToolActivityPhase,
    released_input: Option<&Value>,
    released_output: Option<&str>,
) -> ToolActivityPresentation {
    let (kind, title) = classify(name);
    let visible = !matches!(
        phase,
        ToolActivityPhase::Requested
            | ToolActivityPhase::WaitingApproval
            | ToolActivityPhase::Cancelled
    );
    let input = visible
        .then_some(released_input)
        .flatten()
        .filter(|_| kind != ToolActivityKind::Process);
    let output = visible.then_some(released_output).flatten();
    // File reads return raw UTF-8, so a JSON file cannot impersonate metadata.
    let parsed = output
        .filter(|_| kind != ToolActivityKind::FileRead || phase == ToolActivityPhase::Failed)
        .and_then(parse_output);
    let metadata = parsed.as_ref();
    let field = |key: &str| {
        input
            .and_then(|value| value.get(key))
            .or_else(|| metadata.and_then(|value| value.get(key)))
    };
    let mut presentation = ToolActivityPresentation {
        kind,
        phase,
        tool_name: clean(name, 128),
        title: title.into(),
        context: None,
        facts: Vec::new(),
        truncated: output.is_some_and(|value| {
            value.len() > TOOL_ACTIVITY_DETAIL_BYTES || value.ends_with("… preview truncated")
        }),
    };
    presentation.context = match kind {
        ToolActivityKind::FileRead
        | ToolActivityKind::FileList
        | ToolActivityKind::FileSearch
        | ToolActivityKind::FileWrite
        | ToolActivityKind::Patch
        | ToolActivityKind::RepositoryMap
        | ToolActivityKind::RepositorySearch
        | ToolActivityKind::RepositorySummary => field("path")
            .or_else(|| field("root"))
            .and_then(Value::as_str)
            .map(|value| clean(value, 120)),
        ToolActivityKind::Mcp => {
            let server = field("server")
                .and_then(Value::as_str)
                .map(|value| clean(value, 64));
            let tool = field("tool")
                .and_then(Value::as_str)
                .map(|value| clean(value, 64));
            match (server, tool) {
                (Some(server), Some(tool)) => Some(format!("{server} · {tool}")),
                (server, tool) => server.or(tool),
            }
        }
        ToolActivityKind::Web => field("url").and_then(Value::as_str).map(safe_url),
        ToolActivityKind::Generic => Some(clean(name, 80)),
        _ => None,
    }
    .filter(|value| !value.is_empty());
    if matches!(kind, ToolActivityKind::FileRead) {
        let start = field("line_start").and_then(Value::as_u64);
        let end = field("line_end").and_then(Value::as_u64);
        if let Some(start) = start {
            presentation.add_fact(
                "Lines",
                end.map_or_else(|| start.to_string(), |end| format!("{start}–{end}")),
            );
        }
    }
    if let Some(output) = metadata {
        match kind {
            ToolActivityKind::RepositoryMap => {
                add_number(&mut presentation, output, "file_count", "Files")
            }
            ToolActivityKind::FileSearch | ToolActivityKind::RepositorySearch => {
                if let Some(matches) = output.get("matches").and_then(Value::as_array) {
                    presentation.add_fact("Matches", matches.len().to_string());
                } else {
                    add_number(&mut presentation, output, "match_count", "Matches");
                }
            }
            ToolActivityKind::FileList => {
                if let Some(entries) = output.get("entries").and_then(Value::as_array) {
                    presentation.add_fact("Entries", entries.len().to_string());
                }
            }
            ToolActivityKind::FileWrite => add_number(&mut presentation, output, "bytes", "Bytes"),
            _ => {}
        }
        if output.get("truncated").and_then(Value::as_bool) == Some(true) {
            presentation.truncated = true;
            presentation.add_fact("Results", "limit reached");
        }
        if name == "agent.result" {
            match output.get("status").and_then(Value::as_str) {
                Some("queued") => {
                    presentation.phase = ToolActivityPhase::Requested;
                    presentation.add_fact("Agent", "queued");
                }
                Some("running") => presentation.phase = ToolActivityPhase::Started,
                Some("failed" | "interrupted") => presentation.phase = ToolActivityPhase::Failed,
                Some("cancelled") => presentation.phase = ToolActivityPhase::Cancelled,
                _ => {}
            }
        }
    }
    if presentation.phase == ToolActivityPhase::Failed {
        let reason = match kind {
            ToolActivityKind::Process | ToolActivityKind::Computer => {
                let category = metadata
                    .and_then(|value| {
                        value
                            .pointer("/error/type")
                            .or_else(|| value.pointer("/error/code"))
                    })
                    .and_then(Value::as_str);
                match category {
                    Some("denied" | "tool.denied") => "Request denied",
                    Some("outcome_unknown" | "tool.outcome_unknown") => "Execution outcome unknown",
                    Some("invalid_arguments" | "validation_error") => "Invalid tool arguments",
                    _ => "Tool reported failure",
                }
                .to_owned()
            }
            _ => metadata
                .and_then(|value| value.pointer("/error/message"))
                .and_then(Value::as_str)
                .map(|message| clean(message, 80))
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| "Tool reported failure".into()),
        };
        presentation.add_fact("Reason", reason);
    }
    presentation
}

/// Borrow retained output without cloning it or inventing missing execution metadata.
/// Recognizable legacy command output is conservatively withheld unless its bounded
/// typed projection is safe. An unidentified oversized result never exposes a raw prefix.
pub fn retained_tool_activity_document(
    title: &str,
    name: Option<&str>,
    output: &str,
    detailed: bool,
) -> PresentationDocument {
    let known_file = matches!(name, Some("filesystem.read" | "fs.read"));
    let shell = name == Some("shell.run")
        || (!known_file
            && name.is_none()
            && parse_output(output)
                .as_ref()
                .is_some_and(colossus_contracts::is_command_output));
    let unknown_oversize = name.is_none() && output.len() > TOOL_ACTIVITY_DETAIL_BYTES;
    let effective_name = if shell {
        "shell.run"
    } else {
        name.unwrap_or("unknown")
    };
    let mut activity = tool_activity_presentation(
        effective_name,
        ToolActivityPhase::Recorded,
        None,
        (!unknown_oversize).then_some(output),
    );
    if name.is_none() && !shell {
        activity.title = clean(title, 80);
        activity.context = None;
    }
    if shell {
        activity.add_fact("Command", "details withheld");
    }
    if unknown_oversize {
        activity.add_fact("Details", "not shown without tool name");
    }
    let mut document =
        tool_activity_document(&activity, (!unknown_oversize).then_some(output), detailed);
    if unknown_oversize
        && detailed
        && let Some(PresentationBlock::Card { body, .. }) = document.blocks.first_mut()
    {
        body.push(PresentationBlock::Text("Output details withheld: the tool name is unavailable and this result is too large to inspect safely. The original result is retained.".into()));
    }
    document
}

/// Build a compact row or explicitly requested bounded details from released output.
/// Full original result stays with the caller and is never altered by this projection.
pub fn tool_activity_document(
    activity: &ToolActivityPresentation,
    released_output: Option<&str>,
    detailed: bool,
) -> PresentationDocument {
    if !detailed {
        return PresentationDocument::from_block(PresentationBlock::Text(activity.summary()));
    }
    let mut body = vec![PresentationBlock::KeyValue(vec![(
        "Status".into(),
        activity.phase.label().into(),
    )])];
    if !activity.facts.is_empty() {
        body.push(PresentationBlock::KeyValue(
            activity
                .facts
                .iter()
                .take(4)
                .map(|(key, value)| (clean(key, 32), clean(value, 80)))
                .collect(),
        ));
    }
    let output_released = matches!(
        activity.phase,
        ToolActivityPhase::Completed | ToolActivityPhase::Failed | ToolActivityPhase::Recorded
    );
    if let Some(output) = released_output.filter(|output| output_released && !output.is_empty()) {
        body.extend(details::output_blocks(activity, output));
    }
    PresentationDocument::from_block(PresentationBlock::Card {
        title: format!(
            "{} {}",
            match activity.phase {
                ToolActivityPhase::Failed => "Failed",
                ToolActivityPhase::Cancelled => "Cancelled",
                ToolActivityPhase::Requested
                | ToolActivityPhase::Started
                | ToolActivityPhase::WaitingApproval => "Pending",
                ToolActivityPhase::OutcomeUnknown => "Uncertain",
                ToolActivityPhase::Completed => "Completed",
                ToolActivityPhase::Recorded => "Recorded",
            },
            activity.heading()
        ),
        tone: match activity.phase {
            ToolActivityPhase::Failed | ToolActivityPhase::OutcomeUnknown => {
                PresentationTone::Error
            }
            ToolActivityPhase::Cancelled | ToolActivityPhase::WaitingApproval => {
                PresentationTone::Warning
            }
            ToolActivityPhase::Completed => PresentationTone::Success,
            _ => PresentationTone::Tool,
        },
        body,
    })
}

pub(super) fn clean(value: &str, maximum: usize) -> String {
    let mut result = String::new();
    let mut space = false;
    for character in value.chars().take(maximum.saturating_add(1)) {
        if character.is_control()
            || character.is_whitespace()
            || matches!(character, '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
        {
            space = !result.is_empty();
            continue;
        }
        if space && !result.is_empty() {
            result.push(' ');
        }
        space = false;
        result.push(character);
    }
    if result.chars().count() > maximum {
        let mut bounded = result
            .chars()
            .take(maximum.saturating_sub(1))
            .collect::<String>();
        bounded.push('…');
        bounded
    } else {
        result
    }
}

/// Serialize a bounded display preview without first allocating the complete JSON value.
/// Truncated previews are explicitly textual previews, not complete JSON documents.
pub fn tool_activity_json_preview(value: &Value) -> String {
    let (mut preview, truncated) = details::json_prefix(
        value,
        TOOL_ACTIVITY_DETAIL_BYTES - 64,
        TOOL_ACTIVITY_DETAIL_LINES,
        false,
    );
    if truncated {
        preview.push_str("\n… input preview truncated");
    }
    preview
}

pub(super) fn parse_output(output: &str) -> Option<Value> {
    (output.len() <= TOOL_ACTIVITY_DETAIL_BYTES)
        .then(|| serde_json::from_str(output).ok())
        .flatten()
}

fn add_number(activity: &mut ToolActivityPresentation, value: &Value, key: &str, label: &str) {
    if let Some(number) = value.get(key).and_then(Value::as_u64) {
        activity.add_fact(label, number.to_string());
    }
}

fn safe_url(value: &str) -> String {
    let value = clean(value, 256);
    let value = value.split(['?', '#']).next().unwrap_or("");
    if let Some((scheme, remainder)) = value.split_once("://") {
        let remainder = remainder
            .rsplit_once('@')
            .map_or(remainder, |(_, target)| target);
        clean(&format!("{scheme}://{remainder}"), 120)
    } else {
        clean(value, 120)
    }
}

fn classify(name: &str) -> (ToolActivityKind, &'static str) {
    use ToolActivityKind::*;
    match name {
        "filesystem.read" | "fs.read" => (FileRead, "Read file"),
        "filesystem.list" | "fs.list" => (FileList, "List files"),
        "filesystem.search" | "fs.search" => (FileSearch, "Search files"),
        "filesystem.write" | "fs.write" => (FileWrite, "Write file"),
        "patch.apply" => (Patch, "Apply patch"),
        "repo.map" => (RepositoryMap, "Map repository"),
        "repo.symbol_search" => (RepositorySearch, "Find symbols"),
        "repo.references" => (RepositorySearch, "Find references"),
        "repo.file_summary" => (RepositorySummary, "Summarize file"),
        "mcp.call" => (Mcp, "Call MCP tool"),
        "user.ask" => (UserInput, "Ask user"),
        "shell.run" | "process.start" | "process.spawn" => (Process, "Run command"),
        "process.poll" | "process.read" => (Process, "Read process output"),
        "process.write" => (Process, "Send process input"),
        "process.stop" | "process.terminate" => (Process, "Stop process"),
        "process.list" => (Process, "List processes"),
        "git.status" => (Git, "Inspect Git status"),
        "git.diff" => (Git, "Inspect changes"),
        "git.show" => (Git, "Inspect commit"),
        "git.log" => (Git, "Read commit history"),
        "git.commit" => (Git, "Create commit"),
        "task.create" => (Work, "Create task"),
        "task.list" => (Work, "List tasks"),
        "task.get" => (Work, "Read task"),
        "task.update" => (Work, "Update task"),
        "agent.delegate" => (Work, "Delegate work"),
        "agent.result" => (Work, "Read agent result"),
        "agent.list" => (Work, "List agents"),
        _ => match name.split('.').next().unwrap_or(name) {
            "shell" | "process" => (Process, "Process activity"),
            "git" => (Git, "Git activity"),
            "web" | "network" | "docs" => (Web, "Fetch web content"),
            "task" | "decision" | "plan" | "goal" | "agent" | "memory" => (Work, "Work activity"),
            "context" => (Context, "Inspect context"),
            "plugin" => (Plugin, "Use plugin"),
            "computer" => (Computer, "Computer activity"),
            "browser" => (Browser, "Browser activity"),
            "trace" | "telemetry" | "audit" => (Trace, "Inspect trace"),
            "integration" => (Integration, "Use integration"),
            "bundle" => (Bundle, "Inspect bundle"),
            _ => (Generic, "Run tool"),
        },
    }
}

mod details;
#[cfg(test)]
mod tests;
