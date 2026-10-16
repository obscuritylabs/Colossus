use super::*;
use colossus_contracts::WorkflowAgentOptions;
use std::future::Future;

impl Runtime {
    /// Resolve only existing configured models, without making a provider request.
    /// Reasoning preferences never change role or execution authority.
    pub fn model_route_with_options(
        &self,
        role: &str,
        options: &WorkflowAgentOptions,
    ) -> Result<ModelRoute, RuntimeError> {
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
}
