use super::*;
use colossus_contracts::{ProviderModelInfo, TerminalModelProfile};

#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq, Eq)]
enum Row {
    RoleDefault,
    Configured(usize),
    CurrentConversation,
    Provider(String),
    Model { provider: String, id: String },
}

#[derive(Default)]
struct ProviderGroup {
    expanded: bool,
    loading: bool,
    cards: BTreeMap<String, ProviderModelInfo>,
    loaded: bool,
    error: Option<String>,
}

pub(super) struct ModelCatalogState {
    generation: u64,
    overview: Option<InteractiveModelCatalog>,
    groups: BTreeMap<String, ProviderGroup>,
    rows: Vec<Row>,
    query: String,
    selected: usize,
    editor: Option<ModelEditor>,
    error: Option<String>,
}

struct ModelEditor {
    provider: String,
    model: String,
    context: String,
    output: String,
    editing_output: bool,
}

impl ModelEditor {
    fn new(provider: String, card: &ProviderModelInfo) -> Self {
        let context = card.context_window_tokens.unwrap_or(32_768);
        let output = card.max_output_tokens.unwrap_or(4_096).min(context / 2);
        Self {
            provider,
            model: card.id.clone(),
            context: context.to_string(),
            output: output.to_string(),
            editing_output: false,
        }
    }

    fn active_field(&mut self) -> &mut String {
        if self.editing_output {
            &mut self.output
        } else {
            &mut self.context
        }
    }

    fn selection(&self) -> Result<ModelCatalogSelection, String> {
        let context_window_tokens = self
            .context
            .parse()
            .map_err(|_| "Enter a whole number for the context window.")?;
        let max_output_tokens = self
            .output
            .parse()
            .map_err(|_| "Enter a whole number for max output tokens.")?;
        Ok(ModelCatalogSelection::Discovered(TerminalModelProfile {
            provider_profile: self.provider.clone(),
            model: self.model.clone(),
            context_window_tokens,
            max_output_tokens,
        }))
    }
}

impl ModelCatalogState {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            overview: None,
            groups: BTreeMap::new(),
            rows: Vec::new(),
            query: String::new(),
            selected: 0,
            editor: None,
            error: None,
        }
    }

    fn rebuild_rows(&mut self) {
        let selected = self.rows.get(self.selected).cloned();
        self.rows.clear();
        let Some(overview) = self.overview.as_ref() else {
            return;
        };
        let query = self.query.trim().to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        if matches("Configured role default") {
            self.rows.push(Row::RoleDefault);
        }
        self.rows.extend(
            overview
                .configured
                .iter()
                .enumerate()
                .filter_map(|(index, model)| {
                    (matches(&model.model_profile)
                        || matches(&model.model)
                        || matches(&model.provider_profile))
                    .then_some(Row::Configured(index))
                }),
        );
        if let Some(profile) = &overview.selected_terminal_model
            && (matches(&profile.model) || matches(&profile.provider_profile))
        {
            self.rows.push(Row::CurrentConversation);
        }
        for (profile, group) in &self.groups {
            // Keep groups reachable while filtering so an unloaded provider can be searched.
            self.rows.push(Row::Provider(profile.clone()));
            if group.expanded {
                self.rows.extend(
                    group
                        .cards
                        .values()
                        .filter(|card| {
                            matches(&card.id)
                                || card.display_name.as_deref().is_some_and(matches)
                                || (!query.is_empty() && matches(profile))
                        })
                        .map(|card| Row::Model {
                            provider: profile.clone(),
                            id: card.id.clone(),
                        }),
                );
            }
        }
        self.selected = selected
            .and_then(|selected| self.rows.iter().position(|row| row == &selected))
            .unwrap_or(0);
    }

    pub(super) fn paste(&mut self, text: &str) {
        if let Some(editor) = self.editor.as_mut() {
            let field = editor.active_field();
            field.extend(
                text.chars()
                    .filter(char::is_ascii_digit)
                    .take(10_usize.saturating_sub(field.len())),
            );
            self.error = None;
        } else {
            for character in text.chars().filter(|character| !character.is_control()) {
                if self.query.len() + character.len_utf8() > 128 {
                    break;
                }
                self.query.push(character);
            }
            self.rebuild_rows();
        }
    }

    fn update(&mut self, update: ModelCatalogUpdate) {
        match update {
            ModelCatalogUpdate::Overview(Ok(overview)) => {
                self.groups = overview
                    .providers
                    .iter()
                    .map(|name| (name.clone(), ProviderGroup::default()))
                    .collect();
                self.overview = Some(overview);
                self.rebuild_rows();
            }
            ModelCatalogUpdate::Overview(Err(error)) => self.error = Some(error),
            ModelCatalogUpdate::Provider { profile, result } => {
                let Some(group) = self.groups.get_mut(&profile) else {
                    return;
                };
                group.loading = false;
                match result {
                    Ok(cards) => {
                        group.loaded = true;
                        group.error = None;
                        group.cards = cards
                            .into_iter()
                            .map(|card| (card.id.clone(), card))
                            .collect();
                    }
                    Err(error) => group.error = Some(error),
                }
                self.rebuild_rows();
            }
            ModelCatalogUpdate::Applied(Err(error)) => {
                self.error = Some(error);
            }
            ModelCatalogUpdate::Applied(Ok(_)) => {}
        }
    }
}

