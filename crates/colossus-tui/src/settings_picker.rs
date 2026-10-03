use super::*;

struct SettingChoice {
    label: &'static str,
    description: &'static str,
    command: &'static str,
    value: &'static str,
}

const SETTING_COMMANDS: &[&str] = &[
    "/permissions",
    "/stream",
    "/events",
    "/reasoning",
    "/transcript",
    "/multiline",
    "/provider diagnostics",
];

pub(super) struct SettingsPickerState {
    title: &'static str,
    note: &'static str,
    choices: Vec<SettingChoice>,
    current: String,
    pub(super) selected: usize,
}

impl SettingsPickerState {
    pub(super) fn for_command(command: &str, state: &TuiState) -> Option<Self> {
        let (title, note, current, choices) = match command.trim() {
            "/permissions" => (
                "Update permissions",
                "Applies to this TUI process. Policy, tool authority, and sandbox boundaries still apply.",
                state.footer.approval_mode.clone(),
                vec![
                    choice(
                        "Deny approvals",
                        "Deny effects that require approval without prompting.",
                        "/permissions deny",
                        "deny",
                    ),
                    choice(
                        "Ask for approval",
                        "Ask before each effect that requires approval.",
                        "/permissions ask",
                        "ask",
                    ),
                    choice(
                        "Approve low risk",
                        "Review eligible low-risk effects automatically; ask for the rest.",
                        "/permissions risk-auto",
                        "risk-auto",
                    ),
                    choice(
                        "Full access",
                        "Approve required effects within policy and sandbox limits.",
                        "/permissions full-access",
                        "full-access",
                    ),
                ],
            ),
            "/stream" => (
                "Response streaming",
                "Saved terminal preference.",
                state.preferences.stream_mode.as_str().into(),
                vec![
                    choice(
                        "On",
                        "Stream response text with configured runtime events.",
                        "/stream on",
                        "on",
                    ),
                    choice(
                        "Text only",
                        "Stream response text without semantic event blocks.",
                        "/stream raw",
                        "raw",
                    ),
                    choice(
                        "Off",
                        "Show the response when the run finishes.",
                        "/stream off",
                        "off",
                    ),
                ],
            ),
            "/events" => (
                "Runtime events",
                "Saved terminal preference.",
                state.preferences.events_mode.as_str().into(),
                vec![
                    choice(
                        "Compact",
                        "Show a concise summary of runtime activity.",
                        "/events compact",
                        "compact",
                    ),
                    choice(
                        "Verbose",
                        "Show detailed runtime activity.",
                        "/events verbose",
                        "verbose",
                    ),
                    choice("Off", "Hide runtime event blocks.", "/events off", "off"),
                ],
            ),
            "/reasoning" => (
                "Reasoning summaries",
                "Saved terminal preference.",
                toggle_value(state.preferences.show_reasoning).into(),
                vec![
                    choice(
                        "On",
                        "Show released reasoning summaries when available.",
                        "/reasoning on",
                        "on",
                    ),
                    choice(
                        "Off",
                        "Hide released reasoning summaries.",
                        "/reasoning off",
                        "off",
                    ),
                ],
            ),
            "/transcript" => (
                "Transcript spacing",
                "Saved terminal preference.",
                state.preferences.transcript_density.as_str().into(),
                vec![
                    choice(
                        "Comfortable",
                        "Use labels and space between transcript blocks.",
                        "/transcript comfortable",
                        "comfortable",
                    ),
                    choice(
                        "Compact",
                        "Use less vertical space between transcript blocks.",
                        "/transcript compact",
                        "compact",
                    ),
                ],
            ),
            "/multiline" => (
                "Enter key behavior",
                "Ctrl+J adds a newline in any terminal. Saved terminal preference.",
                toggle_value(state.preferences.multiline).into(),
                vec![
                    choice(
                        "Enter sends",
                        "Send with Enter; add a newline with Shift+Enter or Ctrl+J.",
                        "/multiline off",
                        "off",
                    ),
                    choice(
                        "Enter adds a newline",
                        "Compose with Enter; send with Ctrl+D or Ctrl+Enter.",
                        "/multiline on",
                        "on",
                    ),
                ],
            ),
            "/provider diagnostics" => (
                "Provider diagnostics",
                "Applies to this TUI process.",
                toggle_value(state.provider_response_diagnostics).into(),
                vec![
                    choice(
                        "Off",
                        "Show concise provider errors.",
                        "/provider diagnostics off",
                        "off",
                    ),
                    choice(
                        "On",
                        "Include provider response diagnostics in errors.",
                        "/provider diagnostics on",
                        "on",
                    ),
                ],
            ),
            _ => return None,
        };
        let selected = choices
            .iter()
            .position(|choice| choice.value == current)
            .unwrap_or(0);
        Some(Self {
            title,
            note,
            choices,
            current,
            selected,
        })
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Option<&'static str> {
        match key.code {
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = (self.selected + self.choices.len() - 1) % self.choices.len()
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1) % self.choices.len()
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.choices.len() - 1,
            KeyCode::Char(digit @ '1'..='9') => {
                let index = digit as usize - '1' as usize;
                if index < self.choices.len() {
                    self.selected = index;
                }
            }
            KeyCode::Enter => return Some(self.choices[self.selected].command),
            _ => {}
        }
        None
    }
}

