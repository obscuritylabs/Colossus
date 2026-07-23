use super::*;

#[derive(Clone, Debug)]
pub(super) enum TranscriptRenderSource {
    RetainedToolResult {
        title: String,
        name: Option<String>,
        output: String,
    },
    RunEvent {
        event: RunEvent,
        call: Option<colossus_contracts::ToolCall>,
    },
}

impl TranscriptRenderSource {
    pub(super) fn render(&self, preferences: &TerminalPreferences) -> Option<PresentationDocument> {
        let renderer = SemanticRenderer::new(preferences.clone());
        match self {
            Self::RetainedToolResult {
                title,
                name,
                output,
            } => Some(renderer.retained_tool_result_document(
                title.clone(),
                name.as_deref(),
                output.clone(),
            )),
            Self::RunEvent { event, call } => renderer.run_event_document(event, call.as_ref()),
        }
    }
}

pub(super) fn transcript_from_messages(
    messages: Vec<SessionMessage>,
    preferences: &TerminalPreferences,
) -> (Vec<TranscriptEntry>, Vec<Option<TranscriptRenderSource>>) {
    let mut entries = Vec::new();
    let mut sources = Vec::new();
    let mut tool_names = BTreeMap::<String, String>::new();
    for record in messages {
        let (kind, document, source) = match record.message.role {
            ModelMessageRole::System => continue,
            ModelMessageRole::User => (
                TranscriptKind::User,
                PresentationDocument::from_block(PresentationBlock::Markdown(
                    record.message.content,
                )),
                None,
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
                (TranscriptKind::Assistant, document, None)
            }
            ModelMessageRole::Tool => {
                let (title, name) = record.message.tool_call_id.as_ref().map_or_else(
                    || ("Tool result".into(), None),
                    |id| {
                        tool_names.get(id).map_or_else(
                            || (format!("Tool result {id}"), None),
                            |name| (format!("Completed {name}"), Some(name.clone())),
                        )
                    },
                );
                let source = TranscriptRenderSource::RetainedToolResult {
                    title,
                    name,
                    output: record.message.content,
                };
                let document = source
                    .render(preferences)
                    .expect("retained tool results always render");
                (TranscriptKind::Tool, document, Some(source))
            }
        };
        if !document.is_empty() {
            entries.push(TranscriptEntry {
                sequence: Some(record.sequence),
                kind,
                document,
                temporary: false,
            });
            sources.push(source);
        }
    }
    (entries, sources)
}

pub(super) fn user_entry(content: &str, kind: TranscriptKind) -> TranscriptEntry {
    TranscriptEntry {
        sequence: None,
        kind,
        document: PresentationDocument::from_block(PresentationBlock::Markdown(content.into())),
        temporary: false,
    }
}

pub(super) fn error_entry(message: &str) -> TranscriptEntry {
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

pub(super) fn help_document() -> PresentationDocument {
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

pub(super) fn preferences_document(preferences: &TerminalPreferences) -> PresentationDocument {
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

pub(super) fn runtime_command_name(command: &RuntimeCommand) -> &str {
    match command {
        RuntimeCommand::Known { name, .. } => name,
    }
}

pub(super) fn ratatui_style(style: ThemeTextStyle) -> Style {
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

pub(super) fn composer_height(state: &TuiState, width: u16) -> u16 {
    let inner_width = usize::from(width.saturating_sub(4)).max(1);
    let rows = state
        .composer
        .draft
        .split('\n')
        .map(|line| UnicodeWidthStr::width(line).div_ceil(inner_width).max(1))
        .sum::<usize>();
    u16::try_from(rows.clamp(1, 6) + 2).unwrap_or(8)
}

pub(super) fn completion_menu_height(
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

pub(super) fn composer_cursor_position(before: &str, width: usize) -> (usize, usize) {
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

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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

pub(super) fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

pub(super) fn next_boundary(value: &str, cursor: usize) -> usize {
    value[cursor..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(index, _)| cursor + index)
}

pub(super) fn sanitize_input(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .take(1024 * 1024)
        .collect()
}

pub(super) fn truncate_width(value: &str, maximum: usize) -> String {
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
