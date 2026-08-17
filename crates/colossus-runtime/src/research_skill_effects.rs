use super::*;

pub(super) struct ResearchEffectExecutor {
    pub(super) service: Arc<ResearchService>,
}

pub(super) struct SkillEffectExecutor {
    pub(super) resources: Arc<SkillResourceService>,
    pub(super) authoring: Arc<SkillAuthoringService>,
}

#[async_trait]
impl EffectExecutor for SkillEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: SkillOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(ExecutionError::Failed(
                "skill request does not match authorized content".into(),
            ));
        }
        let value = match operation {
            SkillOperation::Scaffold {
                name,
                description,
                instructions,
                resource_dirs,
            } => serde_json::to_value(
                self.authoring
                    .scaffold(&permit, &name, &description, &instructions, &resource_dirs)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::Inspect { name } => serde_json::to_value(
                self.authoring
                    .inspect_installed(&permit, &name)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::ReadFile { name, path } => serde_json::to_value(
                self.authoring
                    .read_installed(&permit, &name, &path)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::WriteFile {
                name,
                path,
                content,
                expected_sha256,
            } => serde_json::to_value(
                self.authoring
                    .write_installed(&permit, &name, &path, &content, expected_sha256.as_deref())
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::ValidateInstalled { name } => serde_json::to_value(
                self.authoring
                    .validate_installed(&permit, &name)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::ValidateLocal { path } => serde_json::to_value(
                self.authoring
                    .validate_local(&permit, Path::new(&path))
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::InstallLocal { path } => serde_json::to_value(
                self.authoring
                    .install_local(&permit, Path::new(&path))
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::ListResources {
                skill_name,
                active_skills,
            } => serde_json::to_value(
                self.resources
                    .list_resources(&permit, &skill_name, &active_skills)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            SkillOperation::ReadResource {
                skill_name,
                path,
                active_skills,
            } => serde_json::to_value(
                self.resources
                    .read_resource(&permit, &skill_name, &path, &active_skills)
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
