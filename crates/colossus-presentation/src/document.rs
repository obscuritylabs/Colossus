use super::*;

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

pub(super) fn json_block(value: &Value) -> PresentationBlock {
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

pub(super) fn human_field_name(value: &str) -> String {
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
