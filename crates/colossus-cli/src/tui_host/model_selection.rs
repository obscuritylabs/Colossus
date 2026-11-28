use super::*;

#[derive(Clone)]
pub(super) enum TerminalSelection {
    Discovered(TerminalModelProfile),
    Saved(SavedTerminalModelRef),
}

impl TerminalSelection {
    fn target(&self) -> TerminalModelTarget {
        match self {
            Self::Discovered(profile) => TerminalModelTarget::Discovered {
                provider_profile: profile.provider_profile.clone(),
                model: profile.model.clone(),
            },
            Self::Saved(model) => model.target.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct ModelSelection {
    pub(super) options: WorkflowAgentOptions,
    pub(super) terminal: Option<(TerminalSelection, ProviderRoute)>,
    pub(super) revision: u64,
}

impl ModelSelection {
    /// Omit defaults so an ordinary turn keeps the legacy authenticated wire shape.
    pub(super) fn request_options(&self) -> Option<WorkflowAgentOptions> {
        (self.options != WorkflowAgentOptions::default()).then(|| self.options.clone())
    }

    pub(super) fn terminal_choice(&self) -> Option<&TerminalSelection> {
        self.terminal.as_ref().map(|(selection, _)| selection)
    }

    pub(super) fn transient_model(&self) -> Option<TerminalModelProfile> {
        match self.terminal_choice() {
            Some(TerminalSelection::Discovered(profile)) => Some(profile.clone()),
            _ => None,
        }
    }

    pub(super) fn saved_model(&self) -> Option<SavedTerminalModelRef> {
        match self.terminal_choice() {
            Some(TerminalSelection::Saved(target)) => Some(target.clone()),
            _ => None,
        }
    }

    fn configured_profile(&self) -> Option<&str> {
        match self.terminal_choice() {
            Some(TerminalSelection::Saved(SavedTerminalModelRef {
                target: TerminalModelTarget::Configured { profile },
                ..
            })) => Some(profile),
            _ => self.options.model_profile.as_deref(),
        }
    }

    pub(super) fn catalog(
        &self,
        configured: Vec<ProviderRoute>,
        providers: Vec<ProviderReadiness>,
        saved: Vec<TerminalModelCard>,
        supports_model_cards: bool,
    ) -> InteractiveModelCatalog {
        InteractiveModelCatalog {
            configured,
            providers: providers
                .into_iter()
                .filter(|provider| provider.provider != "echo")
                .map(|provider| provider.profile)
                .collect(),
            saved,
            supports_model_cards,
            selected_profile: self.configured_profile().map(str::to_owned),
            selected_terminal_model: self.transient_model(),
            selected_saved_model: self.saved_model(),
        }
    }

    pub(super) fn catalog_choice(
        &self,
        selection: ModelCatalogSelection,
    ) -> Result<(WorkflowAgentOptions, Option<TerminalSelection>), String> {
        let (configured, terminal) = match selection {
            ModelCatalogSelection::RoleDefault => (None, None),
            ModelCatalogSelection::Configured(profile) => (Some(profile), None),
            ModelCatalogSelection::Discovered(profile) => {
                (None, Some(TerminalSelection::Discovered(profile)))
            }
            ModelCatalogSelection::Saved(model) => (None, Some(TerminalSelection::Saved(model))),
            ModelCatalogSelection::SaveCard(_) => {
                return Err("Save the model card before selecting it.".into());
            }
        };
        let current = model_target(
            self.options.model_profile.as_deref(),
            self.terminal_choice(),
        );
        let target = model_target(configured.as_deref(), terminal.as_ref());
        let same_binding = match (self.terminal_choice(), terminal.as_ref()) {
            (Some(TerminalSelection::Saved(current)), Some(TerminalSelection::Saved(next))) => {
                current == next
            }
            _ => true,
        };
        Ok((
            WorkflowAgentOptions {
                model_profile: configured,
                reasoning_effort: self
                    .options
                    .reasoning_effort
                    .filter(|_| current == target && same_binding),
            },
            terminal,
        ))
    }

    pub(super) fn effort_choice(
        &self,
        effort: Option<ReasoningEffort>,
    ) -> (WorkflowAgentOptions, Option<TerminalSelection>) {
        let mut options = self.options.clone();
        options.reasoning_effort = effort;
        (options, self.terminal_choice().cloned())
    }
}

fn model_target(
    configured: Option<&str>,
    terminal: Option<&TerminalSelection>,
) -> Option<TerminalModelTarget> {
    if let Some(selection) = terminal {
        return Some(selection.target());
    }
    configured.map(|profile| TerminalModelTarget::Configured {
        profile: profile.into(),
    })
}

#[derive(Default)]
struct ModelSelectionState {
    revision: u64,
    next_intent: u64,
    entries: BTreeMap<String, ModelSelection>,
    pending: BTreeMap<String, u64>,
}

#[derive(Default)]
pub(super) struct ModelSelections(Mutex<ModelSelectionState>);

impl ModelSelections {
    pub(super) fn get(&self, session: &str) -> Result<ModelSelection, String> {
        let state = self
            .0
            .lock()
            .map_err(|_| "model selection lock is poisoned")?;
        Ok(state
            .entries
            .get(session)
            .cloned()
            .unwrap_or(ModelSelection {
                revision: state.revision,
                ..ModelSelection::default()
            }))
    }

    /// Retain a refreshed route only while the selection used to read it is current.
    pub(super) fn remember_saved_route(
        &self,
        session: &str,
        selection: &mut ModelSelection,
        route: &ProviderRoute,
    ) {
        let Ok(mut state) = self.0.lock() else { return };
        let ModelSelectionState {
            revision, entries, ..
        } = &mut *state;
        let Some(current) = entries
            .get_mut(session)
            .filter(|entry| entry.revision == selection.revision)
        else {
            return;
        };
        let Some((TerminalSelection::Saved(_), cached)) = current.terminal.as_mut() else {
            return;
        };
        if cached != route {
            cached.clone_from(route);
            *revision += 1;
            current.revision = *revision;
            selection.clone_from(current);
        }
    }

    pub(super) fn begin<'a>(
        &'a self,
        session: &'a str,
    ) -> Result<ModelSelectionIntent<'a>, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "model selection lock is poisoned")?;
        state.next_intent += 1;
        let intent = state.next_intent;
        state.pending.insert(session.into(), intent);
        Ok(ModelSelectionIntent {
            selections: self,
            session,
            intent,
        })
    }
}

/// Only the latest explicit selection in a conversation may commit after validation.
pub(super) struct ModelSelectionIntent<'a> {
    selections: &'a ModelSelections,
    session: &'a str,
    intent: u64,
}

