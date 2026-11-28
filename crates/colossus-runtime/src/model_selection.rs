use super::*;
use colossus_contracts::{
    ModelFeatureDeclarations, ModelFeatureSettings, ProviderModelInfo, ReasoningEffort,
    TerminalModelProfile, WorkflowAgentOptions,
};
use colossus_provider::ResolvedModel;
use std::future::Future;

tokio::task_local! {
    static TERMINAL_MODEL: TerminalModelScope;
}

struct TerminalModelScope {
    registry: Arc<ProviderRegistry>,
    role: String,
    selection: String,
    model: ResolvedModel,
}

enum TerminalSelectionSource {
    CurrentCatalog,
    Issued,
}

pub(super) fn scoped_terminal_model(
    registry: &ProviderRegistry,
    role: &str,
    selection: &str,
) -> Option<ResolvedModel> {
    TERMINAL_MODEL
        .try_with(|scope| {
            (std::ptr::eq(registry, scope.registry.as_ref())
                && role == scope.role
                && selection == scope.selection)
                .then(|| scope.model.clone())
        })
        .ok()
        .flatten()
}

impl Runtime {
    /// Resolve only existing configured models, without making a provider request.
    /// Reasoning preferences never change role or execution authority.
    pub fn model_route_with_options(
        &self,
        role: &str,
        options: &WorkflowAgentOptions,
    ) -> Result<ModelRoute, RuntimeError> {
        self.providers.require_role_available(role)?;
        let model = match &options.model_profile {
            Some(profile) if !profile.is_empty() && profile.len() <= 128 => {
                self.providers.model(profile)?
            }
            Some(_) => {
                return Err(RuntimeError::Config(
                    "invalid configured model profile".into(),
                ));
            }
            None => self.providers.resolve(role)?,
        };
        let mut route = model.route();
        route.role = role.into();
        if let Some(effort) = options.reasoning_effort {
            if route.provider == "echo" {
                return Err(RuntimeError::Config(
                    "Echo does not support reasoning effort".into(),
                ));
            }
            route.reasoning_effort = Some(effort);
        }
        Ok(route)
    }

    /// Snapshot a user's model preferences for this operation and its Goal iterations.
    /// Callers validate the configured route before accepting an operation. Internal
    /// workflow and child-agent routes remain independently configured.
    pub fn with_model_selection<F: Future>(
        &self,
        options: WorkflowAgentOptions,
        future: F,
    ) -> impl Future<Output = F::Output> {
        // Bound stack use before nesting the execution inside task-local scopes.
        // Goal futures include the agent loop and can otherwise exceed a normal
        // Windows worker thread's stack in debug builds.
        colossus_agent::scope_model_selection(options, Box::pin(future))
    }

    /// Confirm a discovered model for one session against the current released catalog.
    pub fn terminal_model_route(
        &self,
        role: &str,
        session_id: &str,
        profile: &TerminalModelProfile,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<ModelRoute, RuntimeError> {
        self.resolve_terminal_model(
            role,
            session_id,
            profile,
            reasoning_effort,
            TerminalSelectionSource::CurrentCatalog,
        )
        .map(|model| model.route())
    }

    /// Revalidate reasoning for this session's exact issued configuration without a catalog fetch.
    pub fn pinned_terminal_model_route(
        &self,
        role: &str,
        session_id: &str,
        profile: &TerminalModelProfile,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<ModelRoute, RuntimeError> {
        self.resolve_terminal_model(
            role,
            session_id,
            profile,
            reasoning_effort,
            TerminalSelectionSource::Issued,
        )
        .map(|model| model.route())
    }

    /// Snapshot a discovered model for the root agent turns of one terminal operation.
    /// The selection is private to this runtime and future. Callers must retain the
    /// editable profile in their own terminal session if they want to use it again.
    pub fn with_terminal_model_selection<F: Future>(
        &self,
        role: &str,
        session_id: &str,
        profile: &TerminalModelProfile,
        reasoning_effort: Option<ReasoningEffort>,
        future: F,
    ) -> Result<impl Future<Output = F::Output>, RuntimeError> {
        let model = self.resolve_terminal_model(
            role,
            session_id,
            profile,
            reasoning_effort,
            TerminalSelectionSource::Issued,
        )?;
        // Only agent turns that explicitly inherit this task's selection may resolve
        // the transient route. Other workflow and delegated routes use their own
        // configured options, even when executed in the same task.
        let selection = format!("terminal-selection-{}", Uuid::now_v7());
        let options = WorkflowAgentOptions {
            model_profile: Some(selection.clone()),
            reasoning_effort,
        };
        let scope = TerminalModelScope {
            registry: Arc::clone(&self.providers),
            role: role.into(),
            selection,
            model,
        };
        Ok(TERMINAL_MODEL.scope(scope, self.with_model_selection(options, future)))
    }

    fn resolve_terminal_model(
        &self,
        role: &str,
        session_id: &str,
        profile: &TerminalModelProfile,
        reasoning_effort: Option<ReasoningEffort>,
        source: TerminalSelectionSource,
    ) -> Result<ResolvedModel, RuntimeError> {
        self.providers.require_role_available(role)?;
        let provider = self.providers.profile(&profile.provider_profile)?;
        if provider.profile().kind == ProviderKind::Echo && reasoning_effort.is_some() {
            return Err(RuntimeError::Config(
                "Echo does not support reasoning effort".into(),
            ));
        }
        let mut model = match source {
            TerminalSelectionSource::CurrentCatalog => {
                self.terminal_model_catalog
                    .confirm_model(session_id, profile, |card| {
                        prepare_terminal_model(profile, card)
                    })?
            }
            TerminalSelectionSource::Issued => self
                .terminal_model_catalog
                .pinned_model(session_id, profile)?,
        };
        model.reasoning_effort = reasoning_effort;
        self.providers
            .resolve_terminal_model(role, model)
            .map_err(Into::into)
    }
}

fn prepare_terminal_model(
    profile: &TerminalModelProfile,
    card: &ProviderModelInfo,
) -> Result<ModelProfile, RuntimeError> {
    if let Some(limit) = card.context_window_tokens
        && profile.context_window_tokens > limit
    {
        return Err(RuntimeError::Config(format!(
            "The context window exceeds the model's advertised limit of {limit} tokens."
        )));
    }
    if let Some(limit) = card.max_output_tokens
        && profile.max_output_tokens > limit
    {
        return Err(RuntimeError::Config(format!(
            "The output limit exceeds the model's advertised limit of {limit} tokens."
        )));
    }
    let settings = ModelFeatureSettings {
        declared: ModelFeatureDeclarations {
            tool_calls: card.tool_calls,
            streaming: card.streaming,
            image_inputs: card.image_inputs,
            server_compaction: card.server_compaction,
        },
        ..ModelFeatureSettings::default()
    };
    let mut identity = Sha256::new();
    identity.update(profile.provider_profile.as_bytes());
    identity.update([0]);
    identity.update(profile.model.as_bytes());
    identity.update(profile.context_window_tokens.to_be_bytes());
    identity.update(profile.max_output_tokens.to_be_bytes());
    let model = ModelProfile::new(
        format!("terminal-{}", hex::encode(identity.finalize())),
        &profile.provider_profile,
        &profile.model,
        profile.context_window_tokens,
        profile.max_output_tokens,
        settings.capabilities(),
        None,
    )?
    .with_feature_settings(settings);
    Ok(model)
}
