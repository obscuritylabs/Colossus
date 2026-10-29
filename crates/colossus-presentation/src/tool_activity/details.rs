use super::*;
use crate::PresentationTable;
use std::io::{self, Write};

const CONTENT_BYTES: usize = TOOL_ACTIVITY_DETAIL_BYTES - 8192;
const CONTENT_LINES: usize = TOOL_ACTIVITY_DETAIL_LINES - 20;
const TRUNCATED: &str = "Details truncated for display; original result is retained.";

struct Budget {
    bytes: usize,
    lines: usize,
    truncated: bool,
}
impl Budget {
    fn new() -> Self {
        Self {
            bytes: CONTENT_BYTES,
            lines: CONTENT_LINES,
            truncated: false,
        }
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
    fn cell(&mut self, value: &Value) -> String {
        let text = match value {
            Value::Null => "—".into(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::String(value) => {
                self.truncated |= value.chars().nth(160).is_some();
                clean(value, 160)
            }
            Value::Array(values) => format!("{} items", values.len()),
            Value::Object(values) => format!("{} fields", values.len()),
        };
        self.text(&text)
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
    if activity.kind == ToolActivityKind::FileRead && activity.phase != ToolActivityPhase::Failed {
        let content = budget.text(output);
        let mut blocks = vec![text_block(activity, content)];
        if budget.truncated || activity.truncated {
            blocks.push(PresentationBlock::Text(TRUNCATED.into()));
        }
        return blocks;
    }
    let parsed = parse_output(output);
    let parsed = if activity.tool_name == "shell.run" {
        let Some(display) = parsed
            .as_ref()
            .and_then(colossus_contracts::command_output_display)
        else {
            return vec![PresentationBlock::Text(
                "Command output unavailable for safe inline display; original result is retained."
                    .into(),
            )];
        };
        Some(display)
    } else {
        parsed
    };
    budget.truncated |= parsed
        .as_ref()
        .and_then(|value| value.get("truncated"))
        .and_then(Value::as_bool)
        == Some(true);
    let mut blocks = Vec::new();
    if activity.tool_name == "shell.run" {
        blocks.push(PresentationBlock::Text(
            "Command details withheld; only released output and status are displayed.".into(),
        ));
    }
    if let Some(value) = parsed.as_ref() {
        if let Some(diff) = value.get("diff").and_then(Value::as_str) {
            let additions = diff
                .lines()
                .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
                .count();
            let removals = diff
                .lines()
                .filter(|line| line.starts_with('-') && !line.starts_with("---"))
                .count();
            blocks.push(PresentationBlock::Card {
                title: value.get("path").and_then(Value::as_str).map_or_else(
                    || "Changes".into(),
                    |path| format!("Changes · {}", clean(path, 120)),
                ),
                tone: PresentationTone::Tool,
                body: vec![
                    PresentationBlock::KeyValue(vec![
                        ("Added".into(), additions.to_string()),
                        ("Removed".into(), removals.to_string()),
                    ]),
                    PresentationBlock::Diff(budget.text(diff)),
                ],
            });
        } else if let Some(stream) = value.get("stdout").and_then(Value::as_str).filter(|_| {
            matches!(
                activity.kind,
                ToolActivityKind::Process | ToolActivityKind::Git
            )
        }) {
            if !stream.is_empty() {
                blocks.push(PresentationBlock::Code {
                    language: Some("stdout".into()),
                    content: budget.text(stream),
                });
            }
            if let Some(stderr) = value
                .get("stderr")
                .and_then(Value::as_str)
                .filter(|stream| !stream.is_empty())
            {
                blocks.push(PresentationBlock::Code {
                    language: Some("stderr".into()),
                    content: budget.text(stderr),
                });
            }
        } else if let Some(records) = value.as_array().or_else(|| {
            [
                "files",
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
            .find_map(|key| value.get(key).and_then(Value::as_array))
        }) {
            blocks.push(records_table(records, &mut budget));
        } else if let Some(text) = value.as_str() {
            blocks.push(text_block(activity, budget.text(text)));
        } else {
            let (json, truncated) = json_prefix(value, budget.bytes, budget.lines, true);
            budget.truncated |= truncated;
            blocks.push(PresentationBlock::Code {
                language: Some("json preview".into()),
                content: budget.text(&json),
            });
        }
    } else {
        blocks.push(text_block(activity, budget.text(output)));
    }
    if budget.truncated || activity.truncated {
        blocks.push(PresentationBlock::Text(TRUNCATED.into()));
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
    let keys = records.first().and_then(Value::as_object).map(|row| {
        row.keys()
            .take(TOOL_ACTIVITY_DETAIL_COLUMNS)
            .map(String::as_str)
            .collect::<Vec<_>>()
    });
    let mut headers = Vec::new();
    if let Some(keys) = &keys {
        for (index, key) in keys.iter().enumerate() {
            budget.truncated |= key.chars().nth(80).is_some();
            let clean = clean(key, 80);
            let mut label = clean;
            let mut suffix = index + 1;
            while headers.contains(&label) {
                label = format!("{} [{suffix}]", super::clean(key, 72));
                suffix += 1;
            }
            headers.push(label);
        }
    } else {
        headers.push("Value".into());
    }
    budget.truncated |= records.len() > TOOL_ACTIVITY_DETAIL_ROWS
        || records
            .first()
            .and_then(Value::as_object)
            .is_some_and(|row| row.len() > TOOL_ACTIVITY_DETAIL_COLUMNS);
    let mut table = PresentationTable::new(headers, "No items.");
    for record in records.iter().take(TOOL_ACTIVITY_DETAIL_ROWS) {
        let row = if let (Some(record), Some(keys)) = (record.as_object(), keys.as_ref()) {
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