pub(super) fn open(
    state: &mut TuiState,
    host: Arc<dyn InteractiveHost>,
    events: mpsc::Sender<HostEvent>,
) {
    state.model_catalog_generation = state.model_catalog_generation.wrapping_add(1);
    let generation = state.model_catalog_generation;
    state.overlay = Some(Overlay::ModelCatalog(Box::new(ModelCatalogState::new(
        generation,
    ))));
    let session_id = state.session_id.clone();
    tokio::spawn(async move {
        let overview = host.model_catalog_overview(&session_id).await;
        let _ = events
            .send(HostEvent::ModelCatalogUpdated {
                session_id,
                generation,
                update: ModelCatalogUpdate::Overview(overview),
            })
            .await;
    });
}

pub(super) fn updated(
    state: &mut TuiState,
    session_id: String,
    generation: u64,
    update: ModelCatalogUpdate,
) {
    if session_id != state.session_id {
        return;
    }
    if generation == state.model_catalog_generation
        && matches!(update, ModelCatalogUpdate::Applied(_))
    {
        state.model_catalog_applying = false;
    }
    if let ModelCatalogUpdate::Applied(Ok(applied)) = update {
        if applied.revision >= state.footer.model_revision {
            state.footer.model_revision = applied.revision;
            state.footer.role = applied.role;
            state.footer.route = applied.route;
            state.footer.context = state
                .footer
                .context
                .map(|(used, _)| (used, applied.input_budget_tokens));
        }
        if generation == state.model_catalog_generation
            && matches!(state.overlay, Some(Overlay::ModelCatalog(_)))
        {
            state.overlay = None;
        }
    } else if generation == state.model_catalog_generation
        && let Some(Overlay::ModelCatalog(browser)) = state.overlay.as_mut()
    {
        browser.update(update);
    } else if generation == state.model_catalog_generation
        && let ModelCatalogUpdate::Applied(Err(error)) = update
    {
        let error = sanitize_approval_field(&error);
        state.append_entry(error_entry(&format!("Model selection failed: {error}")));
        if !state.queue.is_empty() {
            state.queue_paused = true;
            if !state.is_busy() && state.overlay.is_none() {
                state.overlay = Some(Overlay::QueuePaused);
            }
        }
    }
}

