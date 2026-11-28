use super::*;
use colossus_contracts::{
    ModelFeatureMode, ReasoningEffort, TerminalModelCard, TerminalModelCardUpdate,
    TerminalModelOverrides,
};

#[derive(Clone, Copy)]
enum Field {
    Context,
    Input,
    Output,
    Tools,
    Streaming,
    Images,
    Compaction,
    Reasoning,
}

const FIELDS: [Field; 8] = [
    Field::Context,
    Field::Input,
    Field::Output,
    Field::Tools,
    Field::Streaming,
    Field::Images,
    Field::Compaction,
    Field::Reasoning,
];
const FEATURE_CHOICES: [Option<ModelFeatureMode>; 4] = [
    None,
    Some(ModelFeatureMode::Auto),
    Some(ModelFeatureMode::On),
    Some(ModelFeatureMode::Off),
];
const REASONING_CHOICES: [Option<ReasoningEffort>; 9] = [
    None,
    Some(ReasoningEffort::None),
    Some(ReasoningEffort::Minimal),
    Some(ReasoningEffort::Low),
    Some(ReasoningEffort::Medium),
    Some(ReasoningEffort::High),
    Some(ReasoningEffort::XHigh),
    Some(ReasoningEffort::Max),
    Some(ReasoningEffort::Ultra),
];

impl Field {
    const fn label(self) -> &'static str {
        match self {
            Self::Context => "Context window",
            Self::Input => "Input token budget",
            Self::Output => "Max output tokens",
            Self::Tools => "Tool calls",
            Self::Streaming => "Streaming",
            Self::Images => "Image inputs",
            Self::Compaction => "Server compaction",
            Self::Reasoning => "Reasoning effort",
        }
    }

    const fn token_index(self) -> Option<usize> {
        match self {
            Self::Context => Some(0),
            Self::Input => Some(1),
            Self::Output => Some(2),
            _ => None,
        }
    }
}

pub(super) enum EditorAction {
    Continue,
    Back,
    Save(TerminalModelCardUpdate),
}

pub(super) struct ModelEditor {
    card: TerminalModelCard,
    overrides: TerminalModelOverrides,
    tokens: [String; 3],
    selected: usize,
}

impl ModelEditor {
    pub(super) fn new(card: TerminalModelCard) -> Self {
        let overrides = card.overrides.clone();
        let tokens = [
            overrides.context_window_tokens,
            overrides.input_budget_tokens,
            overrides.max_output_tokens,
        ]
        .map(|value| value.map_or_else(String::new, |value| value.to_string()));
        Self {
            card,
            overrides,
            tokens,
            selected: 0,
        }
    }

    fn field(&self) -> Field {
        FIELDS[self.selected]
    }

    fn active_tokens(&mut self) -> Option<&mut String> {
        self.field()
            .token_index()
            .map(|index| &mut self.tokens[index])
    }

    fn feature_modes(&self, field: Field) -> Option<(Option<ModelFeatureMode>, ModelFeatureMode)> {
        let defaults = self.card.default_features;
        Some(match field {
            Field::Tools => (self.overrides.tool_calls, defaults.tool_calls),
            Field::Streaming => (self.overrides.streaming, defaults.streaming),
            Field::Images => (self.overrides.image_inputs, defaults.image_inputs),
            Field::Compaction => (self.overrides.server_compaction, defaults.server_compaction),
            _ => return None,
        })
    }

    fn active_feature(&mut self) -> Option<&mut Option<ModelFeatureMode>> {
        match self.field() {
            Field::Tools => Some(&mut self.overrides.tool_calls),
            Field::Streaming => Some(&mut self.overrides.streaming),
            Field::Images => Some(&mut self.overrides.image_inputs),
            Field::Compaction => Some(&mut self.overrides.server_compaction),
            _ => None,
        }
    }

    pub(super) fn paste(&mut self, text: &str) -> Result<(), String> {
        let Some(tokens) = self.active_tokens() else {
            return Ok(());
        };
        let mut digits = String::new();
        for character in text.chars() {
            if character.is_ascii_digit() {
                if tokens.len() + digits.len() >= 20 {
                    return Err("Token counts must fit within 20 digits.".into());
                }
                digits.push(character);
            } else if character != ',' && !character.is_ascii_whitespace() {
                return Err("Enter a whole token count.".into());
            }
        }
        tokens.push_str(&digits);
        Ok(())
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Result<EditorAction, String> {
        match key.code {
            KeyCode::Esc => return Ok(EditorAction::Back),
            KeyCode::Down | KeyCode::Tab => {
                self.selected = rotate_index(self.selected, FIELDS.len(), true);
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = rotate_index(self.selected, FIELDS.len(), false);
            }
            KeyCode::Enter => return self.selection().map(EditorAction::Save),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.overrides = TerminalModelOverrides::default();
                for tokens in &mut self.tokens {
                    tokens.clear();
                }
            }
            KeyCode::Backspace => {
                if let Some(tokens) = self.active_tokens() {
                    tokens.pop();
                }
            }
            KeyCode::Delete => {
                if let Some(tokens) = self.active_tokens() {
                    tokens.clear();
                } else if let Some(feature) = self.active_feature() {
                    *feature = None;
                } else {
                    self.overrides.reasoning_effort = None;
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                let forward = key.code != KeyCode::Left;
                if let Some(feature) = self.active_feature() {
                    *feature = rotate_choice(&FEATURE_CHOICES, *feature, forward);
                } else if matches!(self.field(), Field::Reasoning) {
                    self.overrides.reasoning_effort =
                        rotate_choice(&REASONING_CHOICES, self.overrides.reasoning_effort, forward);
                }
            }
            KeyCode::Char(character)
                if character.is_ascii_digit()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.paste(character.encode_utf8(&mut [0; 4]))?;
            }
            _ => {}
        }
        Ok(EditorAction::Continue)
    }