impl ModelSelectionIntent<'_> {
    pub(super) fn commit(
        self,
        options: WorkflowAgentOptions,
        terminal: Option<TerminalSelection>,
        route: ProviderRoute,
        control: &RunControl,
    ) -> Result<ModelCatalogApplied, String> {
        let mut state = self
            .selections
            .0
            .lock()
            .map_err(|_| "model selection lock is poisoned")?;
        if state.pending.get(self.session) != Some(&self.intent) {
            return Err("A newer model selection replaced this request.".into());
        }
        if control.is_cancelled() {
            return Err("Model selection was cancelled.".into());
        }
        let uses_default = options == WorkflowAgentOptions::default() && terminal.is_none();
        if !uses_default && state.entries.len() >= 128 && !state.entries.contains_key(self.session)
        {
            return Err("model selection session limit reached".into());
        }
        state.revision += 1;
        let revision = state.revision;
        let applied = ModelCatalogApplied {
            role: route.role.clone(),
            route: route_label(&route, terminal.as_ref()),
            input_budget_tokens: route.limits.input_budget_tokens,
            revision,
        };
        if uses_default {
            state.entries.remove(self.session);
        } else {
            state.entries.insert(
                self.session.into(),
                ModelSelection {
                    options,
                    terminal: terminal.map(|profile| (profile, route)),
                    revision,
                },
            );
        }
        Ok(applied)
    }
}

impl Drop for ModelSelectionIntent<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.selections.0.lock()
            && state.pending.get(self.session) == Some(&self.intent)
        {
            state.pending.remove(self.session);
        }
    }
}

pub(super) fn selected_route(
    default: ProviderRoute,
    models: &[ProviderRoute],
    options: &WorkflowAgentOptions,
) -> Result<ProviderRoute, String> {
    let role = default.role.clone();
    let mut route = match &options.model_profile {
        Some(profile) => models
            .iter()
            .find(|model| &model.model_profile == profile)
            .cloned()
            .ok_or_else(|| "selected model is no longer configured".to_owned())?,
        None => default,
    };
    route.role = role;
    if let Some(effort) = options.reasoning_effort {
        if route.provider == "echo" {
            return Err("Echo does not support reasoning effort".into());
        }
        route.reasoning_effort = Some(effort);
    }
    Ok(route)
}

