use super::*;
use colossus_contracts::{
    ProviderModelInfo, TerminalModelCard, TerminalModelProfile, TerminalModelTarget,
};

mod editor;
use editor::{EditorAction, ModelEditor};

#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq, Eq)]
enum Row {
    RoleDefault,
    Configured(usize),
    Saved(usize),
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

enum CatalogAction {
    Keep,
    Close,
    Edit(TerminalModelTarget),
    Select(ModelCatalogSelection),
    Load { profile: String, refresh: bool },
}

pub(super) struct ModelCatalogState {
    generation: u64,
    overview: Option<InteractiveModelCatalog>,
    groups: BTreeMap<String, ProviderGroup>,
    rows: Vec<Row>,
    query: String,
    selected: usize,
    editor: Option<ModelEditor>,
    next_editor_request: u64,
    loading_editor: Option<(u64, TerminalModelTarget)>,
    error: Option<String>,
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
            next_editor_request: 0,
            loading_editor: None,
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
        self.rows.extend(
            overview
                .saved
                .iter()
                .enumerate()
                .filter_map(|(index, card)| {
                    (matches!(card.target, TerminalModelTarget::Discovered { .. })
                        && (matches(&card.route.model)
                            || matches(&card.route.provider_profile)
                            || card
                                .advertised
                                .as_ref()
                                .and_then(|reported| reported.display_name.as_deref())
                                .is_some_and(matches)))
                    .then_some(Row::Saved(index))
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
            self.error = editor.paste(text).err();
        } else if self.loading_editor.is_none() {
            for character in text.chars().filter(|character| !character.is_control()) {
                if self.query.len() + character.len_utf8() > 128 {
                    break;
                }
                self.query.push(character);
            }
            self.rebuild_rows();
        }
    }

    fn saved_card(&self, target: &TerminalModelTarget) -> Option<&TerminalModelCard> {
        self.overview
            .as_ref()?
            .saved
            .iter()
            .find(|card| &card.target == target)
    }

    fn target(&self, row: &Row) -> Option<TerminalModelTarget> {
        let overview = self.overview.as_ref()?;
        match row {
            Row::Configured(index) => Some(TerminalModelTarget::Configured {
                profile: overview.configured[*index].model_profile.clone(),
            }),
            Row::Saved(index) => Some(overview.saved[*index].target.clone()),
            Row::CurrentConversation => {
                let current = overview.selected_terminal_model.as_ref()?;
                Some(TerminalModelTarget::Discovered {
                    provider_profile: current.provider_profile.clone(),
                    model: current.model.clone(),
                })
            }
            Row::Model { provider, id } => Some(TerminalModelTarget::Discovered {
                provider_profile: provider.clone(),
                model: id.clone(),
            }),
            Row::RoleDefault | Row::Provider(_) => None,
        }
    }

    fn browse_key(&mut self, key: KeyEvent) -> CatalogAction {
        match key.code {
            KeyCode::Esc => return CatalogAction::Close,
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
            KeyCode::Backspace => {
                self.query.pop();
                self.rebuild_rows();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self
                    .rows
                    .get(self.selected)
                    .and_then(|row| self.target(row))
                    .map_or(CatalogAction::Keep, CatalogAction::Edit);
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let profile = self.rows.get(self.selected).and_then(|row| match row {
                    Row::Provider(profile) => Some(profile.clone()),
                    row => match self.target(row) {
                        Some(TerminalModelTarget::Discovered {
                            provider_profile, ..
                        }) => Some(provider_profile),
                        _ => None,
                    },
                });
                if let Some(profile) = profile {
                    return CatalogAction::Load {
                        profile,
                        refresh: true,
                    };
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.paste(character.encode_utf8(&mut [0; 4]));
            }
            KeyCode::Enter => return self.activate_row(),
            _ => {}
        }
        CatalogAction::Keep
    }

    fn activate_row(&mut self) -> CatalogAction {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return CatalogAction::Keep;
        };
        match row {
            Row::RoleDefault => CatalogAction::Select(ModelCatalogSelection::RoleDefault),
            Row::Provider(profile) => {
                let group = self.groups.get_mut(&profile).expect("provider row");
                group.expanded = !group.expanded;
                if group.expanded && !group.loaded {
                    CatalogAction::Load {
                        profile,
                        refresh: false,
                    }
                } else {
                    self.rebuild_rows();
                    CatalogAction::Keep
                }
            }
            row => {
                let Some(target) = self.target(&row) else {
                    return CatalogAction::Keep;
                };
                if let Some(card) = self.saved_card(&target) {
                    CatalogAction::Select(ModelCatalogSelection::Saved(card.saved_ref()))
                } else if let TerminalModelTarget::Configured { profile } = target {
                    CatalogAction::Select(ModelCatalogSelection::Configured(profile))
                } else if self
                    .overview
                    .as_ref()
                    .is_some_and(|view| view.supports_model_cards)
                {
                    CatalogAction::Edit(target)
                } else {
                    self.legacy_discovery(&row)
                        .map_or(CatalogAction::Keep, |profile| {
                            CatalogAction::Select(ModelCatalogSelection::Discovered(profile))
                        })
                }
            }
        }
    }

    fn legacy_discovery(&self, row: &Row) -> Option<TerminalModelProfile> {
        match row {
            Row::CurrentConversation => self.overview.as_ref()?.selected_terminal_model.clone(),
            Row::Model { provider, id } => {
                let card = self.groups.get(provider)?.cards.get(id)?;
                let context = card
                    .context_window_tokens
                    .filter(|limit| *limit >= 1_024)
                    .unwrap_or(32_768);
                let available = context - context.div_ceil(10).max(512) - 1;
                Some(TerminalModelProfile {
                    provider_profile: provider.clone(),
                    model: id.clone(),
                    context_window_tokens: context,
                    max_output_tokens: card
                        .max_output_tokens
                        .unwrap_or(4_096)
                        .min(context / 2)
                        .min(available),
                })
            }
            _ => None,
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
            ModelCatalogUpdate::Card { request, result } => {
                if self
                    .loading_editor
                    .as_ref()
                    .is_none_or(|(current, _)| *current != request)
                {
                    return;
                }
                let (_, target) = self
                    .loading_editor
                    .take()
                    .expect("outstanding model editor");
                match result {
                    Ok(mut card) if card.target == target => {
                        if card.revision == 0
                            && let Some(current) = self
                                .overview
                                .as_ref()
                                .and_then(|overview| overview.selected_terminal_model.as_ref())
                            && current.provider_profile == card.route.provider_profile
                            && current.model == card.route.model
                        {
                            card.overrides.context_window_tokens =
                                Some(current.context_window_tokens);
                            card.overrides.max_output_tokens = Some(current.max_output_tokens);
                        }
                        self.editor = Some(ModelEditor::new(*card));
                    }
                    Ok(_) => {
                        self.error = Some("The host returned another model's settings.".into())
                    }
                    Err(error) => self.error = Some(error),
                }
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
        state.model_catalog_control = None;
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
    if let Some(control) = &state.model_catalog_control {
        if key.code == KeyCode::Esc {
            control.cancel();
        }
        return;
    }
    browser.error = None;
    let action = if let Some(editor) = browser.editor.as_mut() {
        match editor.key(key) {
            Ok(EditorAction::Back) => {
                browser.editor = None;
                CatalogAction::Keep
            }
            Ok(EditorAction::Save(update)) => {
                CatalogAction::Select(ModelCatalogSelection::SaveCard(update))
            }
            Ok(EditorAction::Continue) => CatalogAction::Keep,
            Err(error) => {
                browser.error = Some(error);
                CatalogAction::Keep
            }
        }
    } else if browser.loading_editor.is_some() {
        if key.code == KeyCode::Esc {
            browser.loading_editor = None;
        }
        CatalogAction::Keep
    } else {
        browser.browse_key(key)
    };
    match action {
        CatalogAction::Close => state.overlay = None,
        CatalogAction::Edit(target) => {
            load_editor(browser, &state.session_id, target, host, events)
        }
        CatalogAction::Select(selection) => {
            let control = RunControl::default();
            state.model_catalog_control = Some(control.clone());
            let generation = browser.generation;
            let session_id = state.session_id.clone();
            tokio::spawn(async move {
                let result = host
                    .select_catalog_model(&session_id, selection, events.clone(), control)
                    .await;
                let _ = events
                    .send(HostEvent::ModelCatalogUpdated {
                        session_id,
                        generation,
                        update: ModelCatalogUpdate::Applied(result),
                    })
                    .await;
            });
        }
        CatalogAction::Load { profile, refresh } => {
            load_provider(browser, &state.session_id, profile, refresh, host, events);
        }
        CatalogAction::Keep => {}
    }
}

fn load_editor(
    browser: &mut ModelCatalogState,
    session_id: &str,
    target: TerminalModelTarget,
    host: Arc<dyn InteractiveHost>,
    events: mpsc::Sender<HostEvent>,
) {
    browser.next_editor_request = browser.next_editor_request.wrapping_add(1);
    let request = browser.next_editor_request;
    browser.loading_editor = Some((request, target.clone()));
    let generation = browser.generation;
    let session_id = session_id.to_owned();
    tokio::spawn(async move {
        let result = host.terminal_model_card(target).await.map(Box::new);
        let _ = events
            .send(HostEvent::ModelCatalogUpdated {
                session_id,
                generation,
                update: ModelCatalogUpdate::Card { request, result },
            })
            .await;
    });
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
        editor.render(
            frame,
            &palette,
            area,
            state.model_catalog_control.is_some(),
            browser.error.as_deref(),
        );
        return;
    }
    let count = browser.overview.as_ref().map_or_else(
        || "Loading configured models".to_owned(),
        |overview| {
            if overview.saved.is_empty() {
                format!("{} configured", overview.configured.len())
            } else {
                format!(
                    "{} configured · {} saved",
                    overview.configured.len(),
                    overview.saved.len()
                )
            }
        },
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
            search_active: !browser.query.is_empty()
                && browser.loading_editor.is_none()
                && state.model_catalog_control.is_none(),
        },
    );
    let status_visible = browser.error.is_some()
        || browser.loading_editor.is_some()
        || state.model_catalog_control.is_some();
    let (list, preview) = if areas.preview.is_none()
        && (areas.list.height >= 5 || (status_visible && areas.list.height >= 2))
    {
        let detail_rows = if areas.list.height >= 5 { 3 } else { 1 };
        let split = Layout::vertical([Constraint::Min(1), Constraint::Length(detail_rows)])
            .split(areas.list);
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
        let details = if state.model_catalog_control.is_some() {
            "Applying model…".to_owned()
        } else if browser.loading_editor.is_some() {
            "Loading model settings…".to_owned()
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
            ("Enter", "Open or use"),
            ("Ctrl+E", "Edit"),
            ("Ctrl+R", "Refresh"),
            ("Esc", "Close"),
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
                && overview.selected_terminal_model.is_none()
                && overview.selected_saved_model.is_none() =>
        {
            "Configured role default · selected".into()
        }
        Row::RoleDefault => "Configured role default".into(),
        Row::Configured(index) => {
            let route = &overview.configured[*index];
            let target = TerminalModelTarget::Configured {
                profile: route.model_profile.clone(),
            };
            let saved = browser.saved_card(&target);
            let current = overview.selected_profile.as_deref()
                == Some(route.model_profile.as_str())
                || saved.is_some_and(|card| selected_saved_card(overview, card));
            format!(
                "{} · {}{}{}",
                route.model_profile,
                route.model,
                if saved.is_some() {
                    " · saved settings"
                } else {
                    ""
                },
                if current { " · selected" } else { "" }
            )
        }
        Row::Saved(index) => {
            let card = &overview.saved[*index];
            format!(
                "{} · {} · saved{}",
                card.route.model,
                card.route.provider_profile,
                if selected_saved_card(overview, card) {
                    " · selected"
                } else {
                    ""
                }
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
            let target = TerminalModelTarget::Configured {
                profile: route.model_profile.clone(),
            };
            route_details(
                browser
                    .saved_card(&target)
                    .map_or(route, |card| &card.route),
            )
        }
        Row::Saved(index) => route_details(&overview.saved[*index].route),
        Row::CurrentConversation => "Edit and save the current model’s settings.".into(),
        Row::Provider(profile) => browser.groups[profile].error.as_deref().map_or_else(
            || "Enter opens or collapses this provider’s catalog.".into(),
            sanitize_approval_field,
        ),
        Row::Model { provider, id } => {
            let target = TerminalModelTarget::Discovered {
                provider_profile: provider.clone(),
                model: id.clone(),
            };
            if let Some(card) = browser.saved_card(&target) {
                return route_details(&card.route);
            }
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

fn selected_saved_card(overview: &InteractiveModelCatalog, card: &TerminalModelCard) -> bool {
    overview
        .selected_saved_model
        .as_ref()
        .is_some_and(|selected| selected.target == card.target && selected.binding == card.binding)
}

fn route_details(route: &colossus_contracts::ModelRoute) -> String {
    format!(
        "{}\n{}\n\nContext {}\nInput budget {}\nOutput budget {}",
        sanitize_approval_field(&route.model),
        sanitize_approval_field(&route.provider_profile),
        route.limits.context_window_tokens,
        route.limits.input_budget_tokens,
        route.limits.max_output_tokens,
    )
}
