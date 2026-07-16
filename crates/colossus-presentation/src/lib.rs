//! Event-sourced presentation preferences and pure semantic terminal rendering.

use colossus_contracts::{
    Actor, ContextStatus, CustomTheme, EventClassification, ExecutionContext, NewEvent,
    ProviderEvent, RunEvent, RunEventEnvelope, RunPhase, ThemeColor, ThemeSpinner, ThemeTextStyle,
    ToolCall, ToolResult, WorkStateSnapshot,
};
pub use colossus_contracts::{
    EventDisplayMode, StreamDisplayMode, TerminalPreferences, ThemeName, TranscriptDensity,
};
use colossus_ports::{EventJournal, PresentationRepository, StoreError};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Persisted compatibility identifier; changing it would orphan existing preferences.
const PREFERENCES_STREAM: &str = "presentation:repl";
const PREFERENCES_UPDATED: &str = "presentation.preferences.updated.v1";

const HISTORY_STREAM: &str = "presentation:history";
const HISTORY_APPENDED: &str = "presentation.history.appended.v1";
const MAX_HISTORY_ENTRIES: usize = 1_000;
const MAX_HISTORY_ENTRY_BYTES: usize = 1024 * 1024;
const COMPACT_PREVIEW_CHARS: usize = 240;
const VERBOSE_PREVIEW_CHARS: usize = 8 * 1024;
const MAX_CUSTOM_THEMES: usize = 64;
const MAX_THEME_FILE_BYTES: u64 = 64 * 1024;

/// Presentation rendering failure.
#[derive(Debug, Error)]
pub enum PresentationError {
    /// Released content could not be rendered safely.
    #[error("presentation rendering failed: {0}")]
    Invalid(String),
}

/// Visual meaning applied by a human terminal renderer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PresentationTone {
    /// Informational or neutral content.
    #[default]
    Neutral,
    /// Successful completed work.
    Success,
    /// Content that needs operator attention.
    Warning,
    /// Failed or denied work.
    Error,
    /// Model reasoning summary content.
    Thinking,
    /// Tool or integration activity.
    Tool,
}

/// One bounded semantic block in a human presentation document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PresentationBlock {
    /// Plain released text.
    Text(String),
    /// Released Markdown rendered by the terminal backend.
    Markdown(String),
    /// One prompt-shaped sample rendered with the active prompt palette.
    Prompt {
        /// Left prompt identity.
        left: String,
        /// Prompt indicator or caret.
        indicator: String,
        /// Example draft text.
        input: String,
        /// Optional right-side status.
        right: Option<String>,
    },
    /// Width-aware rows and intentional columns.
    Table(PresentationTable),
    /// A titled semantic status or result card.
    Card {
        /// Short card title.
        title: String,
        /// Visual meaning of the card.
        tone: PresentationTone,
        /// Nested bounded content.
        body: Vec<Self>,
    },
    /// One labeled detail list.
    KeyValue(Vec<(String, String)>),
    /// A source or process-output block.
    Code {
        /// Optional language or stream label.
        language: Option<String>,
        /// Released bounded content.
        content: String,
    },
    /// A unified diff whose line kinds remain visually distinct.
    Diff(String),
    /// Intentional vertical separation.
    Blank,
}

/// Semantic terminal output independent of ANSI, terminal width, or transport.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PresentationDocument {
    /// Ordered blocks to render.
    pub blocks: Vec<PresentationBlock>,
}

/// One backend-neutral styled span ready for a terminal UI adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyledSpan {
    /// Sanitized released text.
    pub content: String,
    /// Theme-resolved text style without terminal escape sequences.
    pub style: ThemeTextStyle,
}

/// One backend-neutral styled terminal line.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyledLine {
    /// Ordered spans composing this visual line.
    pub spans: Vec<StyledSpan>,
}

impl StyledLine {
    /// Create a line with one sanitized span.
    pub fn from_span(content: impl Into<String>, style: ThemeTextStyle) -> Self {
        Self {
            spans: vec![StyledSpan {
                content: content.into(),
                style,
            }],
        }
    }

    /// Return the visible text without terminal control sequences.
    pub fn plain_text(&self) -> String {
        self.spans
            .iter()
            .map(|span| span.content.as_str())
            .collect()
    }
}

impl PresentationDocument {
    /// Create an empty document.
    pub const fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Create a document containing one block.
    pub fn from_block(block: PresentationBlock) -> Self {
        Self {
            blocks: vec![block],
        }
    }

    /// Append one block.
    pub fn push(&mut self, block: PresentationBlock) {
        self.blocks.push(block);
    }

    /// Return whether the document contains no visible blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// Intentional columns and bounded rows for human terminal output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationTable {
    /// Column headings.
    pub headers: Vec<String>,
    /// Data rows. Missing trailing cells render as empty values.
    pub rows: Vec<Vec<String>>,
    /// Message used instead of an empty border when no rows exist.
    pub empty_message: String,
}

impl PresentationTable {
    /// Create a table with an explicit empty-state message.
    pub fn new(
        headers: impl IntoIterator<Item = impl Into<String>>,
        empty: impl Into<String>,
    ) -> Self {
        Self {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
            empty_message: empty.into(),
        }
    }

    /// Append one row.
    pub fn push_row(&mut self, row: impl IntoIterator<Item = impl Into<String>>) {
        self.rows.push(row.into_iter().map(Into::into).collect());
    }
}

/// Convert already released structured data into a human-first terminal document.
///
/// Arrays of records become tables, individual records become labeled details, and scalar
/// collections retain a stable index. This is intentionally presentation-only: callers keep
/// JSON as the machine-readable transport and opt into this document at the terminal boundary.
pub fn document_from_json(value: &Value, title: Option<&str>) -> PresentationDocument {
    let block = json_block(value);
    let block = match title {
        Some(title) => PresentationBlock::Card {
            title: title.into(),
            tone: PresentationTone::Neutral,
            body: vec![block],
        },
        None => block,
    };
    PresentationDocument::from_block(block)
}

fn json_block(value: &Value) -> PresentationBlock {
    match value {
        Value::Array(values) if values.iter().all(Value::is_object) => {
            PresentationBlock::Table(json_record_table(values))
        }
        Value::Array(values) => {
            let mut table = PresentationTable::new(["#", "Value"], "No items.");
            for (index, value) in values.iter().enumerate() {
                table.push_row([(index + 1).to_string(), human_json_value(value)]);
            }
            PresentationBlock::Table(table)
        }
        Value::Object(object) => {
            if let Some(output) = object.get("output").and_then(Value::as_str)
                && (object.contains_key("run_id")
                    || object.contains_key("model")
                    || object.contains_key("role"))
            {
                let details = ordered_json_keys(object)
                    .into_iter()
                    .filter(|key| *key != "output")
                    .filter_map(|key| {
                        object
                            .get(key)
                            .map(|value| (human_field_name(key), human_json_value(value)))
                    })
                    .collect();
                PresentationBlock::Card {
                    title: "Agent response".into(),
                    tone: PresentationTone::Success,
                    body: vec![
                        PresentationBlock::Markdown(output.into()),
                        PresentationBlock::KeyValue(details),
                    ],
                }
            } else {
                PresentationBlock::KeyValue(
                    ordered_json_keys(object)
                        .into_iter()
                        .filter_map(|key| {
                            object
                                .get(key)
                                .map(|value| (human_field_name(key), human_json_value(value)))
                        })
                        .collect(),
                )
            }
        }
        Value::String(value) => PresentationBlock::Markdown(value.clone()),
        _ => PresentationBlock::Text(human_json_value(value)),
    }
}

fn json_record_table(values: &[Value]) -> PresentationTable {
    let objects = values
        .iter()
        .filter_map(Value::as_object)
        .collect::<Vec<_>>();
    let mut keys = Vec::<&str>::new();
    for preferred in [
        "active",
        "status",
        "name",
        "title",
        "id",
        "text",
        "objective",
        "content",
        "summary",
        "description",
        "path",
        "kind",
        "version",
        "type",
        "role",
        "model",
        "message_count",
        "updated_at",
        "created_at",
    ] {
        if objects.iter().any(|object| object.contains_key(preferred)) {
            keys.push(preferred);
        }
    }
    for object in &objects {
        for key in object.keys() {
            if keys.len() == 5 {
                break;
            }
            if !keys.contains(&key.as_str()) && json_column_worthy(object.get(key)) {
                keys.push(key);
            }
        }
        if keys.len() == 5 {
            break;
        }
    }
    keys.truncate(5);
    if keys.is_empty() {
        keys.push("value");
    }
    let mut table =
        PresentationTable::new(keys.iter().map(|key| human_field_name(key)), "No items.");
    for object in objects {
        table.push_row(
            keys.iter()
                .map(|key| object.get(*key).map_or_else(String::new, human_json_value)),
        );
    }
    table
}

fn ordered_json_keys(object: &serde_json::Map<String, Value>) -> Vec<&str> {
    let mut keys = Vec::with_capacity(object.len());
    for preferred in [
        "status",
        "name",
        "title",
        "id",
        "version",
        "description",
        "message",
        "reason",
    ] {
        if object.contains_key(preferred) {
            keys.push(preferred);
        }
    }
    for key in object.keys().map(String::as_str) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn json_column_worthy(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.is_null() || value.is_boolean() || value.is_number() || value.is_string()
    })
}

fn human_field_name(value: &str) -> String {
    let mut rendered = value.replace(['_', '-'], " ");
    if let Some(first) = rendered.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    rendered
}

fn human_json_value(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) if values.is_empty() => "None".into(),
        Value::Array(values) if values.iter().all(Value::is_string) && values.len() <= 4 => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Array(values) => format!("{} items", values.len()),
        Value::Object(values) if values.is_empty() => "None".into(),
        Value::Object(values) => serde_json::to_string(values)
            .map(|value| bounded_text(&value, COMPACT_PREVIEW_CHARS))
            .unwrap_or_else(|_| format!("{} fields", values.len())),
    }
}

/// Width-aware human terminal renderer for presentation documents.
pub struct TerminalDocumentRenderer {
    preferences: TerminalPreferences,
    width: usize,
    color: bool,
}

/// Width-aware presentation renderer that emits backend-neutral styled lines.
pub struct StyledDocumentRenderer {
    preferences: TerminalPreferences,
    width: usize,
    surface: StyledRenderSurface,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StyledRenderSurface {
    #[default]
    Document,
    Transcript,
}

impl StyledDocumentRenderer {
    /// Build a renderer for one immutable preference snapshot and viewport width.
    pub fn new(preferences: TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            width: width.clamp(20, 240),
            surface: StyledRenderSurface::Document,
        }
    }

    /// Build a renderer for a TUI transcript where semantic hierarchy is expressed with
    /// colored headings and indentation instead of recursively nested borders.
    pub fn for_transcript(preferences: TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            width: width.clamp(20, 240),
            surface: StyledRenderSurface::Transcript,
        }
    }

    /// Render a retained semantic document for a terminal UI backend.
    pub fn render(&self, document: &PresentationDocument) -> Vec<StyledLine> {
        let renderer = TerminalDocumentRenderer::new(self.preferences.clone(), self.width);
        let palette = TerminalPalette::for_preferences(&self.preferences);
        if self.surface == StyledRenderSurface::Transcript {
            return self.render_transcript_document(document, &renderer, palette);
        }
        let mut lines = Vec::new();
        for block in &document.blocks {
            let rendered = renderer.render_block(block, self.width);
            if !lines.is_empty()
                && !rendered.is_empty()
                && lines
                    .last()
                    .is_some_and(|line: &StyledLine| !line.spans.is_empty())
                && !matches!(block, PresentationBlock::Blank)
            {
                lines.push(StyledLine::default());
            }
            let style = palette.style_for_block(block);
            lines.extend(rendered.into_iter().map(|line| {
                if line.is_empty() {
                    StyledLine::default()
                } else {
                    StyledLine::from_span(line, style)
                }
            }));
        }
        while lines.last().is_some_and(|line| line.spans.is_empty()) {
            lines.pop();
        }
        lines
    }

    fn render_transcript_document(
        &self,
        document: &PresentationDocument,
        renderer: &TerminalDocumentRenderer,
        palette: TerminalPalette,
    ) -> Vec<StyledLine> {
        let mut lines = Vec::new();
        for block in &document.blocks {
            let rendered = self.render_transcript_block(renderer, palette, block, self.width, None);
            if !lines.is_empty()
                && !rendered.is_empty()
                && lines
                    .last()
                    .is_some_and(|line: &StyledLine| !line.spans.is_empty())
                && !matches!(block, PresentationBlock::Blank)
            {
                lines.push(StyledLine::default());
            }
            lines.extend(rendered);
        }
        while lines.last().is_some_and(|line| line.spans.is_empty()) {
            lines.pop();
        }
        lines
    }

    fn render_transcript_block(
        &self,
        renderer: &TerminalDocumentRenderer,
        palette: TerminalPalette,
        block: &PresentationBlock,
        width: usize,
        inherited_accent: Option<ThemeTextStyle>,
    ) -> Vec<StyledLine> {
        match block {
            PresentationBlock::Card { title, tone, body } => {
                let accent = if *tone == PresentationTone::Neutral {
                    palette.section_style()
                } else {
                    palette.tone_style(*tone)
                };
                let mut title_style = accent;
                title_style.bold = true;
                title_style.dim = false;
                let marker = match tone {
                    PresentationTone::Neutral => "◆",
                    PresentationTone::Success => "✓",
                    PresentationTone::Warning => "!",
                    PresentationTone::Error => "×",
                    PresentationTone::Thinking => "…",
                    PresentationTone::Tool => "›",
                };
                let title = truncate_width(
                    &sanitize_terminal_text(title),
                    width.saturating_sub(2).max(1),
                );
                let mut lines = vec![StyledLine {
                    spans: vec![
                        StyledSpan {
                            content: format!("{marker} "),
                            style: accent,
                        },
                        StyledSpan {
                            content: title,
                            style: title_style,
                        },
                    ],
                }];
                for child in body {
                    if lines.len() > 1
                        && !matches!(child, PresentationBlock::Blank)
                        && lines.last().is_some_and(|line| !line.spans.is_empty())
                    {
                        lines.push(StyledLine::default());
                    }
                    let rendered = self.render_transcript_block(
                        renderer,
                        palette,
                        child,
                        width.saturating_sub(2).max(20),
                        Some(accent),
                    );
                    lines.extend(rendered.into_iter().map(|mut line| {
                        if !line.spans.is_empty() {
                            line.spans.insert(
                                0,
                                StyledSpan {
                                    content: "  ".into(),
                                    style: accent,
                                },
                            );
                        }
                        line
                    }));
                }
                lines
            }
            PresentationBlock::KeyValue(entries) => {
                self.render_transcript_key_values(entries, width, palette, inherited_accent)
            }
            PresentationBlock::Table(table) => {
                self.render_transcript_collection(table, width, palette, inherited_accent)
            }
            _ => {
                let style = palette.style_for_block(block);
                renderer
                    .render_block(block, width)
                    .into_iter()
                    .map(|line| {
                        if line.is_empty() {
                            StyledLine::default()
                        } else {
                            StyledLine::from_span(line, style)
                        }
                    })
                    .collect()
            }
        }
    }

    fn render_transcript_key_values(
        &self,
        entries: &[(String, String)],
        width: usize,
        palette: TerminalPalette,
        inherited_accent: Option<ThemeTextStyle>,
    ) -> Vec<StyledLine> {
        if entries.is_empty() {
            return vec![StyledLine::from_span("No details.", palette.meta_style())];
        }
        let maximum_label_width = entries
            .iter()
            .map(|(label, _)| display_width(&sanitize_terminal_text(label)))
            .max()
            .unwrap_or(1);
        let label_width = maximum_label_width.min((width / 3).clamp(8, 20));
        let value_width = width.saturating_sub(label_width + 2).max(8);
        let stacked = width < 36 || value_width < 12;
        let mut label_style = inherited_accent.unwrap_or_else(|| palette.meta_style());
        label_style.bold = true;
        label_style.dim = false;
        let value_style = palette.assistant_style();
        let mut lines = Vec::new();
        for (label, value) in entries {
            let label = sanitize_terminal_text(label);
            let value = sanitize_terminal_text(value);
            if stacked {
                lines.push(StyledLine::from_span(label, label_style));
                lines.extend(
                    wrap_text(&value, width.saturating_sub(2).max(8))
                        .into_iter()
                        .map(|value| StyledLine {
                            spans: vec![
                                StyledSpan {
                                    content: "  ".into(),
                                    style: label_style,
                                },
                                StyledSpan {
                                    content: value,
                                    style: value_style,
                                },
                            ],
                        }),
                );
                continue;
            }
            let label = truncate_width(&label, label_width);
            let padding = label_width.saturating_sub(display_width(&label));
            let values = wrap_text(&value, value_width);
            for (index, value) in values.into_iter().enumerate() {
                lines.push(StyledLine {
                    spans: vec![
                        StyledSpan {
                            content: if index == 0 {
                                format!("{label}{}", " ".repeat(padding))
                            } else {
                                " ".repeat(label_width)
                            },
                            style: label_style,
                        },
                        StyledSpan {
                            content: "  ".into(),
                            style: palette.meta_style(),
                        },
                        StyledSpan {
                            content: value,
                            style: value_style,
                        },
                    ],
                });
            }
        }
        lines
    }

    fn render_transcript_collection(
        &self,
        table: &PresentationTable,
        width: usize,
        palette: TerminalPalette,
        inherited_accent: Option<ThemeTextStyle>,
    ) -> Vec<StyledLine> {
        if table.rows.is_empty() {
            return wrap_text(&sanitize_terminal_text(&table.empty_message), width)
                .into_iter()
                .map(|line| StyledLine::from_span(line, palette.meta_style()))
                .collect();
        }
        let column_count = table
            .headers
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        let headers = (0..column_count)
            .map(|index| {
                table.headers.get(index).map_or_else(
                    || format!("Field {}", index + 1),
                    |header| sanitize_terminal_text(header),
                )
            })
            .collect::<Vec<_>>();
        let primary_index = collection_primary_index(&headers);
        let status_index = collection_status_index(&headers);
        let mut primary_style = inherited_accent.unwrap_or_else(|| palette.section_style());
        primary_style.bold = true;
        primary_style.dim = false;
        let meta_style = palette.meta_style();
        let metadata_style = palette.assistant_style();
        let mut lines = Vec::new();
        for (row_index, row) in table.rows.iter().enumerate() {
            let primary = row
                .get(primary_index)
                .map(|value| sanitize_terminal_text(value))
                .filter(|value| !value.trim().is_empty() && value != "—")
                .unwrap_or_else(|| format!("Item {}", row_index + 1));
            let status = status_index.and_then(|index| {
                row.get(index).and_then(|value| {
                    collection_status(&headers[index], &sanitize_terminal_text(value), palette)
                })
            });
            let status_width = status
                .as_ref()
                .map_or(0, |(label, _, _)| display_width(label) + 4);
            let primary = truncate_width(&primary, width.saturating_sub(status_width + 2).max(8));
            let mut spans = vec![
                StyledSpan {
                    content: "• ".into(),
                    style: inherited_accent.unwrap_or_else(|| palette.section_style()),
                },
                StyledSpan {
                    content: primary,
                    style: primary_style,
                },
            ];
            if let Some((label, marker, style)) = status {
                spans.extend([
                    StyledSpan {
                        content: "  ".into(),
                        style: meta_style,
                    },
                    StyledSpan {
                        content: format!("{marker} {label}"),
                        style,
                    },
                ]);
            }
            lines.push(StyledLine { spans });

            let metadata = row
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    *index != primary_index
                        && status_index != Some(*index)
                        && headers.get(*index).is_none_or(|header| header != "#")
                })
                .filter_map(|(index, value)| {
                    let value = sanitize_terminal_text(value);
                    (!value.trim().is_empty() && value != "—").then(|| {
                        format!(
                            "{}: {value}",
                            headers
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| format!("Field {}", index + 1))
                        )
                    })
                })
                .collect::<Vec<_>>()
                .join(" · ");
            if !metadata.is_empty() {
                lines.extend(
                    wrap_text(&metadata, width.saturating_sub(2).max(8))
                        .into_iter()
                        .map(|line| StyledLine {
                            spans: vec![
                                StyledSpan {
                                    content: "  ".into(),
                                    style: meta_style,
                                },
                                StyledSpan {
                                    content: line,
                                    style: metadata_style,
                                },
                            ],
                        }),
                );
            }
        }
        lines
    }
}