fn choice(
    label: &'static str,
    description: &'static str,
    command: &'static str,
    value: &'static str,
) -> SettingChoice {
    SettingChoice {
        label,
        description,
        command,
        value,
    }
}

const fn toggle_value(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Keep settings in the root menu; explicit argument input reveals their shortcuts.
pub(super) fn setting_completion_visible(candidate: &str, prefix: &str) -> bool {
    SETTING_COMMANDS.iter().all(|root| {
        candidate.strip_prefix(root).is_none_or(|suffix| {
            !suffix.starts_with(' ')
                || prefix
                    .strip_prefix(root)
                    .is_some_and(|tail| tail.starts_with(' '))
        })
    })
}

pub(super) fn render_settings_picker(
    frame: &mut Frame<'_>,
    state: &TuiState,
    picker: &SettingsPickerState,
    area: Rect,
) {
    frame.render_widget(Clear, area);
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let inset = if area.width >= 80 { 2 } else { 1 };
    let inner = Rect::new(
        area.x + inset,
        area.y + 1,
        area.width.saturating_sub(inset * 2),
        area.height.saturating_sub(2),
    );
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(Span::styled(
            picker.title,
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );
    let wide = inner.width >= 76 && area.height >= 16;
    let label_width = 35;
    let description_offset = label_width + 2;
    let mut y = rows[1].y;
    for (index, choice) in picker.choices.iter().enumerate() {
        let selected = index == picker.selected;
        let style = if selected {
            browser_selection_style(&palette)
        } else {
            ratatui_style(palette.meta_style())
        };
        let label = format!(
            "{} {}. {}{}",
            if selected { "›" } else { " " },
            index + 1,
            choice.label,
            if choice.value == picker.current {
                " (current)"
            } else {
                ""
            }
        );
        let height = if wide {
            u16::try_from(
                wrap_approval_value(
                    choice.description,
                    usize::from(inner.width.saturating_sub(description_offset)),
                )
                .len(),
            )
            .unwrap_or(u16::MAX)
        } else {
            1
        };
        let option_area = Rect::new(inner.x, y, inner.width, height);
        if selected {
            frame.render_widget(Block::default().style(style), option_area);
        }
        frame.render_widget(
            Paragraph::new(truncate_width_with_ellipsis(
                &label,
                if wide {
                    usize::from(label_width)
                } else {
                    usize::from(inner.width)
                },
            ))
            .style(style),
            Rect::new(
                inner.x,
                y,
                if wide { label_width } else { inner.width },
                height,
            ),
        );
        if wide {
            frame.render_widget(
                Paragraph::new(choice.description)
                    .style(style)
                    .wrap(Wrap { trim: true }),
                Rect::new(
                    inner.x + description_offset,
                    y,
                    inner.width.saturating_sub(description_offset),
                    height,
                ),
            );
        }
        y = y.saturating_add(height);
    }
    let mut details = Vec::new();
    if !wide {
        details.push(Line::from(picker.choices[picker.selected].description));
    }
    details.push(Line::from(picker.note));
    if !picker
        .choices
        .iter()
        .any(|choice| choice.value == picker.current)
    {
        details.push(Line::from(format!("Current: {}", picker.current)));
    }
    y = y.saturating_add(u16::from(wide));
    frame.render_widget(
        Paragraph::new(details)
            .style(ratatui_style(palette.meta_style()))
            .wrap(Wrap { trim: true }),
        Rect::new(inner.x, y, inner.width, rows[1].bottom().saturating_sub(y)),
    );
    render_browser_controls(
        frame,
        &palette,
        rows[2],
        &[("↑/↓", "Select"), ("Enter", "Apply"), ("Esc", "Back")],
    );
}
