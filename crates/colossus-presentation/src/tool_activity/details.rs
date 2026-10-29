use super::*;
use crate::PresentationTable;
use std::io::{self, Write};

const CONTENT_BYTES: usize = TOOL_ACTIVITY_DETAIL_BYTES - 8192;
const CONTENT_LINES: usize = TOOL_ACTIVITY_DETAIL_LINES - 20;
const MAX_DEPTH: usize = 6;
const MAX_FIELDS: usize = 24;
const TRUNCATED: &str = "Details truncated for display; original result is retained.";

struct Budget {
    bytes: usize,
    lines: usize,
    nodes: usize,
    truncated: bool,
}
impl Budget {
    fn new() -> Self {
        Self {
            bytes: CONTENT_BYTES,
            lines: CONTENT_LINES,
            nodes: 64,
            truncated: false,
        }
    }
    fn reserve_lines(&mut self, count: usize) -> bool {
        if self.lines < count || self.bytes == 0 {
            self.truncated = true;
            return false;
        }
        self.lines -= count;
        true
    }
    fn enter(&mut self, depth: usize) -> bool {
        if depth > MAX_DEPTH || self.nodes == 0 || self.lines == 0 || self.bytes == 0 {
            self.truncated = true;
            return false;
        }
        self.nodes -= 1;
        true
    }
    fn text(&mut self, text: &str) -> String {
        let mut end = 0;
        let mut lines = 1;
        for (offset, character) in text.char_indices() {
            if offset.saturating_add(character.len_utf8()) > self.bytes || lines > self.lines {
                break;
            }
            end = offset + character.len_utf8();
            if character == '\n' {
                lines += 1;
            }
        }
        self.truncated |= end < text.len();
        self.bytes = self.bytes.saturating_sub(end);
        self.lines = self.lines.saturating_sub(lines);
        crate::terminal::sanitize_terminal_text(&text[..end])
    }
    fn short(&mut self, text: &str, maximum: usize) -> String {
        self.truncated |= text.chars().nth(maximum).is_some();
        let text = clean(text, maximum);
        let mut end = text.len().min(self.bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.truncated |= end < text.len();
        self.bytes -= end;
        text[..end].into()
    }
    fn label(&mut self, name: &str) -> String {
        let label = self.short(name, 80);
        crate::human_field_name(&label)
    }
    fn cell(&mut self, value: &Value) -> String {
        let text = match value {
            Value::Null => "—".into(),
            Value::Bool(true) => "yes".into(),
            Value::Bool(false) => "no".into(),
            Value::Number(value) => value.to_string(),
            Value::String(value) => return self.short(value, 160),
            Value::Array(values) => {
                self.truncated |= !values.is_empty();
                format!("{} items", values.len())
            }
            Value::Object(values) => {
                self.truncated |= !values.is_empty();
                format!("{} fields", values.len())
            }
        };
        self.short(&text, 160)
    }
}

pub(super) fn output_blocks(
    activity: &ToolActivityPresentation,
    output: &str,
) -> Vec<PresentationBlock> {
    if activity.kind == ToolActivityKind::Computer && activity.phase == ToolActivityPhase::Failed {
        let parsed = parse_output(output);
        let category = parsed
            .as_ref()
            .and_then(|value| {
                value
                    .pointer("/error/type")
                    .or_else(|| value.pointer("/error/code"))
            })
            .and_then(Value::as_str);
        let (category, reason) = match category {
            Some("denied" | "tool.denied") => ("denied", "Computer request denied"),
            Some("outcome_unknown" | "tool.outcome_unknown") => {
                ("outcome unknown", "Verify the target before retrying")
            }
            Some("invalid_arguments" | "validation_error") => {
                ("invalid arguments", "Check the tool's required arguments")
            }
            Some("unknown_tool") => ("unavailable", "Computer tool unavailable"),
            _ => ("execution error", "Computer request could not complete"),
        };
        return vec![PresentationBlock::KeyValue(vec![
            ("Category".into(), category.into()),
            ("Reason".into(), reason.into()),
        ])];
    }
    let mut budget = Budget::new();
    // File bytes are opaque, even when they happen to contain a tool-shaped JSON object.
    if activity.kind == ToolActivityKind::FileRead && activity.phase != ToolActivityPhase::Failed {
        let mut blocks = vec![text_block(activity, budget.text(output))];
        finish(&mut blocks, &budget, activity);
        return blocks;
    }
    let parsed = parse_output(output);
    let shell = matches!(
        activity.tool_name.as_str(),
        "shell.run" | "shell.read" | "shell.wait" | "shell.stop" | "shell.list"
    );
    let parsed = if shell {
        let display = parsed.as_ref().and_then(|value| {
            if activity.tool_name == "shell.list" {
                command_list_display(value)
            } else {
                colossus_contracts::command_output_display(value)
            }
        });
        let Some(display) = display else {
            return vec![PresentationBlock::Text(
                "Command output unavailable for safe inline display; original result is retained."
                    .into(),
            )];
        };
        Some(display)
    } else {
        parsed
    };
    let mut blocks = Vec::new();
    if shell {
        blocks.push(PresentationBlock::Text(
            "Command details withheld; only released output and status are displayed.".into(),
        ));
    }
    if let Some(value) = parsed.as_ref() {
        if activity.kind == ToolActivityKind::Mcp {
            blocks.extend(mcp_blocks(value, activity, &mut budget));
        } else {
            blocks.extend(structured_blocks(value, activity, &mut budget, 0));
        }
    } else if output.len() > TOOL_ACTIVITY_DETAIL_BYTES
        && output.trim_start().starts_with(['{', '['])
    {
        // Never parse an unbounded result or turn an oversized JSON envelope into a wall of code.
        budget.truncated = true;
        blocks.push(PresentationBlock::Text(
            "Structured result exceeds the detail display limit; its original output is retained."
                .into(),
        ));
    } else {
        blocks.push(text_block(activity, budget.text(output)));
    }
    finish(&mut blocks, &budget, activity);
    blocks
}

// Managed discovery is a different typed shape from a stream snapshot. Keep only
// lifecycle/output metadata, matching command_output_display's disclosure boundary.
fn command_list_display(value: &Value) -> Option<Value> {
    let observation = value.get("_colossusToolObservation");
    let value = if let Some(metadata) = observation {
        if metadata.get("format").and_then(Value::as_str) != Some("json") {
            return None;
        }
        value.get("data")?
    } else {
        value
    };
    let page: colossus_contracts::ProcessSessionPage =
        serde_json::from_value(value.clone()).ok()?;
    let sessions = page.sessions.iter().take(TOOL_ACTIVITY_DETAIL_ROWS).enumerate().map(|(index, session)| serde_json::json!({
        "number":index + 1, "status":session.status, "lifetime":session.lifetime,
        "exit_code":session.exit_code, "truncated":session.truncated, "output_sequence":session.output_sequence,
    })).collect::<Vec<_>>();
    Some(serde_json::json!({
        "session_count":page.sessions.len(), "sessions":sessions, "more":page.next_cursor.is_some(),
        "truncated":observation.is_some() || page.sessions.len() > TOOL_ACTIVITY_DETAIL_ROWS,
    }))
}

fn finish(
    blocks: &mut Vec<PresentationBlock>,
    budget: &Budget,
    activity: &ToolActivityPresentation,
) {
    if budget.truncated || activity.truncated {
        blocks.push(PresentationBlock::Text(TRUNCATED.into()));
    }
}

fn structured_blocks(
    value: &Value,
    activity: &ToolActivityPresentation,
    budget: &mut Budget,
    depth: usize,
) -> Vec<PresentationBlock> {
    if !budget.enter(depth) {
        return Vec::new();
    }
    match value {
        Value::Object(object) => {
            budget.truncated |= object.get("truncated").and_then(Value::as_bool) == Some(true);
            let diff = object.get("diff").and_then(Value::as_str);
            let streams = matches!(
                activity.kind,
                ToolActivityKind::Process | ToolActivityKind::Git
            );
            let mut omit = Vec::new();
            if diff.is_some() {
                omit.push("diff");
            }
            if streams {
                omit.extend(["stdout", "stderr"]);
            }
            let mut blocks = object_blocks(object, activity, budget, depth, &omit);
            if let Some(diff) = diff {
                let additions = diff
                    .lines()
                    .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
                    .count();
                let removals = diff
                    .lines()
                    .filter(|line| line.starts_with('-') && !line.starts_with("---"))
                    .count();
                if budget.reserve_lines(4) {
                    let title = object.get("path").and_then(Value::as_str).map_or_else(
                        || "Changes".into(),
                        |path| format!("Changes · {}", budget.short(path, 120)),
                    );
                    blocks.push(PresentationBlock::Card {
                        title,
                        tone: PresentationTone::Tool,
                        body: vec![
                            PresentationBlock::KeyValue(vec![
                                ("Added".into(), additions.to_string()),
                                ("Removed".into(), removals.to_string()),
                            ]),
                            PresentationBlock::Diff(budget.text(diff)),
                        ],
                    });
                }
            }
            if streams {
                for stream in ["stdout", "stderr"] {
                    if let Some(text) = object
                        .get(stream)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                    {
                        blocks.push(
                            if matches!(activity.tool_name.as_str(), "git.diff" | "git.show")
                                && stream == "stdout"
                            {
                                PresentationBlock::Diff(budget.text(text))
                            } else {
                                PresentationBlock::Code {
                                    language: Some(stream.into()),
                                    content: budget.text(text),
                                }
                            },
                        );
                    }
                }
            }
            blocks
        }
        Value::Array(records) => array_blocks(records, activity, budget, depth),
        Value::String(text) => vec![text_block(activity, budget.text(text))],
        _ => vec![PresentationBlock::Text(budget.cell(value))],
    }
}

fn object_blocks(
    object: &serde_json::Map<String, Value>,
    activity: &ToolActivityPresentation,
    budget: &mut Budget,
    depth: usize,
    omit: &[&str],
) -> Vec<PresentationBlock> {
    let keys = ordered_keys(object, omit);
    budget.truncated |= object
        .len()
        .saturating_sub(omit.iter().filter(|key| object.contains_key(**key)).count())
        > keys.len();
    let mut fields = Vec::new();
    for key in &keys {
        let value = &object[*key];
        if !value.is_array() && !value.is_object() && !long_text(value) {
            if !budget.reserve_lines(1) {
                break;
            }
            fields.push((budget.label(key), budget.cell(value)));
        }
    }
    let mut blocks = Vec::new();
    if !fields.is_empty() {
        blocks.push(PresentationBlock::KeyValue(fields));
    }
    for key in keys {
        let value = &object[key];
        if !value.is_array() && !value.is_object() && !long_text(value) {
            continue;
        }
        if !budget.reserve_lines(2) {
            break;
        }
        let title = budget.label(key);
        match value {
            Value::Array(records) => {
                blocks.push(PresentationBlock::Text(title));
                blocks.extend(array_blocks(records, activity, budget, depth + 1));
            }
            _ => {
                let body = structured_blocks(value, activity, budget, depth + 1);
                if !body.is_empty() {
                    blocks.push(PresentationBlock::Card {
                        title,
                        tone: PresentationTone::Neutral,
                        body,
                    });
                }
            }
        }
    }
    blocks
}

fn array_blocks(
    records: &[Value],
    activity: &ToolActivityPresentation,
    budget: &mut Budget,
    depth: usize,
) -> Vec<PresentationBlock> {
    let visible = &records[..records.len().min(TOOL_ACTIVITY_DETAIL_ROWS)];
    let all_objects = visible.iter().all(Value::is_object);
    let no_objects = visible
        .iter()
        .all(|value| !value.is_object() && !value.is_array());
    let flat_objects = all_objects
        && visible.iter().filter_map(Value::as_object).all(|object| {
            object
                .values()
                .all(|value| !value.is_object() && !value.is_array())
        });
    if flat_objects || no_objects {
        return vec![records_table(records, budget)];
    }
    budget.truncated |= visible.len() < records.len();
    let mut blocks = Vec::new();
    for (index, record) in visible.iter().enumerate() {
        if !budget.reserve_lines(2) {
            break;
        }
        let body = structured_blocks(record, activity, budget, depth + 1);
        if !body.is_empty() {
            blocks.push(PresentationBlock::Card {
                title: format!("Item {}", index + 1),
                tone: PresentationTone::Neutral,
                body,
            });
        }
    }
    blocks
}

fn long_text(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.contains('\n') || text.chars().nth(160).is_some())
}