fn collection_primary_index(headers: &[String]) -> usize {
    for preferred in [
        "name", "title", "id", "path", "tool", "server", "model", "value",
    ] {
        if let Some(index) = headers
            .iter()
            .position(|header| header.eq_ignore_ascii_case(preferred))
        {
            return index;
        }
    }
    headers
        .iter()
        .position(|header| {
            !matches!(
                header.to_ascii_lowercase().as_str(),
                "status" | "active" | "enabled" | "trusted" | "state" | "#"
            )
        })
        .unwrap_or(0)
}

fn collection_status_index(headers: &[String]) -> Option<usize> {
    ["status", "state", "active", "enabled", "trusted"]
        .into_iter()
        .find_map(|preferred| {
            headers
                .iter()
                .position(|header| header.eq_ignore_ascii_case(preferred))
        })
}

fn collection_status(
    header: &str,
    value: &str,
    palette: TerminalPalette,
) -> Option<(String, &'static str, ThemeTextStyle)> {
    let value = value.trim();
    if value.is_empty() || value == "—" {
        return None;
    }
    let header = header.to_ascii_lowercase();
    let normalized = value.to_ascii_lowercase();
    let label: String = match (header.as_str(), normalized.as_str()) {
        ("active", "yes") => "active".into(),
        ("active", "no") => "inactive".into(),
        ("enabled", "yes") => "enabled".into(),
        ("enabled", "no") => "disabled".into(),
        ("trusted", "yes") => "trusted".into(),
        ("trusted", "no") => "untrusted".into(),
        _ => value.into(),
    };
    let semantic = label.to_ascii_lowercase();
    let (marker, style) = match semantic.as_str() {
        "active" | "ready" | "ok" | "healthy" | "completed" | "connected" | "enabled"
        | "trusted" | "running" => ("✓", palette.tone_style(PresentationTone::Success)),
        "failed" | "error" | "denied" | "blocked" | "cancelled" => ("×", palette.error_style()),
        "waiting" | "queued" | "pending" | "paused" | "draft" | "interrupted" | "unknown"
        | "untrusted" => ("!", palette.warning_style()),
        "inactive" | "disabled" => ("·", palette.meta_style()),
        _ => ("·", palette.meta_style()),
    };
    Some((label, marker, style))
}

impl TerminalDocumentRenderer {
    /// Build a renderer for one immutable presentation preference snapshot.
    pub fn new(preferences: TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            width: width.clamp(40, 240),
            color: false,
        }
    }

    /// Enable ANSI styling after the caller has confirmed an interactive terminal.
    pub const fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Render one document into bounded terminal text.
    pub fn render(&self, document: &PresentationDocument) -> String {
        let mut lines = Vec::new();
        for block in &document.blocks {
            let mut rendered = self.render_block(block, self.width);
            if !lines.is_empty()
                && !rendered.is_empty()
                && lines.last().is_some_and(|line: &String| !line.is_empty())
                && !matches!(block, PresentationBlock::Blank)
            {
                lines.push(String::new());
            }
            lines.append(&mut rendered);
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    fn render_block(&self, block: &PresentationBlock, width: usize) -> Vec<String> {
        match block {
            PresentationBlock::Text(text) => wrap_text(&sanitize_terminal_text(text), width),
            PresentationBlock::Markdown(markdown) => self.render_markdown(markdown, width),
            PresentationBlock::Prompt {
                left,
                indicator,
                input,
                right,
            } => self.render_prompt(left, indicator, input, right.as_deref(), width),
            PresentationBlock::Table(table) => self.render_table(table, width),
            PresentationBlock::Card { title, tone, body } => {
                self.render_card(title, *tone, body, width)
            }
            PresentationBlock::KeyValue(entries) => {
                let mut table = PresentationTable::new(["Field", "Value"], "No details.");
                for (key, value) in entries {
                    table.push_row([key, value]);
                }
                self.render_table(&table, width)
            }
            PresentationBlock::Code { language, content } => {
                self.render_code(language.as_deref(), content, width)
            }
            PresentationBlock::Diff(diff) => self.render_diff(diff, width),
            PresentationBlock::Blank => vec![String::new()],
        }
    }

    fn render_prompt(
        &self,
        left: &str,
        indicator: &str,
        input: &str,
        right: Option<&str>,
        width: usize,
    ) -> Vec<String> {
        let left = sanitize_terminal_text(left);
        let indicator = sanitize_terminal_text(indicator);
        let left = truncate_width(
            &left,
            width.saturating_sub(display_width(&indicator) + 4).max(1),
        );
        let input = sanitize_terminal_text(input);
        let right = right.map(sanitize_terminal_text);
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let prompt_style = palette
            .prompt_left
            .map_or_else(TextStyle::plain, TextStyle::color);
        let indicator_style = palette
            .indicator
            .map_or_else(TextStyle::plain, TextStyle::color);
        let right_style = palette
            .prompt_right
            .map_or_else(TextStyle::plain, TextStyle::color);
        let fixed_width = display_width(&left) + display_width(&indicator) + 2;
        let visible_right = right
            .as_deref()
            .filter(|right| fixed_width + display_width(right) + 10 <= width);
        let right_width = visible_right.map_or(0, |right| display_width(right) + 1);
        let input = truncate_width(
            &input,
            width.saturating_sub(fixed_width + right_width).max(1),
        );
        let left_width = fixed_width + display_width(&input);
        let gap = visible_right.map_or(0, |right| {
            width
                .saturating_sub(left_width + display_width(right))
                .max(1)
        });
        let mut rendered = format!(
            "{} {} {}",
            prompt_style.paint(&left, self.color),
            indicator_style.paint(&indicator, self.color),
            palette.assistant.paint(&input, self.color),
        );
        if let Some(right) = visible_right {
            rendered.push_str(&" ".repeat(gap));
            rendered.push_str(&right_style.paint(right, self.color));
        }
        vec![rendered]
    }

    fn render_card(
        &self,
        title: &str,
        tone: PresentationTone,
        body: &[PresentationBlock],
        width: usize,
    ) -> Vec<String> {
        let inner_width = width.saturating_sub(4).max(20);
        let title = truncate_width(&sanitize_terminal_text(title), inner_width);
        let border_style = self.style_for_tone(tone);
        let top_fill = inner_width
            .saturating_add(1)
            .saturating_sub(UnicodeWidthStr::width(title.as_str()));
        let mut lines = vec![format!(
            "{}",
            border_style.paint(&format!("┌─{title}{}┐", "─".repeat(top_fill)), self.color)
        )];
        let mut body_lines = Vec::new();
        for block in body {
            if !body_lines.is_empty() && !matches!(block, PresentationBlock::Blank) {
                body_lines.push(String::new());
            }
            body_lines.extend(self.render_block(block, inner_width));
        }
        if body_lines.is_empty() {
            body_lines.push(String::new());
        }
        for line in body_lines {
            let raw_width = display_width(&line);
            let padding = inner_width.saturating_sub(raw_width);
            lines.push(format!(
                "{} {}{} {}",
                border_style.paint("│", self.color),
                line,
                " ".repeat(padding),
                border_style.paint("│", self.color)
            ));
        }
        lines.push(border_style.paint(&format!("└{}┘", "─".repeat(inner_width + 2)), self.color));
        lines
    }

    fn render_table(&self, table: &PresentationTable, width: usize) -> Vec<String> {
        if table.rows.is_empty() {
            return wrap_text(&sanitize_terminal_text(&table.empty_message), width);
        }
        let original_columns = table
            .headers
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        let columns = original_columns.min(width.saturating_sub(1) / 4).max(1);
        let available = width.saturating_sub(columns * 3 + 1);
        let minimum = (available / columns).clamp(1, 4);
        let mut widths = (0..columns)
            .map(|index| {
                table
                    .headers
                    .get(index)
                    .into_iter()
                    .chain(table.rows.iter().filter_map(|row| row.get(index)))
                    .flat_map(|cell| {
                        sanitize_terminal_text(cell)
                            .lines()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .map(|line| UnicodeWidthStr::width(line.as_str()))
                    .max()
                    .unwrap_or(minimum)
                    .max(minimum)
            })
            .collect::<Vec<_>>();
        while widths.iter().sum::<usize>() > available {
            let Some((index, _)) = widths.iter().enumerate().max_by_key(|(_, value)| **value)
            else {
                break;
            };
            if widths[index] == minimum {
                break;
            }
            widths[index] -= 1;
        }
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let border = |left: char, middle: char, right: char| {
            let mut value = String::new();
            value.push(left);
            for (index, column_width) in widths.iter().enumerate() {
                value.push_str(&"─".repeat(column_width + 2));
                value.push(if index + 1 == columns { right } else { middle });
            }
            palette.meta.paint(&value, self.color)
        };
        let mut lines = vec![border('┌', '┬', '┐')];
        if !table.headers.is_empty() {
            lines.extend(self.render_table_row(&table.headers, &widths, palette.tool));
            lines.push(border('├', '┼', '┤'));
        }
        for (row_index, row) in table.rows.iter().enumerate() {
            lines.extend(self.render_table_row(row, &widths, TextStyle::plain()));
            if row_index + 1 != table.rows.len() {
                lines.push(border('├', '┼', '┤'));
            }
        }
        lines.push(border('└', '┴', '┘'));
        if original_columns > columns {
            lines.push(palette.meta.paint(
                &format!("… {} columns omitted", original_columns - columns),
                self.color,
            ));
        }
        lines
    }

    fn render_table_row(
        &self,
        cells: &[String],
        widths: &[usize],
        style: TextStyle,
    ) -> Vec<String> {
        let wrapped = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                wrap_text(
                    &sanitize_terminal_text(cells.get(index).map_or("", String::as_str)),
                    *width,
                )
            })
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        let palette = TerminalPalette::for_preferences(&self.preferences);
        (0..height)
            .map(|line_index| {
                let mut line = palette.meta.paint("│", self.color);
                for (index, width) in widths.iter().enumerate() {
                    let cell = wrapped[index].get(line_index).map_or("", String::as_str);
                    let padding = width.saturating_sub(display_width(cell));
                    line.push(' ');
                    line.push_str(&style.paint(cell, self.color));
                    line.push_str(&" ".repeat(padding + 1));
                    line.push_str(&palette.meta.paint("│", self.color));
                }
                line
            })
            .collect()
    }

    fn render_markdown(&self, markdown: &str, width: usize) -> Vec<String> {
        let markdown = sanitize_terminal_text(markdown);
        let source = markdown.lines().collect::<Vec<_>>();
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let mut lines = Vec::new();
        let mut index = 0;
        while index < source.len() {
            let line = source[index];
            if line.trim_start().starts_with("```") {
                let language = line.trim().trim_start_matches("```").trim();
                let mut content = Vec::new();
                index += 1;
                while index < source.len() && !source[index].trim_start().starts_with("```") {
                    content.push(source[index]);
                    index += 1;
                }
                lines.extend(self.render_code(
                    (!language.is_empty()).then_some(language),
                    &content.join("\n"),
                    width,
                ));
            } else if is_markdown_table_header(&source, index) {
                let headers = markdown_cells(source[index]);
                let mut table = PresentationTable::new(headers, "No rows.");
                index += 2;
                while index < source.len()
                    && source[index].contains('|')
                    && !source[index].trim().is_empty()
                {
                    table.push_row(markdown_cells(source[index]));
                    index += 1;
                }
                index = index.saturating_sub(1);
                lines.extend(self.render_table(&table, width));
            } else if let Some((level, heading)) = markdown_heading(line) {
                if !lines.is_empty() && lines.last().is_some_and(|value: &String| !value.is_empty())
                {
                    lines.push(String::new());
                }
                let style = if level == 1 {
                    palette.assistant.bold()
                } else {
                    palette.tool.bold()
                };
                lines.extend(
                    wrap_text(heading, width)
                        .into_iter()
                        .map(|value| style.paint(&render_inline_plain(&value), self.color)),
                );
            } else if let Some(item) = markdown_list_item(line) {
                let prefix = if item.0.is_empty() { "• " } else { item.0 };
                let available = width.saturating_sub(display_width(prefix)).max(8);
                let wrapped = wrap_text(item.1, available);
                for (item_index, value) in wrapped.into_iter().enumerate() {
                    let marker = if item_index == 0 {
                        prefix
                    } else {
                        &" ".repeat(display_width(prefix))
                    };
                    lines.push(format!(
                        "{}{}",
                        palette.tool.paint(marker, self.color),
                        render_inline(&value, palette, self.color)
                    ));
                }
            } else if let Some(quote) = line.trim_start().strip_prefix('>') {
                let wrapped = wrap_text(quote.trim_start(), width.saturating_sub(2));
                lines.extend(wrapped.into_iter().map(|value| {
                    format!(
                        "{} {}",
                        palette.meta.paint("│", self.color),
                        palette.meta.paint(&render_inline_plain(&value), self.color)
                    )
                }));
            } else if line.trim().is_empty() {
                if lines.last().is_some_and(|value: &String| !value.is_empty()) {
                    lines.push(String::new());
                }
            } else {
                lines.extend(
                    wrap_text(line, width)
                        .into_iter()
                        .map(|value| render_inline(&value, palette, self.color)),
                );
            }
            index += 1;
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    fn render_code(&self, language: Option<&str>, content: &str, width: usize) -> Vec<String> {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let inner = width.saturating_sub(4).max(12);
        let label = language.map_or_else(|| "code".into(), sanitize_terminal_text);
        let mut lines = vec![palette.meta.paint(&format!("┌─ {label}"), self.color)];
        let content = sanitize_terminal_text(content);
        let numbered = language.is_some_and(|language| language.contains(" · "));
        let source_lines = content.lines().collect::<Vec<_>>();
        let line_count = source_lines.len().max(1);
        let number_width = line_count.to_string().len();
        if content.is_empty() {
            lines.push(format!("{} ", palette.meta.paint("│", self.color)));
        } else {
            for index in bounded_line_indexes(source_lines.len(), 20, 8) {
                let Some(index) = index else {
                    let omitted = source_lines.len().saturating_sub(28);
                    lines.push(format!(
                        "{} {}",
                        palette.meta.paint("│", self.color),
                        palette
                            .meta
                            .paint(&format!("… {omitted} lines omitted …"), self.color)
                    ));
                    continue;
                };
                let prefix = if numbered {
                    format!("{:>number_width$} │ ", index + 1)
                } else {
                    String::new()
                };
                let value = truncate_width(
                    source_lines[index],
                    inner.saturating_sub(display_width(&prefix)),
                );
                lines.push(format!(
                    "{} {}{}",
                    palette.meta.paint("│", self.color),
                    palette.meta.paint(&prefix, self.color),
                    palette.assistant.paint(&value, self.color)
                ));
            }
        }
        lines.push(
            palette
                .meta
                .paint(&format!("└{}", "─".repeat(inner + 2)), self.color),
        );
        lines
    }

    fn render_diff(&self, diff: &str, width: usize) -> Vec<String> {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let diff = sanitize_terminal_text(diff);
        let source_lines = diff.lines().collect::<Vec<_>>();
        bounded_line_indexes(source_lines.len(), 80, 20)
            .into_iter()
            .map(|index| {
                let Some(index) = index else {
                    return palette.meta.paint(
                        &format!(
                            "… {} diff lines omitted …",
                            source_lines.len().saturating_sub(100)
                        ),
                        self.color,
                    );
                };
                let line = source_lines[index];
                let value = truncate_width(line, width);
                let style = if value.starts_with('+') && !value.starts_with("+++") {
                    palette.success
                } else if value.starts_with('-') && !value.starts_with("---") {
                    palette.error
                } else if value.starts_with("@@") {
                    palette.tool
                } else {
                    palette.meta
                };
                style.paint(&value, self.color)
            })
            .collect()
    }

    fn style_for_tone(&self, tone: PresentationTone) -> TextStyle {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        match tone {
            PresentationTone::Neutral => palette.meta,
            PresentationTone::Success => palette.success,
            PresentationTone::Warning => palette.warning,
            PresentationTone::Error => palette.error,
            PresentationTone::Thinking => palette.thinking,
            PresentationTone::Tool => palette.tool,
        }
    }
}

fn bounded_line_indexes(count: usize, head: usize, tail: usize) -> Vec<Option<usize>> {
    if count <= head.saturating_add(tail) {
        return (0..count).map(Some).collect();
    }
    (0..head)
        .map(Some)
        .chain(std::iter::once(None))
        .chain((count - tail..count).map(Some))
        .collect()
}

fn sanitize_terminal_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            matches!(character, '\n' | '\t')
                || (!character.is_control()
                    && !matches!(character, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}'))
        })
        .collect()
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi_for_width(value).as_str())
}

