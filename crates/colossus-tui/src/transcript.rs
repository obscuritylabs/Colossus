use super::*;

#[derive(Clone, Debug)]
pub(super) enum TranscriptRenderSource {
    RetainedToolResult {
        title: String,
        name: Option<String>,
        output: String,
    },
    RunEvent {
        event: Box<RunEvent>,
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
            Self::RunEvent { event, call } => {
                renderer.run_event_document(event.as_ref(), call.as_ref())
            }
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

pub(super) fn help_document(completions: &[String]) -> PresentationDocument {
    let mut grouped = BTreeMap::<&'static str, Vec<&str>>::new();
    for command in completions
        .iter()
        .map(String::as_str)
        .filter(|command| command.starts_with('/'))
    {
        let commands = grouped.entry(command_help_category(command)).or_default();
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    let mut command_groups = Vec::new();
    for (category, purpose) in [
        ("Conversation", "Resume and manage durable sessions"),
        ("Work", "Inspect and drive plans, goals, tasks, and agents"),
        (
            "Memory & context",
            "Recall memory and manage context snapshots",
        ),
        ("Agent resources", "Discover tools and activate skills"),
        (
            "Research & connections",
            "Use research, integrations, and MCP",
        ),
        (
            "Extensions",
            "Inspect and manage packs, collections, and bundles",
        ),
        (
            "Runtime",
            "Inspect workflows, telemetry, audit, and projections",
        ),
        (
            "Provider diagnostics",
            "Inspect model and provider readiness",
        ),
        ("Appearance", "Tune themes and terminal presentation"),
        ("Terminal", "Get help or exit safely"),
        ("Other", "Additional host commands"),
    ] {
        let Some(commands) = grouped.remove(category) else {
            continue;
        };
        command_groups.push((
            category.into(),
            format!("{purpose}: {}", commands.join(" · ")),
        ));
    }
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Colossus terminal".into(),
        tone: PresentationTone::Neutral,
        body: vec![
            PresentationBlock::Text(
                "Type a message to run the agent. Slash commands operate durable state. This list is generated from the commands available in the current terminal.".into(),
            ),
            PresentationBlock::KeyValue(vec![
                (
                    "Send".into(),
                    "Enter; Ctrl/Alt+Enter in multiline mode".into(),
                ),
                (
                    "Scroll".into(),
                    "Mouse wheel uses native scrollback; --alt-screen uses captured wheel or PageUp/PageDown".into(),
                ),
                (
                    "Complete".into(),
                    "Type / or @ for suggestions; Up/Down select; Tab or Right accepts".into(),
                ),
                (
                    "History".into(),
                    "Up/Down at boundaries; Ctrl-R searches".into(),
                ),
                (
                    "Cancel".into(),
                    "Ctrl-C cancels an active run; press again to exit".into(),
                ),
            ]),
            PresentationBlock::Markdown("## Command families".into()),
            PresentationBlock::KeyValue(command_groups),
        ],
    })
}

fn command_help_category(command: &str) -> &'static str {
    match command {
        command if command == "/resume" || command.starts_with("/session") => "Conversation",
        command
            if command == "/work"
                || command == "/tasks"
                || command == "/decisions"
                || command.starts_with("/plan")
                || command.starts_with("/goal")
                || command.starts_with("/agents") =>
        {
            "Work"
        }
        command if command.starts_with("/memor") || command.starts_with("/context") => {
            "Memory & context"
        }
        command
            if command == "/tools"
                || command.starts_with("/skill")
                || command.starts_with("/skills") =>
        {
            "Agent resources"
        }
        command
            if command.starts_with("/research")
                || command.starts_with("/integration")
                || command.starts_with("/mcp") =>
        {
            "Research & connections"
        }
        command
            if command.starts_with("/packs")
                || command.starts_with("/collections")
                || command.starts_with("/registry")
                || command.starts_with("/bundle") =>
        {
            "Extensions"
        }
        command
            if command.starts_with("/workflow")
                || command.starts_with("/telemetry")
                || command.starts_with("/audit")
                || command.starts_with("/projection")
                || command == "/trace" =>
        {
            "Runtime"
        }
        command if command.starts_with("/models") || command.starts_with("/provider") => {
            "Provider diagnostics"
        }
        command
            if command.starts_with("/theme")
                || command.starts_with("/stream")
                || command.starts_with("/events")
                || command.starts_with("/reasoning")
                || command.starts_with("/transcript")
                || command.starts_with("/multiline")
                || command.starts_with("/tui") =>
        {
            "Appearance"
        }
        "/help" | "/exit" | "/quit" => "Terminal",
        _ => "Other",
    }
}

pub(super) fn plan_status_document(
    mode: InteractiveMode,
    selected: Option<&PlanRecord>,
) -> PresentationDocument {
    let mut rows = vec![("Mode".into(), mode.as_str().into())];
    if let Some(plan) = selected {
        rows.extend([
            ("Selected plan".into(), plan.id.clone()),
            ("Revision".into(), plan.revision.to_string()),
            ("Plan status".into(), plan_status_label(plan.status).into()),
            (
                "Plan steps".into(),
                format!(
                    "{} ordered step{} (separate from durable /tasks)",
                    plan.steps.len(),
                    if plan.steps.len() == 1 { "" } else { "s" }
                ),
            ),
            (
                "Next action".into(),
                match plan.status {
                    PlanStatus::Draft => "Review, refine, approve for execution, or discard".into(),
                    PlanStatus::Approved => "Choose Direct or Goal Mode execution".into(),
                    PlanStatus::Executed => "Inspect the linked execution evidence".into(),
                    PlanStatus::Discarded => "Create or select another plan".into(),
                },
            ),
        ]);
    } else {
        rows.push(("Selected plan".into(), "none".into()));
    }
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Plan workflow".into(),
        tone: PresentationTone::Neutral,
        body: vec![PresentationBlock::KeyValue(rows)],
    })
}

pub(super) fn research_status_document(mode: InteractiveMode) -> PresentationDocument {
    let messages = match mode {
        InteractiveMode::Research => "Run bounded source-backed research",
        InteractiveMode::Execute => "Run normal agent turns",
        InteractiveMode::Plan => "Create or refine the selected plan",
    };
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Research mode".into(),
        tone: PresentationTone::Neutral,
        body: vec![PresentationBlock::KeyValue(vec![
            ("Mode".into(), mode.as_str().into()),
            ("Messages".into(), messages.into()),
        ])],
    })
}

pub(super) fn provider_diagnostics_document(enabled: bool) -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Provider response diagnostics".into(),
        tone: if enabled {
            PresentationTone::Warning
        } else {
            PresentationTone::Neutral
        },
        body: vec![PresentationBlock::Text(if enabled {
            concat!(
                "Enabled for model turns in this TUI process. A failed provider request will ",
                "show the exact provider-facing JSON and up to 16 KiB of response body after ",
                "configured-credential redaction and post-effect policy. The diagnostic is not ",
                "written to durable run history, but the request can contain user, session, and ",
                "tool-result data. Use /provider diagnostics off when finished."
            )
            .into()
        } else {
            "Disabled. Provider failures will show status-only diagnostics.".into()
        })],
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
        RuntimeCommand::Plan(_) => "plan",
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
        .saturating_sub(MINIMUM_COMPLETION_TRANSCRIPT_ROWS)
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
