use super::*;

#[derive(Clone, Default)]
pub(super) struct ModelSelection {
    pub(super) options: WorkflowAgentOptions,
    pub(super) terminal: Option<(TerminalModelProfile, ProviderRoute)>,
    pub(super) revision: u64,
}

impl ModelSelection {
    /// Omit defaults so an ordinary turn keeps the legacy authenticated wire shape.
    pub(super) fn request_options(&self) -> Option<WorkflowAgentOptions> {
        (self.options != WorkflowAgentOptions::default()).then(|| self.options.clone())
    }

    pub(super) fn catalog(
        self,
        configured: Vec<ProviderRoute>,
        providers: Vec<ProviderReadiness>,
    ) -> InteractiveModelCatalog {
        InteractiveModelCatalog {
            configured,
            providers: providers
                .into_iter()
                .filter(|provider| provider.provider != "echo")
                .map(|provider| provider.profile)
                .collect(),
            selected_profile: self.options.model_profile,
            selected_terminal_model: self.terminal.map(|(profile, _)| profile),
        }
    }

    pub(super) fn catalog_choice(
        &self,
        selection: ModelCatalogSelection,
    ) -> (WorkflowAgentOptions, Option<TerminalModelProfile>) {
        let (configured, terminal) = match selection {
            ModelCatalogSelection::RoleDefault => (None, None),
            ModelCatalogSelection::Configured(profile) => (Some(profile), None),
            ModelCatalogSelection::Discovered(profile) => (None, Some(profile)),
        };
        let current = self.terminal.as_ref().map(|(profile, _)| profile);
        let same_model = match (current, terminal.as_ref()) {
            (Some(a), Some(b)) => a.provider_profile == b.provider_profile && a.model == b.model,
            (None, None) => self.options.model_profile == configured,
            _ => false,
        };
        (
            WorkflowAgentOptions {
                model_profile: configured,
                reasoning_effort: same_model
                    .then_some(self.options.reasoning_effort)
                    .flatten(),
            },
            terminal,
        )
    }
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
        terminal: Option<TerminalModelProfile>,
        route: ProviderRoute,
    ) -> Result<ModelCatalogApplied, String> {
        let mut state = self
            .selections
            .0
            .lock()
            .map_err(|_| "model selection lock is poisoned")?;
        if state.pending.get(self.session) != Some(&self.intent) {
            return Err("A newer model selection replaced this request.".into());
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
            route: route_label(&route, terminal.is_some()),
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

pub(super) fn route_label(route: &ProviderRoute, terminal: bool) -> String {
    let mut label = if terminal {
        format!("{} via {}", route.model, route.provider_profile)
    } else {
        format!(
            "{}@{} via {}",
            route.model, route.model_profile, route.provider_profile
        )
    };
    if let Some(effort) = route.reasoning_effort {
        label.push_str(&format!(" · {} reasoning", effort.as_str()));
    }
    label
}

pub(super) async fn select_model_options(
    name: &str,
    arguments: &str,
    current: &ModelSelection,
    models: &[ProviderRoute],
    events: &mpsc::Sender<HostEvent>,
) -> Result<Option<(WorkflowAgentOptions, Option<TerminalModelProfile>)>, String> {
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
        let selected = current.options.model_profile.as_deref().unwrap_or("");
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
        return Ok(Some(current.catalog_choice(choice)));
    }
    let mut options = current.options.clone();
    options.reasoning_effort = if value == "default" {
        None
    } else {
        Some(serde_json::from_value(Value::String(value)).map_err(|_| "invalid reasoning effort")?)
    };
    Ok(Some((
        options,
        current
            .terminal
            .as_ref()
            .map(|(profile, _)| profile.clone()),
    )))
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
        let slow = selections.begin("first").unwrap();
        let other = selections.begin("second").unwrap();
        selections
            .begin("first")
            .unwrap()
            .commit(options("new"), None, route("new"))
            .unwrap();
        assert!(slow.commit(options("old"), None, route("old")).is_err());
        other
            .commit(options("other"), None, route("other"))
            .unwrap();
        assert_eq!(selections.get("first").unwrap().options, options("new"));
        assert_eq!(selections.get("second").unwrap().options, options("other"));
    }
}