    fn selection(&self) -> Result<TerminalModelCardUpdate, String> {
        let parse = |index: usize| {
            let text = &self.tokens[index];
            if text.is_empty() {
                Ok(None)
            } else {
                text.parse::<u64>()
                    .ok()
                    .filter(|limit| *limit != 0)
                    .map(Some)
                    .ok_or_else(|| {
                        format!("{} must be a positive whole number.", FIELDS[index].label())
                    })
            }
        };
        Ok(TerminalModelCardUpdate {
            target: self.card.target.clone(),
            expected_binding: self.card.binding.clone(),
            expected_revision: self.card.revision,
            overrides: TerminalModelOverrides {
                context_window_tokens: parse(0)?,
                input_budget_tokens: parse(1)?,
                max_output_tokens: parse(2)?,
                ..self.overrides.clone()
            },
        })
    }

    fn value(&self, field: Field) -> String {
        if let Some(index) = field.token_index() {
            if !self.tokens[index].is_empty() {
                return self.tokens[index].clone();
            }
            let defaults = self.card.default_limits;
            return match field {
                Field::Context => format!("{} (default)", defaults.context_window_tokens),
                Field::Input => format!("Auto ({} at defaults)", defaults.input_budget_tokens),
                Field::Output => format!("{} (default)", defaults.max_output_tokens),
                _ => unreachable!("token index is defined only for token fields"),
            };
        }
        if let Some((overridden, default)) = self.feature_modes(field) {
            return overridden.map_or_else(
                || format!("Default ({})", feature_label(default)),
                |mode| feature_label(mode).into(),
            );
        }
        self.overrides.reasoning_effort.map_or_else(
            || {
                self.card.default_reasoning_effort.map_or_else(
                    || "Default".into(),
                    |effort| format!("Default ({})", effort.as_str()),
                )
            },
            |effort| effort.as_str().into(),
        )
    }

    pub(super) fn render(
        &self,
        frame: &mut Frame<'_>,
        palette: &TerminalPalette,
        area: Rect,
        applying: bool,
        error: Option<&str>,
    ) {
        frame.render_widget(Clear, area);
        let inner = area.inner(ratatui::layout::Margin::new(2, 1));
        let controls = match inner.width {
            100..=u16::MAX => 2,
            58..=99 => 3,
            _ => 4,
        };
        let notice = if applying {
            Some("Saving model settings…")
        } else {
            error
        };
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(u16::from(notice.is_some()) * 2),
            Constraint::Length(controls),
        ])
        .split(inner);
        let model = truncate_width_with_ellipsis(
            &sanitize_approval_field(&format!(
                "{} · {}",
                self.card.route.model, self.card.route.provider_profile
            )),
            usize::from(rows[0].width),
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Configure model",
                    ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
                )),
                Line::from(model),
            ]),
            rows[0],
        );
        let visible = usize::from(rows[1].height).max(1);
        let start = self
            .selected
            .saturating_sub(visible / 2)
            .min(FIELDS.len().saturating_sub(visible));
        let lines = FIELDS
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, field)| {
                let (marker, style) = if index == self.selected {
                    ("› ", palette.user_style())
                } else {
                    ("  ", palette.meta_style())
                };
                Line::from(Span::styled(
                    format!("{marker}{:<19} {}", field.label(), self.value(*field)),
                    ratatui_style(style),
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), rows[1]);
        if let Some(notice) = notice {
            frame.render_widget(
                Paragraph::new(sanitize_approval_field(notice)).wrap(Wrap { trim: false }),
                rows[2],
            );
        }
        render_browser_controls(
            frame,
            palette,
            rows[3],
            &[
                ("Tab/↑↓", "Field"),
                ("←/→", "Change"),
                ("Del", "Inherit"),
                ("Ctrl+R", "Reset"),
                ("Enter", "Save and use"),
                ("Esc", "Back"),
            ],
        );
    }
}

const fn feature_label(mode: ModelFeatureMode) -> &'static str {
    match mode {
        ModelFeatureMode::Auto => "Auto",
        ModelFeatureMode::On => "On",
        ModelFeatureMode::Off => "Off",
    }
}

fn rotate_index(index: usize, count: usize, forward: bool) -> usize {
    (index + if forward { 1 } else { count - 1 }) % count
}

fn rotate_choice<T: Copy + PartialEq>(values: &[T], current: T, forward: bool) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    values[rotate_index(index, values.len(), forward)]
}