fn strip_ansi_for_width(value: &str) -> String {
    let mut clean = String::new();
    let mut bytes = value.chars().peekable();
    while let Some(character) = bytes.next() {
        if character == '\u{1b}' && bytes.peek() == Some(&'[') {
            bytes.next();
            for next in bytes.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            clean.push(character);
        }
    }
    clean
}

fn truncate_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    let target = width.saturating_sub(1);
    let mut current = 0;
    let mut rendered = String::new();
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current + character_width > target {
            break;
        }
        rendered.push(character);
        current += character_width;
    }
    rendered.push('…');
    rendered
}

fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rendered = Vec::new();
    for source_line in value.lines() {
        if source_line.is_empty() {
            rendered.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in source_line.split_whitespace() {
            let separator = usize::from(!line.is_empty());
            if display_width(&line) + separator + display_width(word) <= width {
                if separator == 1 {
                    line.push(' ');
                }
                line.push_str(word);
                continue;
            }
            if !line.is_empty() {
                rendered.push(line);
                line = String::new();
            }
            let mut remainder = word;
            while display_width(remainder) > width {
                let (chunk, consumed) = split_width_prefix(remainder, width);
                rendered.push(chunk.into());
                remainder = &remainder[consumed..];
            }
            line.push_str(remainder);
        }
        rendered.push(line);
    }
    if rendered.is_empty() {
        rendered.push(String::new());
    }
    rendered
}

fn split_width_prefix(value: &str, width: usize) -> (&str, usize) {
    let mut current = 0;
    let mut consumed = 0;
    for (index, character) in value.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current + character_width > width && consumed > 0 {
            break;
        }
        current += character_width;
        consumed = index + character.len_utf8();
        if current >= width {
            break;
        }
    }
    if consumed == 0 {
        consumed = value.chars().next().map_or(0, char::len_utf8);
    }
    (&value[..consumed], consumed)
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    (level > 0 && level <= 6 && trimmed.as_bytes().get(level) == Some(&b' '))
        .then(|| (level, trimmed[level + 1..].trim()))
}

fn markdown_list_item(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(value) = trimmed.strip_prefix(marker) {
            return Some(("", value));
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && trimmed.get(digits..digits + 2) == Some(". ") {
        return Some((&trimmed[..digits + 2], &trimmed[digits + 2..]));
    }
    None
}

fn is_markdown_table_header(lines: &[&str], index: usize) -> bool {
    lines.get(index).is_some_and(|line| line.contains('|'))
        && lines.get(index + 1).is_some_and(|line| {
            let cells = markdown_cells(line);
            !cells.is_empty()
                && cells.iter().all(|cell| {
                    let cell = cell.trim().trim_matches(':');
                    cell.len() >= 3 && cell.chars().all(|character| character == '-')
                })
        })
}

fn markdown_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| render_inline_plain(cell.trim()))
        .collect()
}

fn render_inline_plain(value: &str) -> String {
    let mut rendered = value.replace("**", "").replace("__", "").replace('`', "");
    rendered = rendered.replace(['*', '_'], "");
    while let Some(start) = rendered.find('[') {
        let Some(label_end) = rendered[start + 1..]
            .find("](")
            .map(|value| start + 1 + value)
        else {
            break;
        };
        let url_start = label_end + 2;
        let Some(url_end) = rendered[url_start..]
            .find(')')
            .map(|value| url_start + value)
        else {
            break;
        };
        let replacement = format!(
            "{} ({})",
            &rendered[start + 1..label_end],
            &rendered[url_start..url_end]
        );
        rendered.replace_range(start..=url_end, &replacement);
    }
    rendered
}

