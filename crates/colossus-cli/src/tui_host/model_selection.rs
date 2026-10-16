use super::*;
use colossus_contracts::WorkflowAgentOptions;

#[derive(Default)]
pub(super) struct ModelSelections(Mutex<BTreeMap<String, WorkflowAgentOptions>>);

impl ModelSelections {
    pub(super) fn get(&self, session: &str) -> Result<WorkflowAgentOptions, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "model selection lock is poisoned")?
            .get(session)
            .cloned()
            .unwrap_or_default())
    }

    pub(super) fn set(&self, session: &str, options: WorkflowAgentOptions) -> Result<(), String> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "model selection lock is poisoned")?;
        if options == WorkflowAgentOptions::default() {
            entries.remove(session);
            return Ok(());
        }
        if entries.len() >= 128 && !entries.contains_key(session) {
            return Err("model selection session limit reached".into());
        }
        entries.insert(session.into(), options);
        Ok(())
    }

    /// Omit defaults so an ordinary turn keeps the legacy authenticated wire shape.
    pub(super) fn for_request(
        &self,
        session: &str,
    ) -> Result<Option<WorkflowAgentOptions>, String> {
        let options = self.get(session)?;
        Ok((options != WorkflowAgentOptions::default()).then_some(options))
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

pub(super) fn route_label(route: &ProviderRoute) -> String {
    let mut label = format!(
        "{}@{} via {}",
        route.model, route.model_profile, route.provider_profile
    );
    if let Some(effort) = route.reasoning_effort {
        label.push_str(&format!(" · {} reasoning", effort.as_str()));
    }
    label
}

pub(super) async fn select_model_options(
    name: &str,
    arguments: &str,
    current: WorkflowAgentOptions,
    models: &[ProviderRoute],
    events: &mpsc::Sender<HostEvent>,
) -> Result<Option<WorkflowAgentOptions>, String> {
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
        let selected = current.model_profile.as_deref().unwrap_or("");
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
    let mut next = current;
    if name == "model" {
        let profile = (!value.is_empty()).then_some(value);
        if profile != next.model_profile {
            next.model_profile = profile;
            next.reasoning_effort = None;
        }
    } else {
        next.reasoning_effort = if value == "default" {
            None
        } else {
            Some(
                serde_json::from_value(Value::String(value))
                    .map_err(|_| "invalid reasoning effort")?,
            )
        };
    }
    Ok(Some(next))
}
