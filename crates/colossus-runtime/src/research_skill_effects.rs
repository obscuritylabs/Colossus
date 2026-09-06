use super::*;

pub(super) struct ResearchEffectExecutor {
    pub(super) service: Arc<ResearchService>,
}

pub(super) struct PluginEffectExecutor {
    pub(super) catalog: Arc<PluginCatalogSource>,
}

#[async_trait]
impl EffectExecutor for PluginEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: PluginOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(ExecutionError::Failed(
                "plugin request does not match authorized content".into(),
            ));
        }
        let catalog = self
            .catalog
            .capture()
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let plugins = &catalog.records;
        let value = match operation {
            PluginOperation::List => {
                serde_json::to_value(super::plugin_catalog::narrow_plugin_inventory(
                    plugins
                        .iter()
                        .map(AgentPluginRecord::inventory)
                        .collect::<Vec<_>>(),
                    &self.catalog.configuration,
                ))
            }
            PluginOperation::Inspect { plugin_name } => serde_json::to_value(
                super::plugin_catalog::narrow_plugin_inventory(
                    vec![find_plugin(plugins, &plugin_name)?.inventory()],
                    &self.catalog.configuration,
                )
                .pop(),
            ),
            PluginOperation::SkillRead { skill_id } => {
                serde_json::to_value(find_skill(plugins, &skill_id)?)
            }
            PluginOperation::ListResources { skill_id } => serde_json::to_value(
                list_plugin_resources(find_skill(plugins, &skill_id)?)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            PluginOperation::ReadResource { skill_id, path } => serde_json::to_value(
                read_plugin_resource(find_skill(plugins, &skill_id)?, &path)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
        }
        .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

fn find_plugin<'a>(
    plugins: &'a [AgentPluginRecord],
    name: &str,
) -> Result<&'a AgentPluginRecord, ExecutionError> {
    plugins
        .iter()
        .find(|plugin| plugin.installation.manifest.name == name)
        .ok_or_else(|| ExecutionError::Failed(format!("plugin is not active: {name}")))
}

fn find_skill<'a>(
    plugins: &'a [AgentPluginRecord],
    id: &str,
) -> Result<&'a colossus_contracts::PluginSkillRecord, ExecutionError> {
    plugins
        .iter()
        .flat_map(|plugin| &plugin.skills)
        .find(|skill| skill.id == id)
        .ok_or_else(|| ExecutionError::Failed(format!("plugin skill is not active: {id}")))
}

#[async_trait]
impl EffectExecutor for ResearchEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: ResearchOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action()
            || request.context.session_id.as_deref() != Some(operation.session_id())
        {
            return Err(ExecutionError::Failed(
                "research operation does not match its authorized session context".into(),
            ));
        }
        let ResearchOperation::Run {
            session_id,
            question,
            depth,
            source_kinds,
            message_run_id,
        } = operation;
        if request.context.run_id != message_run_id {
            return Err(ExecutionError::Failed(
                "research operation does not match its authorized run context".into(),
            ));
        }
        let offered_tools = request
            .context
            .offered_tools
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if source_kinds.iter().any(|kind| {
            !offered_tools.contains(match kind {
                ResearchSourceKind::Repo => "filesystem.search",
                ResearchSourceKind::Web => "web.search",
                ResearchSourceKind::Mcp => "mcp.call",
            })
        }) {
            return Err(ExecutionError::Failed(
                "research evidence lane exceeds the authorized tool ceiling".into(),
            ));
        }
        let run = self
            .service
            .run_with_message_run_id(
                &session_id,
                &question,
                depth,
                source_kinds,
                message_run_id.as_deref(),
                request.actor.clone(),
            )
            .await
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&run)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}