pub(super) fn route_label(route: &ProviderRoute, terminal: Option<&TerminalSelection>) -> String {
    let profile = match terminal {
        Some(TerminalSelection::Saved(SavedTerminalModelRef {
            target: TerminalModelTarget::Configured { profile },
            ..
        })) => Some(profile.as_str()),
        Some(_) => None,
        None => Some(route.model_profile.as_str()),
    };
    let mut label = match profile {
        Some(profile) => format!("{}@{profile} via {}", route.model, route.provider_profile),
        None => format!("{} via {}", route.model, route.provider_profile),
    };
    if let Some(effort) = route.reasoning_effort {
        label.push_str(&format!(" · {} reasoning", effort.as_str()));
    }
    label
}

pub(super) enum ModelOptionSelection {
    Model(ModelCatalogSelection),
    Effort(Option<ReasoningEffort>),
}

pub(super) async fn select_model_options(
    name: &str,
    arguments: &str,
    current: &ModelSelection,
    models: &[ProviderRoute],
    events: &mpsc::Sender<HostEvent>,
) -> Result<Option<ModelOptionSelection>, String> {
    let (title, choices, values, selected) = if name == "model" {
        let mut choices = vec!["Use configured role default".into()];
        let mut values = vec![String::new()];
        for model in models.iter().take(256) {
            choices.push(format!(
                "{} · {} · {}",
                model.model_profile, model.model, model.provider_profile
            ));
            values.push(model.model_profile.clone());
        }
        let selected = current.configured_profile().unwrap_or("");
        ("Choose model", choices, values, selected.to_owned())
    } else {
        let values = [
            "default", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let mut choices = values.clone();
        choices[0] = "Use model default".into();
        let selected = current
            .options
            .reasoning_effort
            .map_or("default", ReasoningEffort::as_str);
        ("Choose reasoning effort", choices, values, selected.into())
    };
    let value = if arguments.trim().is_empty() {
        let (response, receiver) = oneshot::channel();
        events
            .send(HostEvent::Prompt(InteractivePrompt {
                id: uuid::Uuid::now_v7().to_string(),
                kind: InteractivePromptKind::ModelSelection,
                title: title.into(),
                document: PresentationDocument::from_block(PresentationBlock::Text(
                    "Applies to the next turn in this session. Research keeps its own model roles."
                        .into(),
                )),
                initial_choice: values.iter().position(|value| value == &selected),
                choices: choices.clone(),
                allow_free_form: false,
                response,
            }))
            .await
            .map_err(|_| "terminal event loop disconnected")?;
        let response = tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, receiver)
            .await
            .map_err(|_| "model picker timed out")?
            .map_err(|_| "model picker was dropped")?;
        match response {
            PromptResponse::Cancelled => return Ok(None),
            PromptResponse::Answer(answer) => choices
                .iter()
                .position(|choice| choice == &answer)
                .map(|index| values[index].clone())
                .ok_or("invalid model picker selection")?,
        }
    } else {
        if name == "model" && arguments.trim() == "default" {
            String::new()
        } else {
            arguments.trim().to_owned()
        }
    };
    if !values.contains(&value) {
        return Err(format!("Use /{name} to choose an available value."));
    }
    if name == "model" {
        let choice = if value.is_empty() {
            ModelCatalogSelection::RoleDefault
        } else {
            ModelCatalogSelection::Configured(value)
        };
        return Ok(Some(ModelOptionSelection::Model(choice)));
    }
    let effort = if value == "default" {
        None
    } else {
        Some(serde_json::from_value(Value::String(value)).map_err(|_| "invalid reasoning effort")?)
    };
    Ok(Some(ModelOptionSelection::Effort(effort)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{ModelCapabilities, ModelLimits};

    fn route(profile: &str) -> ProviderRoute {
        ProviderRoute {
            role: "primary".into(),
            profile: profile.into(),
            model_profile: profile.into(),
            provider_profile: "provider".into(),
            provider: "openai-compatible".into(),
            model: profile.into(),
            limits: ModelLimits {
                context_window_tokens: 8_192,
                max_output_tokens: 1_024,
                safety_margin_tokens: 820,
                input_budget_tokens: 6_348,
            },
            capabilities: ModelCapabilities {
                tool_calls: false,
                streaming: false,
                image_inputs: false,
            },
            reasoning_effort: None,
        }
    }

    fn options(profile: &str) -> WorkflowAgentOptions {
        WorkflowAgentOptions {
            model_profile: Some(profile.into()),
            reasoning_effort: None,
        }
    }

    #[test]
    fn superseded_selection_cannot_overwrite_a_newer_commit_or_another_session() {
        let selections = ModelSelections::default();
        let control = RunControl::default();
        let slow = selections.begin("first").unwrap();
        let other = selections.begin("second").unwrap();
        selections
            .begin("first")
            .unwrap()
            .commit(options("new"), None, route("new"), &control)
            .unwrap();
        assert!(
            slow.commit(options("old"), None, route("old"), &control)
                .is_err()
        );
        other
            .commit(options("other"), None, route("other"), &control)
            .unwrap();
        assert_eq!(selections.get("first").unwrap().options, options("new"));
        assert_eq!(selections.get("second").unwrap().options, options("other"));
    }

    #[test]
    fn selecting_a_saved_configured_card_keeps_only_its_session_reasoning_override() {
        let selections = ModelSelections::default();
        let control = RunControl::default();
        let mut options = options("alias");
        options.reasoning_effort = Some(ReasoningEffort::High);
        selections
            .begin("first")
            .unwrap()
            .commit(options, None, route("alias"), &control)
            .unwrap();
        let target = saved_alias();
        let (options, terminal) = selections
            .get("first")
            .unwrap()
            .catalog_choice(ModelCatalogSelection::Saved(target.clone()))
            .unwrap();
        let mut resolved = route("alias");
        resolved.reasoning_effort = options.reasoning_effort;
        let applied = selections
            .begin("first")
            .unwrap()
            .commit(options, terminal, resolved, &control)
            .unwrap();
        let chosen = selections.get("first").unwrap();
        let catalog = chosen.catalog(Vec::new(), Vec::new(), Vec::new(), true);
        assert_eq!(applied.route, "alias@alias via provider · high reasoning");
        assert_eq!(catalog.selected_profile.as_deref(), Some("alias"));
        assert_eq!(catalog.selected_saved_model, Some(target.clone()));
        assert!(chosen.transient_model().is_none());
        let options = chosen.request_options().unwrap();
        assert_eq!(options.model_profile, None);
        assert_eq!(options.reasoning_effort, Some(ReasoningEffort::High));
        assert!(selections.get("second").unwrap().saved_model().is_none());
        let rebound = SavedTerminalModelRef {
            binding: "2".repeat(64),
            ..target.clone()
        };
        let (rebound_options, _) = chosen
            .catalog_choice(ModelCatalogSelection::Saved(rebound))
            .unwrap();
        assert_eq!(rebound_options.reasoning_effort, None);

        let (options, terminal) = chosen.effort_choice(None);
        selections
            .begin("first")
            .unwrap()
            .commit(options, terminal, route("alias"), &control)
            .unwrap();
        let chosen = selections.get("first").unwrap();
        assert_eq!(chosen.saved_model(), Some(target));
        assert!(chosen.request_options().is_none());
    }

    #[test]
    fn refreshed_saved_route_replaces_the_fallback_and_rejects_an_older_footer_lookup() {
        let selections = ModelSelections::default();
        let initial = route("alias");
        selections
            .begin("session")
            .unwrap()
            .commit(
                WorkflowAgentOptions::default(),
                Some(TerminalSelection::Saved(saved_alias())),
                initial.clone(),
                &RunControl::default(),
            )
            .unwrap();

        let mut older_lookup = selections.get("session").unwrap();
        let mut current = older_lookup.clone();
        let mut refreshed = initial.clone();
        refreshed.limits.input_budget_tokens = 2_048;
        refreshed.reasoning_effort = Some(ReasoningEffort::Low);
        selections.remember_saved_route("session", &mut current, &refreshed);
        assert!(current.revision > older_lookup.revision);
        selections.remember_saved_route("session", &mut older_lookup, &initial);
        let cached = selections.get("session").unwrap();
        assert_eq!(cached.revision, current.revision);
        assert_eq!(cached.terminal.map(|(_, route)| route), Some(refreshed));
    }

    #[test]
    fn cancelled_validation_cannot_commit_a_new_route() {
        let selections = ModelSelections::default();
        let intent = selections.begin("session").unwrap();
        let control = RunControl::default();
        control.cancel();
        assert!(
            intent
                .commit(options("alias"), None, route("alias"), &control)
                .is_err()
        );
        assert!(
            selections
                .get("session")
                .unwrap()
                .request_options()
                .is_none()
        );
    }

    fn saved_alias() -> SavedTerminalModelRef {
        SavedTerminalModelRef {
            target: TerminalModelTarget::Configured {
                profile: "alias".into(),
            },
            binding: "1".repeat(64),
        }
    }
}