pub(super) fn key(
    state: &mut TuiState,
    key: KeyEvent,
    host: Arc<dyn InteractiveHost>,
    events: mpsc::Sender<HostEvent>,
) {
    let Some(Overlay::ModelCatalog(browser)) = state.overlay.as_mut() else {
        return;
    };
    if state.model_catalog_applying {
        return;
    }
    let mut selection = None;
    if let Some(editor) = browser.editor.as_mut() {
        browser.error = None;
        match key.code {
            KeyCode::Esc => browser.editor = None,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                editor.editing_output = !editor.editing_output;
            }
            KeyCode::Enter => match editor.selection() {
                Ok(choice) => selection = Some(choice),
                Err(error) => browser.error = Some(error),
            },
            KeyCode::Backspace => {
                editor.active_field().pop();
            }
            KeyCode::Delete => editor.active_field().clear(),
            KeyCode::Char(character) if character.is_ascii_digit() => {
                let field = editor.active_field();
                if field.len() < 10 {
                    field.push(character);
                }
            }
            _ => {}
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                state.overlay = None;
                return;
            }
            KeyCode::Up | KeyCode::BackTab => {
                browser.selected = browser.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Tab => {
                browser.selected = (browser.selected + 1).min(browser.rows.len().saturating_sub(1));
            }
            KeyCode::Home => browser.selected = 0,
            KeyCode::End => browser.selected = browser.rows.len().saturating_sub(1),
            KeyCode::Backspace => {
                browser.query.pop();
                browser.rebuild_rows();
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                let profile = match browser.rows.get(browser.selected) {
                    Some(
                        Row::Provider(profile)
                        | Row::Model {
                            provider: profile, ..
                        },
                    ) => profile,
                    _ => return,
                };
                load_provider(
                    browser,
                    &state.session_id,
                    profile.clone(),
                    true,
                    host,
                    events,
                );
                return;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                browser.paste(character.encode_utf8(&mut [0; 4]));
            }
            KeyCode::Enter => match browser.rows.get(browser.selected).cloned() {
                Some(Row::RoleDefault) => selection = Some(ModelCatalogSelection::RoleDefault),
                Some(Row::Configured(index)) => {
                    let model = &browser
                        .overview
                        .as_ref()
                        .expect("configured model rows")
                        .configured[index];
                    selection = Some(ModelCatalogSelection::Configured(
                        model.model_profile.clone(),
                    ));
                }
                Some(Row::CurrentConversation) => {
                    let profile = browser
                        .overview
                        .as_ref()
                        .and_then(|overview| overview.selected_terminal_model.as_ref())
                        .expect("current conversation model row");
                    browser.editor = Some(ModelEditor {
                        provider: profile.provider_profile.clone(),
                        model: profile.model.clone(),
                        context: profile.context_window_tokens.to_string(),
                        output: profile.max_output_tokens.to_string(),
                        editing_output: false,
                    });
                }
                Some(Row::Provider(profile)) => {
                    let group = browser.groups.get_mut(&profile).expect("provider row");
                    group.expanded = !group.expanded;
                    if group.expanded && !group.loaded {
                        load_provider(browser, &state.session_id, profile, false, host, events);
                        return;
                    }
                    browser.rebuild_rows();
                }
                Some(Row::Model { provider, id }) => {
                    if let Some(card) = browser.groups[&provider].cards.get(&id) {
                        browser.editor = Some(ModelEditor::new(provider, card));
                        browser.error = None;
                    }
                }
                None => {}
            },
            _ => {}
        }
    }
    if let Some(selection) = selection {
        state.model_catalog_applying = true;
        let generation = browser.generation;
        let session_id = state.session_id.clone();
        tokio::spawn(async move {
            let result = host.select_catalog_model(&session_id, selection).await;
            let _ = events
                .send(HostEvent::ModelCatalogUpdated {
                    session_id,
                    generation,
                    update: ModelCatalogUpdate::Applied(result),
                })
                .await;
        });
    }
}