fn render_inline(value: &str, palette: TerminalPalette, color: bool) -> String {
    if !color {
        return render_inline_plain(value);
    }
    let mut rendered = String::new();
    let mut remaining = value;
    while !remaining.is_empty() {
        if let Some(content) = remaining.strip_prefix("**")
            && let Some(end) = content.find("**")
        {
            rendered.push_str(&palette.assistant.bold().paint(&content[..end], true));
            remaining = &content[end + 2..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix("__")
            && let Some(end) = content.find("__")
        {
            rendered.push_str(&palette.assistant.bold().paint(&content[..end], true));
            remaining = &content[end + 2..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix('`')
            && let Some(end) = content.find('`')
        {
            rendered.push_str(&palette.tool.paint(&content[..end], true));
            remaining = &content[end + 1..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix('*')
            && let Some(end) = content.find('*')
        {
            rendered.push_str(&palette.assistant.italic().paint(&content[..end], true));
            remaining = &content[end + 1..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix('_')
            && let Some(end) = content.find('_')
        {
            rendered.push_str(&palette.assistant.italic().paint(&content[..end], true));
            remaining = &content[end + 1..];
            continue;
        }
        if let Some(label) = remaining.strip_prefix('[')
            && let Some(label_end) = label.find("](")
        {
            let url = &label[label_end + 2..];
            if let Some(url_end) = url.find(')') {
                rendered.push_str(&palette.assistant.paint(&label[..label_end], true));
                rendered.push_str(&palette.meta.paint(&format!(" ({})", &url[..url_end]), true));
                remaining = &url[url_end + 1..];
                continue;
            }
        }
        let next = remaining
            .char_indices()
            .skip(1)
            .find(|(_, character)| matches!(character, '*' | '`' | '[' | '_'))
            .map_or(remaining.len(), |(index, _)| index);
        rendered.push_str(&palette.assistant.paint(&remaining[..next], true));
        remaining = &remaining[next..];
    }
    rendered
}

/// Terminal RGB value shared by interactive prompts and semantic transcript palettes.
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

impl From<ThemeColor> for RgbColor {
    fn from(color: ThemeColor) -> Self {
        Self::new(color.red, color.green, color.blue)
    }
}

impl From<RgbColor> for ThemeColor {
    fn from(color: RgbColor) -> Self {
        Self {
            red: color.red,
            green: color.green,
            blue: color.blue,
        }
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

impl From<ThemeTextStyle> for TextStyle {
    fn from(style: ThemeTextStyle) -> Self {
        Self {
            foreground: style.foreground.map(Into::into),
            bold: style.bold,
            dim: style.dim,
            italic: style.italic,
        }
    }
}

impl From<TextStyle> for ThemeTextStyle {
    fn from(style: TextStyle) -> Self {
        Self {
            foreground: style.foreground.map(Into::into),
            bold: style.bold,
            dim: style.dim,
            italic: style.italic,
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
                meta: TextStyle::color(RgbColor::new(174, 184, 194)),
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
                meta: TextStyle::color(RgbColor::new(220, 174, 136)),
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
                meta: TextStyle::color(RgbColor::new(112, 222, 146)),
                spinner_frames: &["░", "▒", "▓", "█", "▓", "▒"],
            },
        }
    }

    /// Resolve a built-in palette or an immutable custom snapshot.
    pub fn for_preferences(preferences: &TerminalPreferences) -> Self {
        preferences
            .custom_theme
            .as_ref()
            .map_or_else(|| Self::for_theme(preferences.theme), Self::for_custom)
    }

    fn for_custom(theme: &CustomTheme) -> Self {
        Self {
            prompt_left: theme.prompt_left.map(Into::into),
            prompt_right: theme.prompt_right.map(Into::into),
            indicator: theme.indicator.map(Into::into),
            continuation: theme.continuation_color.map(Into::into),
            assistant: theme.assistant.into(),
            activity: theme.activity.into(),
            thinking: theme.thinking.into(),
            tool: theme.tool.into(),
            success: theme.success.into(),
            warning: theme.warning.into(),
            error: theme.error.into(),
            meta: theme.meta.into(),
            spinner_frames: spinner_frames(theme.spinner),
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

    /// Metadata style used for low-emphasis terminal affordances such as type-ahead.
    pub fn meta_style(self) -> ThemeTextStyle {
        self.meta.into()
    }

    /// Assistant transcript style.
    pub fn assistant_style(self) -> ThemeTextStyle {
        self.assistant.into()
    }

    /// Operator/user transcript style derived from the prompt identity color.
    pub fn user_style(self) -> ThemeTextStyle {
        let mut style: ThemeTextStyle = self.meta.into();
        style.foreground = self.prompt_left.map(Into::into);
        style.bold = true;
        style.dim = false;
        style
    }

    /// Neutral semantic-section accent used to break up transcript hierarchy.
    pub fn section_style(self) -> ThemeTextStyle {
        let mut style = self.user_style();
        style.italic = false;
        style
    }

    /// Active background-operation style.
    pub fn activity_style(self) -> ThemeTextStyle {
        self.activity.into()
    }

    /// Tool and integration transcript style.
    pub fn tool_style(self) -> ThemeTextStyle {
        self.tool.into()
    }

    /// Warning and attention style.
    pub fn warning_style(self) -> ThemeTextStyle {
        self.warning.into()
    }

    /// Error and denial style.
    pub fn error_style(self) -> ThemeTextStyle {
        self.error.into()
    }

    /// Theme-resolved style for one semantic presentation tone.
    pub fn tone_style(self, tone: PresentationTone) -> ThemeTextStyle {
        match tone {
            PresentationTone::Neutral => self.assistant,
            PresentationTone::Success => self.success,
            PresentationTone::Warning => self.warning,
            PresentationTone::Error => self.error,
            PresentationTone::Thinking => self.thinking,
            PresentationTone::Tool => self.tool,
        }
        .into()
    }

    fn style_for_block(self, block: &PresentationBlock) -> ThemeTextStyle {
        match block {
            PresentationBlock::Card { tone, .. } => return self.tone_style(*tone),
            PresentationBlock::Table(_)
            | PresentationBlock::KeyValue(_)
            | PresentationBlock::Prompt { .. } => self.meta,
            PresentationBlock::Code { .. } | PresentationBlock::Diff(_) => self.tool,
            PresentationBlock::Text(_) | PresentationBlock::Markdown(_) => self.assistant,
            PresentationBlock::Blank => TextStyle::plain(),
        }
        .into()
    }

    /// Render the theme's bounded spinner frame for one elapsed duration.
    pub fn activity_frame(self, elapsed_seconds: f64, color: bool) -> String {
        let index = ((elapsed_seconds.max(0.0) * 10.0) as usize) % self.spinner_frames.len();
        self.activity.paint(self.spinner_frames[index], color)
    }
}

const fn spinner_frames(spinner: ThemeSpinner) -> &'static [&'static str] {
    match spinner {
        ThemeSpinner::Dots => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
        ThemeSpinner::Line => &["-", "\\", "|", "/"],
        ThemeSpinner::Arc => &["◜", "◠", "◝", "◞", "◡", "◟"],
        ThemeSpinner::BouncingBar => &[
            "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█", "▉", "▊", "▋", "▌", "▍", "▎",
        ],
        ThemeSpinner::Aesthetic => &["░", "▒", "▓", "█", "▓", "▒"],
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeFile {
    schema_version: u16,
    name: String,
    #[serde(default)]
    base: ThemeName,
    title: Option<String>,
    caret: Option<String>,
    continuation: Option<String>,
    #[serde(default)]
    prompt: ThemePromptFile,
    #[serde(default)]
    styles: ThemeStylesFile,
    spinner: Option<ThemeSpinner>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemePromptFile {
    left: Option<String>,
    right: Option<String>,
    indicator: Option<String>,
    continuation: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeStylesFile {
    assistant: Option<ThemeStyleFile>,
    activity: Option<ThemeStyleFile>,
    thinking: Option<ThemeStyleFile>,
    tool: Option<ThemeStyleFile>,
    success: Option<ThemeStyleFile>,
    warning: Option<ThemeStyleFile>,
    error: Option<ThemeStyleFile>,
    meta: Option<ThemeStyleFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeStyleFile {
    foreground: Option<String>,
    bold: Option<bool>,
    dim: Option<bool>,
    italic: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyThemeFile {
    name: Option<String>,
    title: Option<String>,
    caret: Option<String>,
    continuation: Option<String>,
    #[serde(default)]
    styles: BTreeMap<String, String>,
    #[serde(default, alias = "trace_styles")]
    trace: BTreeMap<String, String>,
    #[serde(default, alias = "transcript_styles")]
    transcript: BTreeMap<String, String>,
}

const LEGACY_STYLE_KEYS: &[&str] = &[
    "prompt.band",
    "prompt.title",
    "prompt.badge",
    "prompt.model",
    "prompt.caret",
    "prompt.rprompt",
    "prompt.continuation",
    "bottom-toolbar",
    "bottom-toolbar.key",
    "bottom-toolbar.warn",
];
const LEGACY_TRACE_KEYS: &[&str] = &[
    "thinking",
    "done",
    "tool_call",
    "tool_result",
    "approval_requested",
    "approval_auto_granted",
    "risk_assessment",
    "research",
    "context",
];
const LEGACY_TRANSCRIPT_KEYS: &[&str] = &[
    "user",
    "assistant",
    "reasoning",
    "tool",
    "tool_output",
    "approval",
    "risk",
    "research",
    "context",
    "error",
    "meta",
    "border",
    "activity_spinner",
];

/// Bounded metadata for one loaded custom-theme library.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThemeLibraryStatus {
    /// Deterministic built-in and custom names.
    pub names: Vec<String>,
    /// Configuration directories inspected in precedence order.
    pub directories: Vec<PathBuf>,
}

/// Validated custom-theme template that can be saved by an operator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThemeScaffold {
    /// Canonical custom theme identity.
    pub name: String,
    /// Suggested config-adjacent destination.
    pub suggested_path: Option<PathBuf>,
    /// Template serialization format.
    pub format: String,
    /// Strict schema-v1 template content.
    pub content: String,
}

/// Strict data-only custom-theme library loaded during trusted configuration bootstrap.
#[derive(Clone, Debug, Default)]
pub struct ThemeLibrary {
    custom: BTreeMap<String, CustomTheme>,
    directories: Vec<PathBuf>,
}

impl ThemeLibrary {
    /// Load JSON and TOML themes from bounded, non-symlink configuration directories.
    pub fn load(directories: &[PathBuf]) -> Result<Self, PresentationError> {
        let mut library = Self {
            custom: BTreeMap::new(),
            directories: directories.to_vec(),
        };
        for directory in directories {
            let metadata = match fs::symlink_metadata(directory) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(PresentationError::Invalid(error.to_string())),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PresentationError::Invalid(format!(
                    "theme library is not a real directory: {}",
                    directory.display()
                )));
            }
            let mut paths = fs::read_dir(directory)
                .map_err(|error| PresentationError::Invalid(error.to_string()))?
                .map(|entry| {
                    entry
                        .map(|entry| entry.path())
                        .map_err(|error| PresentationError::Invalid(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            paths.sort();
            for path in paths {
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| PresentationError::Invalid(error.to_string()))?;
                if metadata.file_type().is_symlink() {
                    return Err(PresentationError::Invalid(format!(
                        "theme files cannot be symlinks: {}",
                        path.display()
                    )));
                }
                let extension = path.extension().and_then(|value| value.to_str());
                if !matches!(extension, Some("json" | "toml")) {
                    continue;
                }
                if !metadata.is_file() || metadata.len() > MAX_THEME_FILE_BYTES {
                    return Err(PresentationError::Invalid(format!(
                        "theme file must be regular and at most {MAX_THEME_FILE_BYTES} bytes: {}",
                        path.display()
                    )));
                }
                if library.custom.len() == MAX_CUSTOM_THEMES {
                    return Err(PresentationError::Invalid(format!(
                        "custom theme count exceeds {MAX_CUSTOM_THEMES}"
                    )));
                }
                let bytes = read_bounded_theme_file(&path)?;
                let theme = parse_theme_file(&path, &bytes)?;
                if ThemeName::parse(&theme.name).is_some()
                    || library.custom.insert(theme.name.clone(), theme).is_some()
                {
                    return Err(PresentationError::Invalid(format!(
                        "theme identity is built-in or duplicated: {}",
                        path.display()
                    )));
                }
            }
        }
        Ok(library)
    }

    /// Load config-adjacent themes plus the platform user theme library.
    pub fn load_for_config(config_path: &Path) -> Result<Self, PresentationError> {
        let mut directories = Vec::new();
        if let Some(parent) = config_path.parent() {
            directories.push(parent.join("themes"));
        }
        let user_directory = if let Some(value) = std::env::var_os("COLOSSUS_THEME_DIR") {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(PresentationError::Invalid(
                    "COLOSSUS_THEME_DIR must be absolute".into(),
                ));
            }
            Some(path)
        } else {
            default_user_theme_directory()
        };
        if let Some(directory) = user_directory
            && !directories.contains(&directory)
        {
            directories.push(directory);
        }
        Self::load(&directories)
    }

    /// Deterministic library status without file contents.
    pub fn status(&self) -> ThemeLibraryStatus {
        ThemeLibraryStatus {
            names: self.names(),
            directories: self.directories.clone(),
        }
    }

    /// Build the human-first theme library view used by interactive terminal surfaces.
    pub fn status_document(&self, selected: &str) -> PresentationDocument {
        let mut themes = PresentationTable::new(
            ["#", "Active", "Theme", "Type", "Character"],
            "No terminal themes are available.",
        );
        for (index, name) in self.names().into_iter().enumerate() {
            themes.push_row([
                (index + 1).to_string(),
                if name == selected { "yes" } else { "" }.into(),
                name.clone(),
                if ThemeName::parse(&name).is_some() {
                    "Built-in"
                } else {
                    "Custom"
                }
                .into(),
                theme_character(&name).into(),
            ]);
        }

        let mut directories = PresentationTable::new(
            ["#", "Custom theme folder"],
            "No custom theme folders are configured.",
        );
        for (index, directory) in self.directories.iter().enumerate() {
            directories.push_row([(index + 1).to_string(), directory.display().to_string()]);
        }

        PresentationDocument {
            blocks: vec![
                PresentationBlock::Markdown(format!(
                    "# Themes\n\nActive theme: **{selected}**\n\nChoose a number or name to apply a theme. Use `p NUMBER` or `/theme preview NAME` to inspect one without changing your selection."
                )),
                PresentationBlock::Table(themes),
                PresentationBlock::Card {
                    title: "Custom theme search locations".into(),
                    tone: PresentationTone::Neutral,
                    body: vec![PresentationBlock::Table(directories)],
                },
            ],
        }
    }

    /// Build a complete visual sample for one theme without selecting it.
    pub fn preview_document(&self, name: &str) -> Result<PresentationDocument, PresentationError> {
        let theme = self.preview(name)?;
        let built_in = ThemeName::parse(&theme.name).is_some();
        let prompt_title = if built_in {
            "Colossus"
        } else {
            theme.title.as_str()
        };
        let mut metadata = PresentationTable::new(["Field", "Value"], "No theme metadata.");
        for (field, value) in [
            ("Name", theme.name.clone()),
            ("Type", if built_in { "Built-in" } else { "Custom" }.into()),
            ("Base", theme.base.as_str().into()),
            ("Spinner", format!("{:?}", theme.spinner)),
        ] {
            metadata.push_row([field, value.as_str()]);
        }
        if !built_in {
            metadata.push_row(["Source hash", theme.source_hash.as_str()]);
        }

        Ok(PresentationDocument {
            blocks: vec![
                PresentationBlock::Markdown(format!(
                    "# {} theme preview\n\nThis is a preview only; your active theme has not changed.",
                    human_field_name(&theme.name)
                )),
                PresentationBlock::Prompt {
                    left: format!("{prompt_title} 019f-theme"),
                    indicator: if built_in { "›".into() } else { theme.caret.clone() },
                    input: "Ask Colossus to review this change".into(),
                    right: Some("primary:openrouter status=ready".into()),
                },
                PresentationBlock::Table(metadata),
                PresentationBlock::Card {
                    title: "Assistant".into(),
                    tone: PresentationTone::Neutral,
                    body: vec![PresentationBlock::Markdown(
                        "## Markdown sample\n\nThe theme styles **answers**, `inline code`, links, and lists.\n\n- Clear hierarchy\n- Readable content".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Thinking".into(),
                    tone: PresentationTone::Thinking,
                    body: vec![PresentationBlock::Markdown(
                        "_Reviewing the relevant files and constraints…_".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Completed filesystem.read".into(),
                    tone: PresentationTone::Tool,
                    body: vec![PresentationBlock::KeyValue(vec![
                        ("Status".into(), "ok".into()),
                        ("Duration".into(), "0.42s".into()),
                    ])],
                },
                PresentationBlock::Card {
                    title: "Approval required".into(),
                    tone: PresentationTone::Warning,
                    body: vec![PresentationBlock::Text(
                        "This effect needs your confirmation before it runs.".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Completed".into(),
                    tone: PresentationTone::Success,
                    body: vec![PresentationBlock::Text(
                        "The requested work finished successfully.".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Needs attention".into(),
                    tone: PresentationTone::Error,
                    body: vec![PresentationBlock::Text(
                        "A denied or failed effect is visually distinct.".into(),
                    )],
                },
                PresentationBlock::Diff(
                    "@@ -1,2 +1,2 @@\n-old terminal output\n+human-first terminal output".into(),
                ),
            ],
        })
    }

    /// Resolve temporary preferences for a visual preview without mutating the caller.
    pub fn preview_preferences(
        &self,
        name: &str,
        current: &TerminalPreferences,
    ) -> Result<TerminalPreferences, PresentationError> {
        let mut preview = current.clone();
        self.select(name, &mut preview)?;
        Ok(preview)
    }

    /// Build a human validation summary for the already loaded strict library.
    pub fn validation_document(&self) -> PresentationDocument {
        let custom_count = self.custom.len();
        PresentationDocument::from_block(PresentationBlock::Card {
            title: "Theme library valid".into(),
            tone: PresentationTone::Success,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Built-in themes".into(), "5".into()),
                    ("Custom themes".into(), custom_count.to_string()),
                    ("Maximum custom themes".into(), MAX_CUSTOM_THEMES.to_string()),
                    (
                        "Maximum file size".into(),
                        format!("{} KiB", MAX_THEME_FILE_BYTES / 1024),
                    ),
                ]),
                PresentationBlock::Markdown(
                    "Every discovered JSON/TOML theme passed the strict schema, identity, color, size, collision, and symlink checks.".into(),
                ),
            ],
        })
    }

    /// Build a concise confirmation after a theme preference is durably saved.
    pub fn selection_document(&self, selected: &str) -> PresentationDocument {
        PresentationDocument::from_block(PresentationBlock::Card {
            title: "Theme applied".into(),
            tone: PresentationTone::Success,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Theme".into(), selected.into()),
                    ("Saved".into(), "yes".into()),
                ]),
                PresentationBlock::Markdown(format!(
                    "Use `/theme preview {selected}` for the complete visual sample."
                )),
            ],
        })
    }

    /// Produce a strict TOML starter without writing through the terminal interface.
    pub fn scaffold(&self, name: &str) -> Result<ThemeScaffold, PresentationError> {
        let name = normalize_theme_name(name)?;
        if ThemeName::parse(&name).is_some() || self.custom.contains_key(&name) {
            return Err(PresentationError::Invalid(format!(
                "theme identity already exists: {name}"
            )));
        }
        let content = format!(
            "schemaVersion = 1\nname = \"{name}\"\nbase = \"default\"\ntitle = \"Colossus\"\ncaret = \"›\"\ncontinuation = \"…\"\nspinner = \"dots\"\n\n[prompt]\nleft = \"#5FD7FF\"\nright = \"#7F8790\"\nindicator = \"#5FD7FF\"\ncontinuation = \"#7F8790\"\n\n[styles.tool]\nforeground = \"#58A6FF\"\nbold = true\n\n[styles.meta]\nforeground = \"#7F8790\"\ndim = true\n"
        );
        Ok(ThemeScaffold {
            suggested_path: self
                .directories
                .first()
                .map(|directory| directory.join(format!("{name}.toml"))),
            name,
            format: "toml".into(),
            content,
        })
    }

    /// Render one scaffold with explicit no-write and restart guidance.
    pub fn scaffold_document(scaffold: &ThemeScaffold) -> PresentationDocument {
        let destination = scaffold.suggested_path.as_ref().map_or_else(
            || "a configured theme folder".into(),
            |path| path.display().to_string(),
        );
        PresentationDocument::from_block(PresentationBlock::Card {
            title: format!("Custom theme scaffold: {}", scaffold.name),
            tone: PresentationTone::Neutral,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Suggested path".into(), destination),
                    ("Format".into(), scaffold.format.clone()),
                ]),
                PresentationBlock::Code {
                    language: Some("toml".into()),
                    content: scaffold.content.clone(),
                },
                PresentationBlock::Markdown(
                    "The TUI does **not** write this file. Save it deliberately, restart Colossus, then run `/theme validate` and `/theme NAME`.".into(),
                ),
            ],
        })
    }

    /// Deterministic built-in and custom identities.
    pub fn names(&self) -> Vec<String> {
        ["default", "mono", "high_contrast", "carrot", "hacker"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.custom.keys().cloned())
            .collect()
    }

    /// Resolve one immutable preview snapshot without changing preferences.
    pub fn preview(&self, name: &str) -> Result<CustomTheme, PresentationError> {
        let normalized = normalize_theme_name(name)?;
        if let Some(theme) = ThemeName::parse(&normalized) {
            return Ok(builtin_snapshot(theme));
        }
        self.custom.get(&normalized).cloned().ok_or_else(|| {
            PresentationError::Invalid(format!(
                "unknown theme {name}; available={}",
                self.names().join(",")
            ))
        })
    }

    /// Select one built-in or immutable custom snapshot.
    pub fn select(
        &self,
        name: &str,
        preferences: &mut TerminalPreferences,
    ) -> Result<(), PresentationError> {
        let normalized = normalize_theme_name(name)?;
        if let Some(theme) = ThemeName::parse(&normalized) {
            preferences.select_builtin_theme(theme);
            return Ok(());
        }
        let theme = self.custom.get(&normalized).cloned().ok_or_else(|| {
            PresentationError::Invalid(format!(
                "unknown theme {name}; available={}",
                self.names().join(",")
            ))
        })?;
        preferences.select_custom_theme(theme);
        Ok(())
    }
}

fn theme_character(name: &str) -> &'static str {
    match ThemeName::parse(name) {
        Some(ThemeName::Default) => "Balanced blue",
        Some(ThemeName::Mono) => "Color-free",
        Some(ThemeName::HighContrast) => "Strong contrast",
        Some(ThemeName::Carrot) => "Warm orange",
        Some(ThemeName::Hacker) => "Green terminal",
        None => "Custom palette",
    }
}

/// Platform-standard Colossus user theme directory.
pub fn default_user_theme_directory() -> Option<PathBuf> {
    BaseDirs::new().map(|directories| directories.config_dir().join("colossus").join("themes"))
}

fn read_bounded_theme_file(path: &Path) -> Result<Vec<u8>, PresentationError> {
    let file = fs::File::open(path)
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_THEME_FILE_BYTES {
        return Err(PresentationError::Invalid(format!(
            "theme file must be regular and at most {MAX_THEME_FILE_BYTES} bytes: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_THEME_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    if bytes.len() as u64 > MAX_THEME_FILE_BYTES {
        return Err(PresentationError::Invalid(format!(
            "theme file exceeds {MAX_THEME_FILE_BYTES} bytes: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn parse_theme_file(path: &Path, bytes: &[u8]) -> Result<CustomTheme, PresentationError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    let versioned = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str::<Value>(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?
            .get("schemaVersion")
            .is_some(),
        Some("toml") => toml::from_str::<toml::Value>(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?
            .get("schemaVersion")
            .is_some(),
        _ => {
            return Err(PresentationError::Invalid(
                "unsupported theme extension".into(),
            ));
        }
    };
    if !versioned {
        let file: LegacyThemeFile = match path.extension().and_then(|value| value.to_str()) {
            Some("json") => serde_json::from_str(text).map_err(|error| {
                PresentationError::Invalid(format!("{}: {error}", path.display()))
            })?,
            Some("toml") => toml::from_str(text).map_err(|error| {
                PresentationError::Invalid(format!("{}: {error}", path.display()))
            })?,
            _ => unreachable!("validated extension"),
        };
        return resolve_legacy_theme(path, bytes, file);
    }
    let file: ThemeFile = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?,
        Some("toml") => toml::from_str(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?,
        _ => unreachable!("validated extension"),
    };
    if file.schema_version != 1 {
        return Err(PresentationError::Invalid(format!(
            "theme schemaVersion must be 1: {}",
            path.display()
        )));
    }
    let name = normalize_theme_name(&file.name)?;
    let base = builtin_snapshot(file.base);
    Ok(CustomTheme {
        schema_version: 1,
        name,
        source_hash: hex::encode(Sha256::digest(bytes)),
        base: file.base,
        title: bounded_theme_text(file.title.as_deref().unwrap_or("colossus"), 32, "title")?,
        caret: bounded_theme_text(file.caret.as_deref().unwrap_or(">"), 8, "caret")?,
        continuation: bounded_theme_text(
            file.continuation.as_deref().unwrap_or("|"),
            8,
            "continuation",
        )?,
        prompt_left: resolve_color(file.prompt.left.as_deref(), base.prompt_left)?,
        prompt_right: resolve_color(file.prompt.right.as_deref(), base.prompt_right)?,
        indicator: resolve_color(file.prompt.indicator.as_deref(), base.indicator)?,
        continuation_color: resolve_color(
            file.prompt.continuation.as_deref(),
            base.continuation_color,
        )?,
        assistant: resolve_style(base.assistant, file.styles.assistant.as_ref())?,
        activity: resolve_style(base.activity, file.styles.activity.as_ref())?,
        thinking: resolve_style(base.thinking, file.styles.thinking.as_ref())?,
        tool: resolve_style(base.tool, file.styles.tool.as_ref())?,
        success: resolve_style(base.success, file.styles.success.as_ref())?,
        warning: resolve_style(base.warning, file.styles.warning.as_ref())?,
        error: resolve_style(base.error, file.styles.error.as_ref())?,
        meta: resolve_style(base.meta, file.styles.meta.as_ref())?,
        spinner: file.spinner.unwrap_or(base.spinner),
    })
}

fn resolve_legacy_theme(
    path: &Path,
    bytes: &[u8],
    file: LegacyThemeFile,
) -> Result<CustomTheme, PresentationError> {
    validate_legacy_keys(&file.styles, LEGACY_STYLE_KEYS, "style")?;
    validate_legacy_keys(&file.trace, LEGACY_TRACE_KEYS, "trace")?;
    validate_legacy_keys(&file.transcript, LEGACY_TRANSCRIPT_KEYS, "transcript")?;
    let base = builtin_snapshot(ThemeName::Default);
    let name = normalize_theme_name(
        file.name
            .as_deref()
            .or_else(|| path.file_stem().and_then(|value| value.to_str()))
            .ok_or_else(|| PresentationError::Invalid("theme file has no UTF-8 name".into()))?,
    )?;
    let prompt_color = |key: &str, fallback: Option<ThemeColor>| {
        file.styles
            .get(key)
            .map(|value| rich_foreground(value, fallback))
            .transpose()
            .map(|value| value.flatten().or(fallback))
    };
    Ok(CustomTheme {
        schema_version: 1,
        name,
        source_hash: hex::encode(Sha256::digest(bytes)),
        base: ThemeName::Default,
        title: bounded_theme_text(file.title.as_deref().unwrap_or("colossus"), 32, "title")?,
        caret: bounded_theme_text(file.caret.as_deref().unwrap_or(">"), 8, "caret")?,
        continuation: bounded_theme_text(
            file.continuation.as_deref().unwrap_or("|"),
            8,
            "continuation",
        )?,
        prompt_left: prompt_color("prompt.title", base.prompt_left)?,
        prompt_right: prompt_color("prompt.rprompt", base.prompt_right)?,
        indicator: prompt_color("prompt.caret", base.indicator)?,
        continuation_color: prompt_color("prompt.continuation", base.continuation_color)?,
        assistant: legacy_style(&file.transcript, &["assistant"], base.assistant)?,
        activity: legacy_style(&file.transcript, &["meta"], base.activity)?,
        thinking: legacy_style_maps(
            &[
                (&file.trace, &["thinking"]),
                (&file.transcript, &["reasoning"]),
            ],
            base.thinking,
        )?,
        tool: legacy_style_maps(
            &[(&file.trace, &["tool_call"]), (&file.transcript, &["tool"])],
            base.tool,
        )?,
        success: legacy_style(&file.trace, &["done", "tool_result"], base.success)?,
        warning: legacy_style_maps(
            &[
                (&file.trace, &["approval_requested"]),
                (&file.transcript, &["approval"]),
            ],
            base.warning,
        )?,
        error: legacy_style_maps(
            &[
                (&file.trace, &["risk_assessment"]),
                (&file.transcript, &["error", "risk"]),
            ],
            base.error,
        )?,
        meta: legacy_style(&file.transcript, &["meta", "context"], base.meta)?,
        spinner: file
            .transcript
            .get("activity_spinner")
            .map(|value| parse_spinner(value))
            .transpose()?
            .unwrap_or(base.spinner),
    })
}

fn validate_legacy_keys(
    values: &BTreeMap<String, String>,
    allowed: &[&str],
    family: &str,
) -> Result<(), PresentationError> {
    if let Some(key) = values.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(PresentationError::Invalid(format!(
            "theme has unsupported {family} key: {key}"
        )));
    }
    if values
        .values()
        .any(|value| value.len() > 128 || value.chars().any(char::is_control))
    {
        return Err(PresentationError::Invalid(format!(
            "theme {family} values must be control-free and at most 128 bytes"
        )));
    }
    Ok(())
}

fn legacy_style(
    values: &BTreeMap<String, String>,
    keys: &[&str],
    base: ThemeTextStyle,
) -> Result<ThemeTextStyle, PresentationError> {
    keys.iter()
        .find_map(|key| values.get(*key))
        .map_or(Ok(base), |value| parse_rich_style(value, base))
}

fn legacy_style_maps(
    maps: &[(&BTreeMap<String, String>, &[&str])],
    base: ThemeTextStyle,
) -> Result<ThemeTextStyle, PresentationError> {
    maps.iter()
        .find_map(|(values, keys)| keys.iter().find_map(|key| values.get(*key)))
        .map_or(Ok(base), |value| parse_rich_style(value, base))
}

fn rich_foreground(
    value: &str,
    base: Option<ThemeColor>,
) -> Result<Option<ThemeColor>, PresentationError> {
    parse_rich_style(
        value,
        ThemeTextStyle {
            foreground: base,
            bold: false,
            dim: false,
            italic: false,
        },
    )
    .map(|style| style.foreground)
}

fn parse_rich_style(
    value: &str,
    base: ThemeTextStyle,
) -> Result<ThemeTextStyle, PresentationError> {
    let mut style = ThemeTextStyle {
        foreground: base.foreground,
        bold: false,
        dim: false,
        italic: false,
    };
    let mut skip_background = false;
    for token in value.split_ascii_whitespace() {
        if skip_background {
            skip_background = false;
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "bold" => style.bold = true,
            "dim" => style.dim = true,
            "italic" => style.italic = true,
            "on" => skip_background = true,
            token if token.starts_with("bg:") => {}
            token => style.foreground = Some(parse_named_or_hex_color(token)?),
        }
    }
    Ok(style)
}

fn parse_named_or_hex_color(value: &str) -> Result<ThemeColor, PresentationError> {
    if value.starts_with('#') {
        return parse_color(value);
    }
    let (red, green, blue) = match value {
        "black" => (0, 0, 0),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "yellow" => (255, 255, 0),
        "blue" => (0, 0, 255),
        "magenta" => (255, 0, 255),
        "cyan" => (0, 255, 255),
        "white" => (255, 255, 255),
        "bright_black" => (128, 128, 128),
        "bright_red" => (255, 85, 85),
        "bright_green" => (85, 255, 85),
        "bright_yellow" => (255, 255, 85),
        "bright_blue" => (85, 85, 255),
        "bright_magenta" => (255, 85, 255),
        "bright_cyan" => (85, 255, 255),
        "bright_white" => (255, 255, 255),
        _ => {
            return Err(PresentationError::Invalid(format!(
                "unsupported theme color: {value}"
            )));
        }
    };
    Ok(ThemeColor { red, green, blue })
}

fn parse_spinner(value: &str) -> Result<ThemeSpinner, PresentationError> {
    match value {
        "dots" => Ok(ThemeSpinner::Dots),
        "line" => Ok(ThemeSpinner::Line),
        "arc" => Ok(ThemeSpinner::Arc),
        "bouncingBar" | "bouncing_bar" => Ok(ThemeSpinner::BouncingBar),
        "aesthetic" => Ok(ThemeSpinner::Aesthetic),
        _ => Err(PresentationError::Invalid(format!(
            "unsupported theme spinner: {value}"
        ))),
    }
}

fn normalize_theme_name(value: &str) -> Result<String, PresentationError> {
    let value = value.trim().to_ascii_lowercase().replace('-', "_");
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PresentationError::Invalid(
            "theme name must be a simple 1-64 character identifier".into(),
        ));
    }
    Ok(value)
}

fn bounded_theme_text(
    value: &str,
    max_chars: usize,
    field: &str,
) -> Result<String, PresentationError> {
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return Err(PresentationError::Invalid(format!(
            "theme {field} must be nonempty, control-free, and at most {max_chars} characters"
        )));
    }
    Ok(value.into())
}

fn resolve_color(
    value: Option<&str>,
    base: Option<ThemeColor>,
) -> Result<Option<ThemeColor>, PresentationError> {
    value
        .map(parse_color)
        .transpose()
        .map(|value| value.or(base))
}

fn parse_color(value: &str) -> Result<ThemeColor, PresentationError> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(PresentationError::Invalid(format!(
            "theme colors must use #RRGGBB: {value}"
        )));
    }
    Ok(ThemeColor {
        red: u8::from_str_radix(&value[1..3], 16)
            .map_err(|error| PresentationError::Invalid(error.to_string()))?,
        green: u8::from_str_radix(&value[3..5], 16)
            .map_err(|error| PresentationError::Invalid(error.to_string()))?,
        blue: u8::from_str_radix(&value[5..7], 16)
            .map_err(|error| PresentationError::Invalid(error.to_string()))?,
    })
}

fn resolve_style(
    base: ThemeTextStyle,
    value: Option<&ThemeStyleFile>,
) -> Result<ThemeTextStyle, PresentationError> {
    let Some(value) = value else {
        return Ok(base);
    };
    Ok(ThemeTextStyle {
        foreground: resolve_color(value.foreground.as_deref(), base.foreground)?,
        bold: value.bold.unwrap_or(base.bold),
        dim: value.dim.unwrap_or(base.dim),
        italic: value.italic.unwrap_or(base.italic),
    })
}

fn builtin_snapshot(theme: ThemeName) -> CustomTheme {
    let palette = TerminalPalette::for_theme(theme);
    let identity = theme.as_str();
    CustomTheme {
        schema_version: 1,
        name: identity.into(),
        source_hash: hex::encode(Sha256::digest(format!("builtin:{identity}").as_bytes())),
        base: theme,
        title: "colossus".into(),
        caret: "›".into(),
        continuation: "…".into(),
        prompt_left: palette.prompt_left.map(Into::into),
        prompt_right: palette.prompt_right.map(Into::into),
        indicator: palette.indicator.map(Into::into),
        continuation_color: palette.continuation.map(Into::into),
        assistant: palette.assistant.into(),
        activity: palette.activity.into(),
        thinking: palette.thinking.into(),
        tool: palette.tool.into(),
        success: palette.success.into(),
        warning: palette.warning.into(),
        error: palette.error.into(),
        meta: palette.meta.into(),
        spinner: match theme {
            ThemeName::Default => ThemeSpinner::Dots,
            ThemeName::Mono => ThemeSpinner::Line,
            ThemeName::HighContrast => ThemeSpinner::Arc,
            ThemeName::Carrot => ThemeSpinner::BouncingBar,
            ThemeName::Hacker => ThemeSpinner::Aesthetic,
        },
    }
}

fn validate_preferences(preferences: &TerminalPreferences) -> Result<(), StoreError> {
    if preferences.schema_version != 1 {
        return Err(StoreError::Adapter("schema_version must be 1".into()));
    }
    if let Some(theme) = &preferences.custom_theme
        && (theme.schema_version != 1
            || theme.base != preferences.theme
            || normalize_theme_name(&theme.name).is_err()
            || theme.name.contains('-')
            || ThemeName::parse(&theme.name).is_some()
            || theme.source_hash.len() != 64
            || !theme
                .source_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || bounded_theme_text(&theme.title, 32, "title").is_err()
            || bounded_theme_text(&theme.caret, 8, "caret").is_err()
            || bounded_theme_text(&theme.continuation, 8, "continuation").is_err())
    {
        return Err(StoreError::Adapter(
            "custom theme snapshot is invalid or inconsistent".into(),
        ));
    }
    Ok(())
}

/// Immutable-journal implementation of the presentation preference port.
pub struct EventSourcedPresentationRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedPresentationRepository {
    /// Bind the global terminal presentation profile to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl PresentationRepository for EventSourcedPresentationRepository {
    fn load(&self) -> Result<TerminalPreferences, StoreError> {
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        let Some(event) = events.last() else {
            return Ok(TerminalPreferences::default());
        };
        if event.event_type != PREFERENCES_UPDATED {
            return Err(StoreError::Verification(
                "presentation stream contains an unknown event".into(),
            ));
        }
        let payload = self.journal.decrypt_payload(event)?;
        let preferences: TerminalPreferences = serde_json::from_value(
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
        preferences: TerminalPreferences,
        actor: Actor,
    ) -> Result<TerminalPreferences, StoreError> {
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
    preferences: TerminalPreferences,
    color: bool,
}

impl SemanticRenderer {
    /// Create a renderer for one immutable preference snapshot.
    pub fn new(preferences: TerminalPreferences) -> Self {
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
        TerminalPalette::for_preferences(&self.preferences)
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
        let palette = TerminalPalette::for_preferences(&self.preferences);
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
        self.render_document(work_state_document(state))
    }

    /// Render context budget and compaction state.
    pub fn context_status(&self, status: &ContextStatus) -> String {
        let summary = format!(
            "{} session={} messages={} tokens={}/{} compacted={} snapshot={}",
            self.label("context"),
            status.session_id,
            status.message_count,
            status.token_estimate,
            status.context_window_tokens,
            status.compacted,
            status.active_snapshot_id.as_deref().unwrap_or("none")
        );
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            return summary;
        }
        self.render_document(context_status_document(status))
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
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    Some(self.render_document(PresentationDocument::from_block(
                        PresentationBlock::Card {
                            title: "Thinking".into(),
                            tone: PresentationTone::Thinking,
                            body: vec![PresentationBlock::Markdown(summary.clone())],
                        },
                    )))
                } else {
                    Some(format!("{} {summary}", self.label("thinking")))
                }
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
            RunEvent::ToolCancelled {
                turn,
                call,
                elapsed_seconds,
            } => Ok(Some(
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    self.render_document(PresentationDocument::from_block(
                        PresentationBlock::Card {
                            title: format!("Cancelled {}", call.name),
                            tone: PresentationTone::Warning,
                            body: vec![PresentationBlock::KeyValue(vec![
                                ("Turn".into(), turn.to_string()),
                                ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                                (
                                    "Reason".into(),
                                    "operator cancelled before the effect began".into(),
                                ),
                            ])],
                        },
                    ))
                } else {
                    format!(
                        "{} cancelled {} turn={} elapsed={elapsed_seconds:.2}s",
                        self.label("tool"),
                        call.name,
                        turn
                    )
                },
            )),
            RunEvent::ToolCompleted {
                turn,
                result,
                duration_seconds,
                elapsed_seconds,
            } => {
                self.render_tool_completed(*turn, result, *duration_seconds, *elapsed_seconds, None)
            }
            RunEvent::Error {
                code,
                message,
                recoverable,
                turn,
                elapsed_seconds,
            } => {
                if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                    Ok(Some(self.render_document(
                        PresentationDocument::from_block(PresentationBlock::Card {
                            title: "Run error".into(),
                            tone: PresentationTone::Error,
                            body: vec![
                                PresentationBlock::KeyValue(vec![
                                    ("Code".into(), code.clone()),
                                    (
                                        "Recoverable".into(),
                                        if *recoverable { "yes" } else { "no" }.into(),
                                    ),
                                    (
                                        "Turn".into(),
                                        turn.map_or_else(|| "—".into(), |value| value.to_string()),
                                    ),
                                    ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                                ]),
                                PresentationBlock::Markdown(message.clone()),
                            ],
                        }),
                    )))
                } else {
                    Ok(Some(self.with_detail(
                        format!(
                            "{} code={} recoverable={} turn={} elapsed={:.2}s",
                            self.label("error"),
                            code,
                            if *recoverable { "yes" } else { "no" },
                            turn.map_or_else(|| "none".into(), |value| value.to_string()),
                            elapsed_seconds,
                        ),
                        Some(bounded_text(message, COMPACT_PREVIEW_CHARS)),
                    )))
                }
            }
        }
    }

    /// Build a retained semantic document for one transcript-worthy run event.
    ///
    /// Live deltas, final assistant text, phases, and tool-start activity are handled by
    /// their dedicated TUI rows. Everything returned here can be reflowed after resize.
    pub fn run_event_document(
        &self,
        event: &RunEvent,
        call: Option<&ToolCall>,
    ) -> Option<PresentationDocument> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return None;
        }
        match event {
            RunEvent::Provider {
                event: ProviderEvent::ReasoningSummary { summary },
            } if self.preferences.show_reasoning => {
                Some(PresentationDocument::from_block(PresentationBlock::Card {
                    title: "Thinking".into(),
                    tone: PresentationTone::Thinking,
                    body: vec![PresentationBlock::Markdown(summary.clone())],
                }))
            }
            RunEvent::Provider {
                event: ProviderEvent::Usage { usage },
            } if self.preferences.events_mode == EventDisplayMode::Verbose => Some(
                PresentationDocument::from_block(PresentationBlock::KeyValue(vec![
                    ("Input tokens".into(), usage.input_tokens.to_string()),
                    ("Output tokens".into(), usage.output_tokens.to_string()),
                    ("Total tokens".into(), usage.total_tokens.to_string()),
                ])),
            ),
            RunEvent::ToolCompleted {
                result,
                duration_seconds,
                ..
            } if self.preferences.events_mode != EventDisplayMode::Off || result.exit_code != 0 => {
                Some(tool_result_document(result, *duration_seconds, call))
            }
            RunEvent::ToolCancelled {
                turn,
                call,
                elapsed_seconds,
            } => Some(PresentationDocument::from_block(PresentationBlock::Card {
                title: format!("Cancelled {}", call.name),
                tone: PresentationTone::Warning,
                body: vec![PresentationBlock::KeyValue(vec![
                    ("Turn".into(), turn.to_string()),
                    ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                    (
                        "Reason".into(),
                        "operator cancelled before the effect began".into(),
                    ),
                ])],
            })),
            RunEvent::Error {
                code,
                message,
                recoverable,
                turn,
                elapsed_seconds,
            } => Some(PresentationDocument::from_block(PresentationBlock::Card {
                title: "Run error".into(),
                tone: PresentationTone::Error,
                body: vec![
                    PresentationBlock::KeyValue(vec![
                        ("Code".into(), code.clone()),
                        (
                            "Recoverable".into(),
                            if *recoverable { "yes" } else { "no" }.into(),
                        ),
                        (
                            "Turn".into(),
                            turn.map_or_else(|| "—".into(), |value| value.to_string()),
                        ),
                        ("Elapsed".into(), format!("{elapsed_seconds:.2}s")),
                    ]),
                    PresentationBlock::Markdown(message.clone()),
                ],
            })),
            _ => None,
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
            RunPhase::Cancelling => "cancelling",
            RunPhase::Cancelled => "cancelled",
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
        if call.name == "user.ask" {
            return Ok(Some(match self.preferences.events_mode {
                EventDisplayMode::Verbose => format!(
                    "{} waiting name=user.ask call_id={} turn={turn}",
                    self.label("input"),
                    call.call_id
                ),
                EventDisplayMode::Compact | EventDisplayMode::Off => {
                    format!("{} user.ask waiting for your answer", self.label("input"))
                }
            }));
        }
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
        call: Option<&ToolCall>,
    ) -> Result<Option<String>, PresentationError> {
        let parsed = serde_json::from_str::<Value>(&result.output)
            .unwrap_or_else(|_| Value::String(result.output.clone()));
        let family = ToolFamily::from_name(&result.name);
        let recoverable = parsed
            .pointer("/error/recoverable")
            .and_then(Value::as_bool);
        let lifecycle_status = parsed.get("status").and_then(Value::as_str);
        let pending =
            result.name == "agent.result" && matches!(lifecycle_status, Some("queued" | "running"));
        let failed_child = result.name == "agent.result"
            && matches!(lifecycle_status, Some("failed" | "interrupted"));
        let failed = result.exit_code != 0 || failed_child;
        if self.preferences.events_mode == EventDisplayMode::Off && !failed {
            return Ok(None);
        }
        let status = if pending {
            lifecycle_status.unwrap_or("pending")
        } else if failed {
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
        if self.preferences.transcript_density == TranscriptDensity::Comfortable {
            return Ok(Some(self.render_document(tool_result_document(
                result,
                duration_seconds,
                call,
            ))));
        }
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

    /// Render a tool completion with its matching request context for richer source and process
    /// cards. Callers must supply the already released call paired by its opaque call ID.
    pub fn tool_completed_with_call(
        &self,
        turn: u16,
        result: &ToolResult,
        duration_seconds: f64,
        elapsed_seconds: f64,
        call: Option<&ToolCall>,
    ) -> Result<Option<String>, PresentationError> {
        self.render_tool_completed(turn, result, duration_seconds, elapsed_seconds, call)
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

    fn render_document(&self, document: PresentationDocument) -> String {
        TerminalDocumentRenderer::new(self.preferences.clone(), 100)
            .with_color(self.color)
            .render(&document)
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

/// Build the canonical semantic card for one released tool result.
///
/// Terminal strings and the Ratatui transcript consume this same retained document so
/// source previews, process streams, diffs, tables, and failures reflow on resize.
pub fn tool_result_document(
    result: &ToolResult,
    duration_seconds: f64,
    call: Option<&ToolCall>,
) -> PresentationDocument {
    let parsed = serde_json::from_str::<Value>(&result.output)
        .unwrap_or_else(|_| Value::String(result.output.clone()));
    let lifecycle_status = parsed.get("status").and_then(Value::as_str);
    let pending =
        result.name == "agent.result" && matches!(lifecycle_status, Some("queued" | "running"));
    let failed_child =
        result.name == "agent.result" && matches!(lifecycle_status, Some("failed" | "interrupted"));
    let failed = result.exit_code != 0 || failed_child;
    let recoverable = parsed
        .pointer("/error/recoverable")
        .and_then(Value::as_bool);
    let status = if pending {
        lifecycle_status.unwrap_or("pending")
    } else if failed {
        if recoverable == Some(true) {
            "recoverable_error"
        } else {
            "failed"
        }
    } else {
        "ok"
    };
    let mut body = vec![PresentationBlock::KeyValue(vec![
        ("Status".into(), status.replace('_', " ")),
        ("Duration".into(), format!("{duration_seconds:.2}s")),
        ("Exit".into(), result.exit_code.to_string()),
    ])];
    if failed && let Some(message) = parsed.pointer("/error/message").and_then(Value::as_str) {
        body.push(PresentationBlock::Markdown(message.into()));
    } else {
        body.push(tool_output_block(
            &result.name,
            &parsed,
            call.map(|call| &call.arguments),
        ));
    }
    let context = call
        .and_then(|call| tool_call_context(call, ToolFamily::from_name(&result.name)))
        .map(|value| format!(" · {}", bounded_text(&value, 60)))
        .unwrap_or_default();
    PresentationDocument::from_block(PresentationBlock::Card {
        title: format!(
            "{} {}{}",
            if failed {
                "Failed"
            } else if pending {
                "Pending"
            } else {
                "Completed"
            },
            result.name,
            context,
        ),
        tone: if failed {
            PresentationTone::Error
        } else if pending {
            PresentationTone::Warning
        } else {
            PresentationTone::Success
        },
        body,
    })
}

/// Build the canonical semantic work-state document for terminal and TUI backends.
pub fn work_state_document(state: &WorkStateSnapshot) -> PresentationDocument {
    let mut body = vec![PresentationBlock::KeyValue(vec![
        ("Session".into(), state.session_id.clone()),
        (
            "Tasks".into(),
            format!(
                "{} open / {} total",
                state.open_task_count,
                state.tasks.len()
            ),
        ),
        (
            "Active decisions".into(),
            state.active_decisions.len().to_string(),
        ),
        (
            "Actionable plans".into(),
            state.actionable_plans.len().to_string(),
        ),
        ("Goals".into(), state.current_goals.len().to_string()),
        (
            "Subagents".into(),
            state.current_subagents.len().to_string(),
        ),
    ])];
    let mut work = PresentationTable::new(
        ["Kind", "ID", "Status", "Summary"],
        "No active tasks or goals.",
    );
    for task in state.tasks.iter().filter(|task| {
        !matches!(
            task.status,
            colossus_contracts::TaskStatus::Completed | colossus_contracts::TaskStatus::Cancelled
        )
    }) {
        work.push_row([
            "Task".into(),
            task.id.clone(),
            format!("{:?}", task.status).to_ascii_lowercase(),
            task.title.clone(),
        ]);
    }
    for goal in &state.current_goals {
        work.push_row([
            "Goal".into(),
            goal.id.clone(),
            format!("{:?}", goal.status).to_ascii_lowercase(),
            goal.objective.clone(),
        ]);
    }
    body.push(PresentationBlock::Table(work));
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Current work".into(),
        tone: PresentationTone::Neutral,
        body,
    })
}

/// Build the canonical semantic context-status document for terminal and TUI backends.
pub fn context_status_document(status: &ContextStatus) -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Context".into(),
        tone: if status.compacted {
            PresentationTone::Warning
        } else {
            PresentationTone::Neutral
        },
        body: vec![PresentationBlock::KeyValue(vec![
            ("Session".into(), status.session_id.clone()),
            ("Messages".into(), status.message_count.to_string()),
            (
                "Tokens".into(),
                format!(
                    "{} / {}",
                    status.token_estimate, status.context_window_tokens
                ),
            ),
            (
                "Compacted".into(),
                if status.compacted { "yes" } else { "no" }.into(),
            ),
            (
                "Snapshot".into(),
                status
                    .active_snapshot_id
                    .clone()
                    .unwrap_or_else(|| "—".into()),
            ),
        ])],
    })
}

fn tool_output_block(name: &str, output: &Value, arguments: Option<&Value>) -> PresentationBlock {
    if let Some(diff) = output.get("diff").and_then(Value::as_str) {
        let (additions, deletions) = diff_counts(diff);
        let title = output
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(|| "Changes".into(), |path| format!("Changes · {path}"));
        return PresentationBlock::Card {
            title,
            tone: PresentationTone::Tool,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Added".into(), additions.to_string()),
                    ("Removed".into(), deletions.to_string()),
                ]),
                PresentationBlock::Diff(diff.into()),
            ],
        };
    }
    if (name == "git.diff" || name == "git.show" || name.ends_with(".diff"))
        && let Some(diff) = output
            .as_str()
            .or_else(|| output.get("stdout").and_then(Value::as_str))
            .or_else(|| output.get("diff").and_then(Value::as_str))
            .or_else(|| output.get("output").and_then(Value::as_str))
    {
        return PresentationBlock::Diff(diff.into());
    }
    if matches!(
        ToolFamily::from_name(name),
        ToolFamily::Shell | ToolFamily::Git
    ) {
        let stdout = output.get("stdout").and_then(Value::as_str);
        let stderr = output.get("stderr").and_then(Value::as_str);
        let mut body = Vec::new();
        if let Some(stdout) = stdout.filter(|value| !value.is_empty()) {
            body.push(PresentationBlock::Code {
                language: Some("stdout".into()),
                content: stdout.into(),
            });
        }
        if let Some(stderr) = stderr.filter(|value| !value.is_empty()) {
            body.push(PresentationBlock::Code {
                language: Some("stderr".into()),
                content: stderr.into(),
            });
        }
        if body.len() == 1 {
            return body.remove(0);
        }
        if !body.is_empty() {
            return PresentationBlock::Card {
                title: "Process output".into(),
                tone: PresentationTone::Neutral,
                body,
            };
        }
    }
    if let Some(records) = [
        "entries",
        "matches",
        "results",
        "sources",
        "tasks",
        "decisions",
        "plans",
        "goals",
        "memories",
        "sessions",
        "tools",
        "resources",
    ]
    .iter()
    .find_map(|key| output.get(*key).filter(|value| value.is_array()))
    {
        return json_block(records);
    }
    if let Some(text) = output.as_str() {
        return if matches!(ToolFamily::from_name(name), ToolFamily::Files) {
            PresentationBlock::Code {
                language: Some(source_label(arguments)),
                content: text.into(),
            }
        } else {
            PresentationBlock::Markdown(text.into())
        };
    }
    json_block(output)
}

fn tool_call_context(call: &ToolCall, family: ToolFamily) -> Option<String> {
    if matches!(family, ToolFamily::Shell)
        && let Some(arguments) = call.arguments.get("argv").and_then(Value::as_array)
    {
        let command = arguments
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        if !command.is_empty() {
            return Some(format!("$ {command}"));
        }
    }
    summarize_value(&call.arguments, family.keys())
}

fn diff_counts(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(additions, deletions), line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            (additions + 1, deletions)
        } else if line.starts_with('-') && !line.starts_with("---") {
            (additions, deletions + 1)
        } else {
            (additions, deletions)
        }
    })
}

fn source_label(arguments: Option<&Value>) -> String {
    let Some(path) = arguments
        .and_then(|value| find_key(value, "path", 0))
        .and_then(Value::as_str)
    else {
        return "file".into();
    };
    let language = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| match extension.to_ascii_lowercase().as_str() {
            "rs" => "rust",
            "py" => "python",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" | "tsx" => "typescript",
            "md" => "markdown",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            "toml" => "toml",
            "sh" | "bash" | "zsh" => "shell",
            _ => "file",
        })
        .unwrap_or("file");
    format!("{language} · {path}")
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
        EventDisplayMode, EventSourcedPresentationRepository, MAX_CUSTOM_THEMES,
        MAX_THEME_FILE_BYTES, PresentationBlock, PresentationDocument, PresentationTable,
        PresentationTone, SemanticRenderer, StreamDisplayMode, StyledDocumentRenderer,
        TerminalDocumentRenderer, TerminalPalette, TerminalPreferences, ThemeLibrary, ThemeName,
        TranscriptDensity, display_width, document_from_json,
    };
    use colossus_contracts::{
        Actor, ActorType, ProviderEvent, ProviderUsage, RunEvent, RunEventEnvelope, RunPhase,
        ToolCall, ToolResult, WorkStateSnapshot,
    };
    use colossus_ports::{EventJournal, PresentationRepository, ToolRegistry};
    use colossus_testkit::{InMemoryEventJournal, assert_presentation_repository_conformance};
    use colossus_tools::{StaticToolRegistry, builtin_names};
    use std::{fs, path::PathBuf, sync::Arc};
    use tempfile::tempdir;

    #[test]
    fn terminal_documents_render_markdown_tables_cards_and_diff_within_width() {
        let mut items = PresentationTable::new(["Name", "Status"], "No tools available.");
        items.push_row(["filesystem.read", "ready"]);
        let document = PresentationDocument {
            blocks: vec![
                PresentationBlock::Markdown(
                    "# Result\n\nA **useful** answer.\n\n- first\n- second\n\n```rust\nfn main() {}\n```"
                        .into(),
                ),
                PresentationBlock::Table(items),
                PresentationBlock::Card {
                    title: "Git changes".into(),
                    tone: PresentationTone::Success,
                    body: vec![PresentationBlock::Diff(
                        "@@ -1 +1 @@\n-old\n+new".into(),
                    )],
                },
            ],
        };
        let rendered =
            TerminalDocumentRenderer::new(TerminalPreferences::default(), 64).render(&document);
        assert!(rendered.contains("Result"));
        assert!(rendered.contains("• first"));
        assert!(rendered.contains("fn main() {}"));
        assert!(rendered.contains("filesystem.read"));
        assert!(rendered.contains("Git changes"));
        assert!(rendered.contains("+new"));
        assert!(rendered.lines().all(|line| display_width(line) <= 64));
        for width in [60, 80, 120, 160] {
            let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), width)
                .render(&document);
            assert!(
                rendered.lines().all(|line| display_width(line) <= width),
                "width {width}"
            );
        }
        let colored = TerminalDocumentRenderer::new(TerminalPreferences::default(), 64)
            .with_color(true)
            .render(&PresentationDocument::from_block(
                PresentationBlock::Markdown("A **bold** value and `code`.".into()),
            ));
        assert!(colored.contains("\x1b["));
        assert!(!colored.contains("**"));
        assert!(!colored.contains('`'));
    }

    #[test]
    fn transcript_documents_flatten_card_and_detail_chrome_into_colored_hierarchy() {
        let document = PresentationDocument::from_block(PresentationBlock::Card {
            title: "Colossus terminal".into(),
            tone: PresentationTone::Neutral,
            body: vec![
                PresentationBlock::Text("Type a message to run the agent.".into()),
                PresentationBlock::KeyValue(vec![
                    ("Send".into(), "Enter sends".into()),
                    ("Scroll".into(), "PageUp and PageDown".into()),
                ]),
                PresentationBlock::Card {
                    title: "Nested warning".into(),
                    tone: PresentationTone::Warning,
                    body: vec![PresentationBlock::Text("Still one visual level.".into())],
                },
            ],
        });
        let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 80)
            .render(&document);
        let rendered = lines
            .iter()
            .map(super::StyledLine::plain_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("◆ Colossus terminal"), "{rendered}");
        assert!(rendered.contains("Send"), "{rendered}");
        assert!(rendered.contains("Enter sends"), "{rendered}");
        assert!(rendered.contains("Scroll"), "{rendered}");
        assert!(rendered.contains("! Nested warning"), "{rendered}");
        assert!(!rendered.contains(['┌', '┐', '└', '┘']), "{rendered}");
        let heading = lines.first().expect("semantic heading");
        assert_eq!(heading.spans.len(), 2);
        assert!(heading.spans[1].style.bold);
        let details = lines
            .iter()
            .find(|line| line.plain_text().contains("Enter sends"))
            .expect("detail line");
        assert_ne!(
            heading.spans[0].style,
            details.spans.last().expect("detail value").style
        );
    }

    #[test]
    fn transcript_collections_render_as_readable_borderless_scan_rows() {
        let document = document_from_json(
            &serde_json::json!([
                {
                    "active": true,
                    "name": "coding",
                    "description": "Implement and verify scoped software changes with repository evidence.",
                    "version": "0.1.0",
                    "source": "bundled:coding"
                },
                {
                    "active": false,
                    "name": "offline-dev",
                    "description": "Prefer credential-free and network-free verification paths.",
                    "version": "0.1.0",
                    "source": "bundled:offline-dev"
                }
            ]),
            Some("Skills"),
        );
        let lines = StyledDocumentRenderer::for_transcript(TerminalPreferences::default(), 56)
            .render(&document);
        let rendered = lines
            .iter()
            .map(super::StyledLine::plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("◆ Skills"), "{rendered}");
        assert!(rendered.contains("• coding  ✓ active"), "{rendered}");
        assert!(rendered.contains("• offline-dev  · inactive"), "{rendered}");
        assert!(rendered.contains("Description: Implement"), "{rendered}");
        assert!(
            !rendered.contains(['┌', '┐', '└', '┘', '│', '─']),
            "{rendered}"
        );
        assert!(
            lines
                .iter()
                .all(|line| display_width(&line.plain_text()) <= 56),
            "{rendered}"
        );
        let metadata = lines
            .iter()
            .find(|line| line.plain_text().contains("Description: Implement"))
            .expect("readable metadata line");
        assert!(!metadata.spans.last().expect("metadata span").style.dim);
    }

    #[test]
    fn terminal_documents_sanitize_untrusted_controls_and_bound_unicode_width() {
        let document = PresentationDocument::from_block(PresentationBlock::Card {
            title: "unsafe\u{1b}[31m\u{200b}".into(),
            tone: PresentationTone::Warning,
            body: vec![PresentationBlock::Text(format!(
                "wide 界界界 and a-very-long-unbroken-value-that-must-wrap \u{1b}]8;;https://example.test\u{7}{}\u{1b}]8;;\u{7}",
                "oversized ".repeat(1_000)
            ))],
        });
        let rendered =
            TerminalDocumentRenderer::new(TerminalPreferences::default(), 40).render(&document);
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{7}'));
        assert!(!rendered.contains('\u{200b}'));
        assert!(rendered.lines().all(|line| display_width(line) <= 40));

        let mut wide_table =
            PresentationTable::new((0..20).map(|index| format!("Column {index}")), "No rows.");
        wide_table.push_row((0..20).map(|index| format!("value-{index}")));
        let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 40).render(
            &PresentationDocument::from_block(PresentationBlock::Table(wide_table)),
        );
        assert!(rendered.contains("columns omitted"));
        assert!(rendered.lines().all(|line| display_width(line) <= 40));
    }

    #[test]
    fn structured_json_becomes_intentional_human_tables_and_details() {
        let values = serde_json::json!([
            {"id": "task-1", "title": "Build UX", "status": "running", "internal": {"x": 1}},
            {"id": "task-2", "title": "Test UX", "status": "queued", "internal": {"x": 2}}
        ]);
        let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 90)
            .render(&document_from_json(&values, Some("Tasks")));
        assert!(rendered.contains("Tasks"));
        assert!(rendered.contains("Status"));
        assert!(rendered.contains("Build UX"));
        assert!(!rendered.contains("internal"));

        let details = TerminalDocumentRenderer::new(TerminalPreferences::default(), 80).render(
            &document_from_json(
                &serde_json::json!({"status": "ready", "id": "worker-1", "active": true}),
                None,
            ),
        );
        assert!(details.contains("Status"));
        assert!(details.contains("ready"));
        assert!(details.contains("Active"));
        assert!(details.contains("yes"));

        let run = TerminalDocumentRenderer::new(TerminalPreferences::default(), 80).render(
            &document_from_json(
                &serde_json::json!({
                    "run_id": "run-1",
                    "model": "openrouter/free",
                    "output": "## Connected\n\n- yes"
                }),
                None,
            ),
        );
        assert!(run.contains("Agent response"));
        assert!(run.contains("Connected"));
        assert!(run.contains("• yes"));
        assert!(!run.contains("##"));
    }

    #[test]
    fn comfortable_semantics_render_specialized_tool_and_error_cards() {
        let renderer = SemanticRenderer::new(TerminalPreferences::default());
        let search = renderer
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-search".into(),
                    name: "filesystem.search".into(),
                    output: serde_json::json!({
                        "matches": [
                            {"path": "src/main.rs", "line": 42, "text": "fn main()"}
                        ]
                    })
                    .to_string(),
                    exit_code: 0,
                },
                duration_seconds: 0.2,
                elapsed_seconds: 0.4,
            })
            .expect("render search")
            .expect("visible search");
        assert!(search.contains("Completed filesystem.search"));
        assert!(search.contains("src/main.rs"));
        assert!(search.contains("fn main()"));
        assert!(!search.contains("\"matches\""));

        let pending_subagent = renderer
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-agent-result".into(),
                    name: "agent.result".into(),
                    output: serde_json::json!({
                        "id": "agent-1",
                        "status": "queued",
                        "error": ""
                    })
                    .to_string(),
                    exit_code: 0,
                },
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            })
            .expect("render pending subagent")
            .expect("visible pending subagent");
        assert!(pending_subagent.contains("Pending agent.result"));
        assert!(pending_subagent.contains("queued"));
        assert!(!pending_subagent.contains("Failed agent.result"));

        let process = renderer
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-shell".into(),
                    name: "shell.run".into(),
                    output: serde_json::json!({"stdout": "ok\n", "stderr": "warning\n"})
                        .to_string(),
                    exit_code: 0,
                },
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            })
            .expect("render process")
            .expect("visible process");
        assert!(process.contains("stdout"));
        assert!(process.contains("stderr"));
        assert!(process.contains("warning"));

        let source = renderer
            .tool_completed_with_call(
                1,
                &ToolResult {
                    call_id: "call-read".into(),
                    name: "filesystem.read".into(),
                    output: "fn main() {}\nprintln!(\"ready\");".into(),
                    exit_code: 0,
                },
                0.1,
                0.2,
                Some(&ToolCall {
                    call_id: "call-read".into(),
                    name: "filesystem.read".into(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                }),
            )
            .expect("render source")
            .expect("visible source");
        assert!(source.contains("rust · src/main.rs"));
        assert!(source.contains("1 │ fn main() {}"));
        assert!(source.contains("path=src/main.rs"));

        let edit = renderer
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-edit".into(),
                    name: "patch.apply".into(),
                    output: serde_json::json!({
                        "path": "src/main.rs",
                        "diff": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new"
                    })
                    .to_string(),
                    exit_code: 0,
                },
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            })
            .expect("render edit")
            .expect("visible edit");
        assert!(edit.contains("Changes · src/main.rs"));
        assert!(edit.contains("Added"));
        assert!(edit.contains("Removed"));
        assert!(edit.contains("+new"));

        let error = renderer
            .run_event(&RunEvent::Error {
                code: "provider_unavailable".into(),
                message: "Try another profile.".into(),
                recoverable: true,
                turn: Some(2),
                elapsed_seconds: 1.5,
            })
            .expect("render error")
            .expect("visible error");
        assert!(error.contains("Run error"));
        assert!(error.contains("Try another profile."));
    }

    #[test]
    fn preferences_reconstruct_from_immutable_events_and_validate_schema() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(
            repository.load().expect("defaults"),
            TerminalPreferences::default()
        );
        let preferences = TerminalPreferences {
            theme: ThemeName::HighContrast,
            multiline: true,
            stream_mode: StreamDisplayMode::Off,
            events_mode: EventDisplayMode::Verbose,
            show_reasoning: false,
            transcript_density: TranscriptDensity::Compact,
            ..TerminalPreferences::default()
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
        let invalid = TerminalPreferences {
            schema_version: 2,
            ..TerminalPreferences::default()
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
        let compact = SemanticRenderer::new(TerminalPreferences {
            transcript_density: TranscriptDensity::Compact,
            ..TerminalPreferences::default()
        });
        assert_eq!(
            compact.work_state(&state),
            "[work] session=session-1 tasks=0/0 decisions=0 plans=0 goals=0 agents=0"
        );
        let comfortable = SemanticRenderer::new(TerminalPreferences::default());
        let rendered = comfortable.work_state(&state);
        assert!(rendered.contains("Current work"));
        assert!(rendered.contains("session-1"));
        assert!(rendered.contains("No active tasks or goals."));
    }

    #[test]
    fn provider_events_respect_reasoning_events_and_theme_independently() {
        let renderer = SemanticRenderer::new(TerminalPreferences {
            theme: ThemeName::HighContrast,
            events_mode: EventDisplayMode::Off,
            show_reasoning: true,
            ..TerminalPreferences::default()
        });
        let reasoning = renderer
            .provider_event(&ProviderEvent::ReasoningSummary {
                summary: "safe summary".into(),
            })
            .expect("reasoning")
            .expect("visible reasoning");
        assert!(reasoning.contains("Thinking"));
        assert!(reasoning.contains("safe summary"));
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

        let verbose = SemanticRenderer::new(TerminalPreferences {
            theme: ThemeName::Mono,
            events_mode: EventDisplayMode::Verbose,
            ..TerminalPreferences::default()
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
        let renderer = SemanticRenderer::new(TerminalPreferences::default());
        let input_wait = renderer
            .run_event(&RunEvent::ToolStarted {
                turn: 1,
                call: ToolCall {
                    call_id: "call-user-ask".into(),
                    name: "user.ask".into(),
                    arguments: serde_json::json!({"question": "What should I remember?"}),
                },
                elapsed_seconds: 0.25,
            })
            .expect("render input wait")
            .expect("visible input wait");
        assert_eq!(input_wait, "[input] user.ask waiting for your answer");

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
        assert!(completed.contains("Duration"));
        assert!(completed.contains("1.25s"));
        assert!(completed.contains("README.md"));

        let quiet = SemanticRenderer::new(TerminalPreferences {
            events_mode: EventDisplayMode::Off,
            ..TerminalPreferences::default()
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
        assert!(recoverable.contains("recoverable error"));
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
    fn every_run_event_variant_and_builtin_tool_has_compact_and_verbose_semantics() {
        let provider_events = [
            (
                ProviderEvent::ModelDelta {
                    text: "delta".into(),
                },
                false,
            ),
            (
                ProviderEvent::ReasoningSummary {
                    summary: "safe summary".into(),
                },
                true,
            ),
            (
                ProviderEvent::ToolCallRequested {
                    call_id: "provider-call".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({"text": "hello"}),
                },
                false,
            ),
            (
                ProviderEvent::FinalOutput {
                    text: "final".into(),
                },
                false,
            ),
            (
                ProviderEvent::Usage {
                    usage: ProviderUsage {
                        input_tokens: 4,
                        output_tokens: 2,
                        total_tokens: 6,
                        cached_input_tokens: None,
                        reasoning_tokens: None,
                    },
                },
                false,
            ),
        ];
        for mode in [EventDisplayMode::Compact, EventDisplayMode::Verbose] {
            let renderer = SemanticRenderer::new(TerminalPreferences {
                events_mode: mode,
                show_reasoning: true,
                transcript_density: TranscriptDensity::Compact,
                ..TerminalPreferences::default()
            });
            for (event, compact_visible) in &provider_events {
                let rendered = renderer
                    .run_event(&RunEvent::Provider {
                        event: event.clone(),
                    })
                    .expect("provider event");
                let visible = *compact_visible
                    || mode == EventDisplayMode::Verbose
                        && matches!(event, ProviderEvent::Usage { .. });
                assert_eq!(rendered.is_some(), visible, "{mode:?}: {event:?}");
                assert!(
                    rendered
                        .as_deref()
                        .is_none_or(|value| !value.contains("\x1b["))
                );
            }

            for phase in [
                RunPhase::Preparing,
                RunPhase::WaitingForModel,
                RunPhase::Responding,
                RunPhase::Completed,
            ] {
                assert!(
                    renderer
                        .run_event(&RunEvent::Phase {
                            phase,
                            turn: Some(1),
                            action: Some("acceptance".into()),
                            elapsed_seconds: 0.25,
                        })
                        .expect("phase")
                        .is_some(),
                    "{mode:?}: {phase:?}"
                );
            }
            assert!(
                renderer
                    .run_event(&RunEvent::Error {
                        code: "acceptance_error".into(),
                        message: "bounded safe message".into(),
                        recoverable: true,
                        turn: Some(1),
                        elapsed_seconds: 0.5,
                    })
                    .expect("error")
                    .is_some()
            );

            let registry = StaticToolRegistry::builtins(&builtin_names()).expect("catalog");
            let specs = registry.list_specs();
            assert!(specs.len() >= 50, "built-in catalog unexpectedly shrank");
            for spec in specs {
                let call_id = format!("call-{}", spec.name);
                let started = renderer
                    .run_event(&RunEvent::ToolStarted {
                        turn: 1,
                        call: ToolCall {
                            call_id: call_id.clone(),
                            name: spec.name.clone(),
                            arguments: serde_json::json!({"name": &spec.name, "status": "start"}),
                        },
                        elapsed_seconds: 0.75,
                    })
                    .expect("tool start")
                    .expect("visible tool start");
                let completed = renderer
                    .run_event(&RunEvent::ToolCompleted {
                        turn: 1,
                        result: ToolResult {
                            call_id,
                            name: spec.name.clone(),
                            output: serde_json::json!({"name": &spec.name, "status": "ok"})
                                .to_string(),
                            exit_code: 0,
                        },
                        duration_seconds: 0.25,
                        elapsed_seconds: 1.0,
                    })
                    .expect("tool completion")
                    .expect("visible tool completion");
                for rendered in [started, completed] {
                    assert!(rendered.contains(&spec.name), "{mode:?}: {rendered}");
                    assert!(!rendered.contains("\x1b["), "{mode:?}: {rendered}");
                }
            }
        }
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
            let preferences = TerminalPreferences {
                theme,
                ..TerminalPreferences::default()
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
        let assistant = SemanticRenderer::new(TerminalPreferences {
            theme: ThemeName::Hacker,
            ..TerminalPreferences::default()
        })
        .with_color(true)
        .assistant_text("connected");
        assert!(assistant.contains("\x1b["));
        assert!(assistant.contains("connected"));
    }

    #[test]
    fn bounded_json_and_toml_theme_files_resolve_into_hash_bound_snapshots() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        fs::write(
            themes.join("ocean.json"),
            r##"{
              "schemaVersion": 1,
              "name": "ocean",
              "base": "default",
              "title": "Ocean",
              "caret": ">",
              "continuation": "|",
              "prompt": {
                "left": "#00ffff",
                "indicator": "#00d7ff"
              },
              "styles": {
                "assistant": {"foreground": "#d7ffff"},
                "tool": {"foreground": "#00afff", "bold": true}
              },
              "spinner": "line"
            }"##,
        )
        .expect("ocean");
        fs::write(
            themes.join("ember.toml"),
            r##"schemaVersion = 1
name = "ember"
base = "carrot"
spinner = "aesthetic"

[prompt]
right = "#ffaf5f"

[styles.warning]
foreground = "#ffff00"
bold = true
"##,
        )
        .expect("ember");

        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        assert_eq!(
            library.names(),
            vec![
                "default",
                "mono",
                "high_contrast",
                "carrot",
                "hacker",
                "ember",
                "ocean",
            ]
        );
        let mut preferences = TerminalPreferences::default();
        library
            .select("OCEAN", &mut preferences)
            .expect("select ocean");
        assert_eq!(preferences.theme_name(), "ocean");
        let ocean = preferences.custom_theme.as_ref().expect("snapshot");
        assert_eq!(ocean.source_hash.len(), 64);
        assert_eq!(ocean.prompt_left.expect("left").green, 255);
        assert_eq!(ocean.spinner, colossus_contracts::ThemeSpinner::Line);
        let palette = TerminalPalette::for_preferences(&preferences);
        assert_ne!(
            palette.activity_frame(0.0, false),
            palette.activity_frame(0.1, false)
        );
        let terminal = SemanticRenderer::new(preferences.clone())
            .with_color(true)
            .assistant_text("connected");
        assert!(terminal.contains("38;2;215;255;255"));
        assert!(terminal.contains("connected"));
        assert!(
            !SemanticRenderer::new(preferences)
                .assistant_text("connected")
                .contains("\x1b[")
        );

        let preview = library.preview("ember").expect("preview ember");
        assert_eq!(preview.base, ThemeName::Carrot);
        assert_eq!(preview.spinner, colossus_contracts::ThemeSpinner::Aesthetic);
    }

    #[test]
    fn theme_library_status_is_a_readable_semantic_view() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");

        let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 160)
            .render(&library.status_document("default"));

        assert!(rendered.contains("Themes"));
        assert!(rendered.contains("Active theme: default"));
        assert!(rendered.contains("Active"));
        assert!(rendered.contains("high_contrast"));
        assert!(rendered.contains("Built-in"));
        assert!(rendered.contains("Custom theme search locations"));
        assert!(rendered.contains(&themes.display().to_string()));
        assert!(!rendered.contains("{\"names\""));
        assert!(!rendered.contains("\u{1b}["));
    }

    #[test]
    fn every_builtin_theme_preview_is_visual_bounded_and_ansi_safe() {
        let library = ThemeLibrary::default();
        for name in ["default", "mono", "high_contrast", "carrot", "hacker"] {
            let preferences = library
                .preview_preferences(name, &TerminalPreferences::default())
                .expect("preview preferences");
            let document = library.preview_document(name).expect("preview document");
            for width in [60, 80, 120, 160] {
                let rendered =
                    TerminalDocumentRenderer::new(preferences.clone(), width).render(&document);
                assert!(rendered.contains("theme preview"), "{name}:\n{rendered}");
                assert!(rendered.contains(name), "{name}:\n{rendered}");
                assert!(rendered.contains("Colossus 019f-theme"));
                assert!(rendered.contains("Approval required"));
                assert!(rendered.contains("Needs attention"));
                assert!(rendered.contains("human-first terminal output"));
                assert!(!rendered.contains("\u{1b}["));
                assert!(
                    rendered.lines().all(|line| display_width(line) <= width),
                    "{name} exceeded width {width}:\n{rendered}"
                );
            }
            let colored = TerminalDocumentRenderer::new(preferences, 100)
                .with_color(true)
                .render(&document);
            if name == "mono" {
                assert!(!colored.contains("38;2;"));
            } else {
                assert!(colored.contains("38;2;"), "{name}");
            }
        }
    }

    #[test]
    fn theme_scaffold_is_strict_valid_and_does_not_write_the_suggested_file() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        let scaffold = library.scaffold("Night-Sky").expect("scaffold");
        let suggested = scaffold.suggested_path.clone().expect("suggested path");
        assert_eq!(scaffold.name, "night_sky");
        assert_eq!(suggested, themes.join("night_sky.toml"));
        assert!(!suggested.exists());
        assert!(scaffold.content.contains("schemaVersion = 1"));
        assert!(scaffold.content.contains("name = \"night_sky\""));

        fs::write(&suggested, &scaffold.content).expect("write test scaffold");
        let reloaded = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("valid scaffold");
        assert!(reloaded.names().contains(&"night_sky".into()));
        assert!(library.scaffold("default").is_err());
    }

    #[test]
    fn bundled_ocean_example_remains_a_valid_custom_theme() {
        let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("examples/themes");
        let library = ThemeLibrary::load(&[examples]).expect("example theme library");
        let ocean = library.preview("ocean").expect("ocean example");
        assert_eq!(ocean.base, ThemeName::Default);
        assert_eq!(ocean.title, "Colossus Ocean");
    }

    #[test]
    fn legacy_python_theme_schema_is_strictly_mapped_during_cutover() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        fs::write(
            themes.join("ocean.json"),
            r##"{
              "name": "ocean",
              "title": "Ocean",
              "caret": ">",
              "continuation": "|",
              "styles": {
                "prompt.title": "#00ffff bold",
                "prompt.caret": "bright_cyan"
              },
              "trace": {"tool_call": "bold cyan"},
              "transcript": {
                "assistant": "#d7ffff",
                "tool": "bold blue",
                "activity_spinner": "line"
              }
            }"##,
        )
        .expect("legacy theme");

        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        let ocean = library.preview("ocean").expect("legacy preview");
        assert_eq!(ocean.base, ThemeName::Default);
        assert_eq!(ocean.title, "Ocean");
        assert_eq!(ocean.prompt_left.expect("prompt color").green, 255);
        assert_eq!(ocean.indicator.expect("indicator").blue, 255);
        assert_eq!(ocean.assistant.foreground.expect("assistant").red, 215);
        assert!(ocean.tool.bold);
        assert_eq!(ocean.tool.foreground.expect("tool").green, 255);
        assert_eq!(ocean.spinner, colossus_contracts::ThemeSpinner::Line);

        fs::write(
            themes.join("invalid.json"),
            r#"{"name":"invalid","transcript":{"activity_spinner":"unknown"}}"#,
        )
        .expect("invalid legacy theme");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
    }

    #[test]
    fn custom_theme_snapshot_reconstructs_without_rereading_mutated_source() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        let source = themes.join("stable.json");
        fs::write(
            &source,
            r##"{"schemaVersion":1,"name":"stable","styles":{"assistant":{"foreground":"#010203"}}}"##,
        )
        .expect("theme");
        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        let mut preferences = TerminalPreferences::default();
        library.select("stable", &mut preferences).expect("select");
        let selected_hash = preferences
            .custom_theme
            .as_ref()
            .expect("custom")
            .source_hash
            .clone();
        fs::write(
            source,
            r##"{"schemaVersion":1,"name":"stable","styles":{"assistant":{"foreground":"#ffffff"}}}"##,
        )
        .expect("mutate source");

        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        repository
            .save(
                preferences.clone(),
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .expect("save snapshot");
        let loaded = EventSourcedPresentationRepository::new(journal)
            .load()
            .expect("load snapshot");
        assert_eq!(loaded, preferences);
        assert_eq!(
            loaded.custom_theme.as_ref().expect("custom").source_hash,
            selected_hash
        );
        assert_eq!(
            loaded
                .custom_theme
                .as_ref()
                .expect("custom")
                .assistant
                .foreground
                .expect("color")
                .red,
            1
        );
    }

    #[test]
    fn theme_library_rejects_unknown_fields_collisions_and_symlinks() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        fs::write(
            themes.join("invalid.json"),
            r#"{"schemaVersion":1,"name":"invalid","executable":"no"}"#,
        )
        .expect("invalid");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
        fs::remove_file(themes.join("invalid.json")).expect("remove invalid");
        fs::write(
            themes.join("builtin.toml"),
            "schemaVersion = 1\nname = \"hacker\"\n",
        )
        .expect("builtin");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());

        #[cfg(unix)]
        {
            fs::remove_file(themes.join("builtin.toml")).expect("remove builtin");
            let outside = directory.path().join("outside.json");
            fs::write(&outside, r#"{"schemaVersion":1,"name":"outside"}"#).expect("outside");
            std::os::unix::fs::symlink(&outside, themes.join("linked.json")).expect("symlink");
            assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
        }
    }

    #[test]
    fn theme_library_enforces_file_size_count_and_color_bounds() {
        let directory = tempdir().expect("directory");

        let oversized = directory.path().join("oversized");
        fs::create_dir(&oversized).expect("oversized directory");
        fs::write(
            oversized.join("large.json"),
            vec![b' '; MAX_THEME_FILE_BYTES as usize + 1],
        )
        .expect("oversized theme");
        assert!(ThemeLibrary::load(std::slice::from_ref(&oversized)).is_err());

        let invalid_color = directory.path().join("invalid-color");
        fs::create_dir(&invalid_color).expect("invalid color directory");
        fs::write(
            invalid_color.join("invalid.json"),
            r##"{"schemaVersion":1,"name":"invalid","prompt":{"left":"red"}}"##,
        )
        .expect("invalid color");
        assert!(ThemeLibrary::load(std::slice::from_ref(&invalid_color)).is_err());

        let excess = directory.path().join("excess");
        fs::create_dir(&excess).expect("excess directory");
        for index in 0..=MAX_CUSTOM_THEMES {
            fs::write(
                excess.join(format!("theme-{index:02}.json")),
                format!(r#"{{"schemaVersion":1,"name":"theme_{index:02}"}}"#),
            )
            .expect("theme");
        }
        assert!(ThemeLibrary::load(std::slice::from_ref(&excess)).is_err());

        #[cfg(unix)]
        {
            let real = directory.path().join("real");
            fs::create_dir(&real).expect("real directory");
            let linked = directory.path().join("linked");
            std::os::unix::fs::symlink(real, &linked).expect("directory symlink");
            assert!(ThemeLibrary::load(&[linked]).is_err());
        }
    }
}