fn ordered_keys<'a>(object: &'a serde_json::Map<String, Value>, omit: &[&str]) -> Vec<&'a str> {
    let mut keys = Vec::new();
    for key in [
        "status",
        "root",
        "path",
        "name",
        "title",
        "id",
        "file_count",
        "match_count",
        "exit_code",
        "duration_seconds",
        "truncated",
        "message",
        "reason",
    ] {
        if object.contains_key(key) && !omit.contains(&key) {
            keys.push(key);
        }
    }
    for key in object.keys().map(String::as_str) {
        if keys.len() >= MAX_FIELDS {
            break;
        }
        if !omit.contains(&key) && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn mcp_blocks(
    value: &Value,
    activity: &ToolActivityPresentation,
    budget: &mut Budget,
) -> Vec<PresentationBlock> {
    let result = value.get("result").unwrap_or(value);
    let Some(object) = result.as_object().filter(|object| {
        object.contains_key("content")
            || object.contains_key("structuredContent")
            || object.contains_key("structured_content")
    }) else {
        return structured_blocks(value, activity, budget, 0);
    };
    let mut blocks = Vec::new();
    if let Some(envelope) = value.as_object().filter(|_| value.get("result").is_some()) {
        blocks.extend(object_blocks(envelope, activity, budget, 0, &["result"]));
    }
    blocks.extend(object_blocks(
        object,
        activity,
        budget,
        0,
        &["content", "structuredContent", "structured_content"],
    ));
    let structured = object
        .get("structuredContent")
        .or_else(|| object.get("structured_content"));
    if let Some(structured) = structured {
        blocks.extend(structured_blocks(structured, activity, budget, 0));
    }
    if let Some(content) = object.get("content").and_then(Value::as_array) {
        budget.truncated |= content.len() > TOOL_ACTIVITY_DETAIL_ROWS;
        for item in content.iter().take(TOOL_ACTIVITY_DETAIL_ROWS) {
            if !budget.enter(0) {
                break;
            }
            if let Some(text) = item
                .get("text")
                .and_then(Value::as_str)
                .filter(|_| item.get("type").and_then(Value::as_str) == Some("text"))
            {
                if let Some(parsed) = parse_output(text) {
                    if structured != Some(&parsed) {
                        blocks.extend(structured_blocks(&parsed, activity, budget, 0));
                    }
                } else {
                    blocks.push(text_block(activity, budget.text(text)));
                }
            } else if matches!(
                item.get("type").and_then(Value::as_str),
                Some("image" | "audio")
            ) {
                if budget.reserve_lines(2) {
                    let mut fields = vec![("Content".into(), budget.cell(&item["type"]))];
                    if let Some(mime) = item.get("mimeType") {
                        fields.push(("Format".into(), budget.cell(mime)));
                    }
                    blocks.push(PresentationBlock::KeyValue(fields));
                }
            } else if item.get("type").and_then(Value::as_str) == Some("resource")
                && item.get("resource").and_then(Value::as_object).is_some()
            {
                let resource = item["resource"]
                    .as_object()
                    .expect("resource object checked");
                blocks.extend(object_blocks(
                    resource,
                    activity,
                    budget,
                    0,
                    &["blob", "text"],
                ));
                if let Some(text) = resource.get("text").and_then(Value::as_str) {
                    blocks.push(text_block(activity, budget.text(text)));
                }
            } else {
                blocks.extend(structured_blocks(item, activity, budget, 0));
            }
        }
    } else if let Some(content) = object.get("content") {
        blocks.extend(structured_blocks(content, activity, budget, 0));
    }
    blocks
}

fn text_block(activity: &ToolActivityPresentation, text: String) -> PresentationBlock {
    if matches!(activity.tool_name.as_str(), "git.diff" | "git.show") {
        return PresentationBlock::Diff(text);
    }
    let language = if activity.kind == ToolActivityKind::FileRead {
        let path = activity.context.as_deref().unwrap_or("file");
        let extension = path.rsplit('.').next().unwrap_or("");
        let language = match extension {
            "rs" => "rust",
            "py" => "python",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" | "tsx" => "typescript",
            "md" => "markdown",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            "sh" | "bash" => "shell",
            _ => "file",
        };
        Some(format!("{language} · {path}"))
    } else {
        None
    };
    PresentationBlock::Code {
        language,
        content: text,
    }
}

fn records_table(records: &[Value], budget: &mut Budget) -> PresentationBlock {
    let objects = records
        .iter()
        .take(TOOL_ACTIVITY_DETAIL_ROWS)
        .filter_map(Value::as_object)
        .collect::<Vec<_>>();
    let mut keys = Vec::<&str>::new();
    for preferred in [
        "path",
        "name",
        "title",
        "status",
        "id",
        "kind",
        "type",
        "line",
        "text",
        "summary",
        "description",
    ] {
        if objects.iter().any(|row| row.contains_key(preferred)) {
            keys.push(preferred);
        }
    }
    for row in &objects {
        for key in row.keys().map(String::as_str) {
            if keys.len() >= TOOL_ACTIVITY_DETAIL_COLUMNS {
                break;
            }
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys.truncate(TOOL_ACTIVITY_DETAIL_COLUMNS);
    budget.truncated |= records.len() > TOOL_ACTIVITY_DETAIL_ROWS
        || objects
            .iter()
            .any(|row| row.keys().any(|key| !keys.contains(&key.as_str())));
    let mut headers = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let original = budget.label(key);
        let mut label = original.clone();
        let mut suffix = index + 1;
        while headers.contains(&label) {
            label = format!("{original} [{suffix}]");
            suffix += 1;
        }
        headers.push(label);
    }
    if headers.is_empty() {
        headers.push("Value".into());
    }
    let mut table = PresentationTable::new(headers, "No items.");
    if !budget.reserve_lines(2) {
        return PresentationBlock::Table(table);
    }
    for record in records.iter().take(TOOL_ACTIVITY_DETAIL_ROWS) {
        if !budget.reserve_lines(1) {
            break;
        }
        let row = if let Some(record) = record.as_object().filter(|_| !keys.is_empty()) {
            keys.iter()
                .map(|key| {
                    record
                        .get(*key)
                        .map_or_else(|| "—".into(), |value| budget.cell(value))
                })
                .collect::<Vec<_>>()
        } else {
            vec![budget.cell(record)]
        };
        table.push_row(row);
    }
    PresentationBlock::Table(table)
}

struct PrefixWriter {
    bytes: Vec<u8>,
    limit: usize,
    lines: usize,
    truncated: bool,
}
impl Write for PrefixWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let byte_limit = self.limit.saturating_sub(self.bytes.len());
        let mut take = buffer.len().min(byte_limit);
        for (index, byte) in buffer.iter().take(take).enumerate() {
            if *byte == b'\n' {
                if self.lines <= 1 {
                    take = index;
                    break;
                }
                self.lines -= 1;
            }
        }
        self.bytes.extend_from_slice(&buffer[..take]);
        if take < buffer.len() {
            self.truncated = true;
            return Err(io::Error::other("presentation limit reached"));
        }
        Ok(take)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn json_prefix(
    value: &Value,
    bytes: usize,
    lines: usize,
    pretty: bool,
) -> (String, bool) {
    let mut writer = PrefixWriter {
        bytes: Vec::new(),
        limit: bytes,
        lines,
        truncated: false,
    };
    let result = if pretty {
        serde_json::to_writer_pretty(&mut writer, value)
    } else {
        serde_json::to_writer(&mut writer, value)
    };
    let truncated = writer.truncated || result.is_err();
    while std::str::from_utf8(&writer.bytes).is_err() {
        writer.bytes.pop();
    }
    (
        String::from_utf8(writer.bytes).unwrap_or_default(),
        truncated,
    )
}

#[cfg(test)]
#[path = "detail_tests.rs"]
mod tests;