fn load_provider(
    browser: &mut ModelCatalogState,
    session_id: &str,
    profile: String,
    refresh: bool,
    host: Arc<dyn InteractiveHost>,
    events: mpsc::Sender<HostEvent>,
) {
    let group = browser.groups.get_mut(&profile).expect("selected provider");
    if group.loading {
        return;
    }
    group.expanded = true;
    group.loading = true;
    group.error = None;
    browser.rebuild_rows();
    let generation = browser.generation;
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        let result = host.provider_model_catalog(&profile, refresh).await;
        let _ = events
            .send(HostEvent::ModelCatalogUpdated {
                session_id,
                generation,
                update: ModelCatalogUpdate::Provider { profile, result },
            })
            .await;
    });
}

pub(super) fn render(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let Some(Overlay::ModelCatalog(browser)) = state.overlay.as_ref() else {
        return;
    };
    let palette = TerminalPalette::for_preferences(&state.preferences);
    if let Some(editor) = &browser.editor {
        render_editor(
            frame,
            &palette,
            area,
            editor,
            state.model_catalog_applying,
            browser.error.as_deref(),
        );
        return;
    }
    let count = browser.overview.as_ref().map_or_else(
        || "Loading configured models".to_owned(),
        |overview| format!("{} configured", overview.configured.len()),
    );
    let areas = render_browser_shell(
        frame,
        &palette,
        area,
        60,
        BrowserHeader {
            title: "Models",
            count,
            search_hint: "Type to search",
            query: &browser.query,
            search_active: !browser.query.is_empty() && !state.model_catalog_applying,
        },
    );
    let (list, preview) = if areas.preview.is_none() && areas.list.height >= 5 {
        let split = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).split(areas.list);
        (split[0], Some(split[1]))
    } else {
        (areas.list, areas.preview)
    };
    let height = usize::from(list.height).max(1);
    let start = browser
        .selected
        .saturating_sub(height / 2)
        .min(browser.rows.len().saturating_sub(height));
    let mut rows = Vec::new();
    for (offset, row) in browser.rows.iter().skip(start).take(height).enumerate() {
        let selected = start + offset == browser.selected;
        let label = sanitize_approval_field(&row_label(browser, row));
        rows.push(Line::from(Span::styled(
            format!("{} {label}", if selected { "›" } else { " " }),
            ratatui_style(if selected {
                palette.user_style()
            } else {
                palette.meta_style()
            }),
        )));
    }
    if rows.is_empty() {
        rows.push(Line::from(if browser.overview.is_some() {
            "No matching models"
        } else if browser.error.is_some() {
            "Could not load configured models"
        } else {
            "Loading configured models…"
        }));
    }
    frame.render_widget(Paragraph::new(rows), list);
    if let Some(preview) = preview {
        let details = if state.model_catalog_applying {
            "Applying model…".to_owned()
        } else if let Some(error) = &browser.error {
            sanitize_approval_field(error)
        } else {
            browser
                .rows
                .get(browser.selected)
                .map_or_else(String::new, |row| row_details(browser, row))
        };
        frame.render_widget(Paragraph::new(details).wrap(Wrap { trim: false }), preview);
    }
    render_browser_controls(
        frame,
        &palette,
        areas.controls,
        &[
            ("↑/↓", "Browse"),
            ("Enter", "Open or select"),
            ("Ctrl+R", "Refresh provider"),
            ("Esc", "Close"),
        ],
    );
}

fn render_editor(
    frame: &mut Frame<'_>,
    palette: &TerminalPalette,
    area: Rect,
    editor: &ModelEditor,
    applying: bool,
    error: Option<&str>,
) {
    frame.render_widget(Clear, area);
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    let rows = Layout::vertical([Constraint::Min(6), Constraint::Length(3)]).split(inner);
    let model = truncate_width_with_ellipsis(
        &sanitize_approval_field(&format!("{} · {}", editor.model, editor.provider)),
        usize::from(rows[0].width),
    );
    let field = |name, value, editing| {
        let (marker, style) = if editing {
            ("› ", palette.user_style())
        } else {
            ("  ", palette.meta_style())
        };
        Line::from(Span::styled(
            format!("{marker}{name}  {value}"),
            ratatui_style(style),
        ))
    };
    let mut details = vec![
        Line::from(Span::styled(
            "Configure model",
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        )),
        Line::from(model),
        Line::default(),
        field("Context window", &editor.context, !editor.editing_output),
        field("Max output    ", &editor.output, editor.editing_output),
    ];
    if applying {
        details.push(Line::from("Applying model…"));
    } else if let Some(error) = error {
        details.push(Line::from(sanitize_approval_field(error)));
    }
    frame.render_widget(Paragraph::new(details).wrap(Wrap { trim: false }), rows[0]);
    render_browser_controls(
        frame,
        palette,
        rows[1],
        &[
            ("Tab", "Next field"),
            ("Del", "Clear field"),
            ("Enter", "Use in conversation"),
            ("Esc", "Back"),
        ],
    );
}

fn row_label(browser: &ModelCatalogState, row: &Row) -> String {
    let overview = browser
        .overview
        .as_ref()
        .expect("catalog rows require an overview");
    match row {
        Row::RoleDefault
            if overview.selected_profile.is_none()
                && overview.selected_terminal_model.is_none() =>
        {
            "Configured role default · selected".into()
        }
        Row::RoleDefault => "Configured role default".into(),
        Row::Configured(index) => {
            let route = &overview.configured[*index];
            let current =
                overview.selected_profile.as_deref() == Some(route.model_profile.as_str());
            format!(
                "{} · {}{}",
                route.model_profile,
                route.model,
                if current { " · selected" } else { "" }
            )
        }
        Row::CurrentConversation => {
            format!(
                "Current · {}",
                overview
                    .selected_terminal_model
                    .as_ref()
                    .expect("conversation model")
                    .model
            )
        }
        Row::Provider(profile) => {
            let group = &browser.groups[profile];
            let marker = if group.expanded { "▾" } else { "▸" };
            let status = if group.loading {
                " · loading".into()
            } else if group.error.is_some() {
                " · unavailable".into()
            } else if group.loaded {
                format!(" · {} models", group.cards.len())
            } else {
                String::new()
            };
            format!("{marker} {profile}{status}")
        }
        Row::Model { id, .. } => format!("    {id}"),
    }
}

fn row_details(browser: &ModelCatalogState, row: &Row) -> String {
    let overview = browser
        .overview
        .as_ref()
        .expect("catalog rows require an overview");
    match row {
        Row::RoleDefault => "Use the configured role for the next turn.".into(),
        Row::Configured(index) => {
            let route = &overview.configured[*index];
            format!(
                "{}\n{}\n\nContext {}\nOutput budget {}",
                sanitize_approval_field(&route.model),
                sanitize_approval_field(&route.provider_profile),
                route.limits.context_window_tokens,
                route.limits.max_output_tokens
            )
        }
        Row::CurrentConversation => {
            "Edit the current conversation’s context and output limits.".into()
        }
        Row::Provider(profile) => browser.groups[profile].error.as_deref().map_or_else(
            || "Enter expands the provider’s model catalog.".into(),
            sanitize_approval_field,
        ),
        Row::Model { provider, id } => {
            let card = &browser.groups[provider].cards[id];
            let label =
                |tokens: Option<u64>| tokens.map_or_else(|| "unknown".into(), |n| n.to_string());
            format!(
                "{}\n{}\n\nReported context {}\nReported max output {}",
                sanitize_approval_field(id),
                sanitize_approval_field(card.display_name.as_deref().unwrap_or(provider)),
                label(card.context_window_tokens),
                label(card.max_output_tokens)
            )
        }
    }
}
