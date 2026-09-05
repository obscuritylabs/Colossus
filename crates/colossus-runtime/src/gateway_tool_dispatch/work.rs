use super::*;

impl GatewayToolExecutor {
    pub(super) async fn execute_work(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let exit_code = 0;
        let output = match call.name.as_str() {
            "task.create" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::TaskCreate {
                        session_id,
                        title: required_tool_string(&call, "title")?.into(),
                        description: optional_tool_string(&call, "description")?
                            .unwrap_or_default()
                            .into(),
                        status: optional_tool_value(&call, "status")?
                            .unwrap_or(TaskStatus::Pending),
                    },
                )
                .await?
            }
            "task.update" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::TaskUpdate {
                        id: required_tool_string(&call, "id")?.into(),
                        title: optional_tool_string(&call, "title")?.map(str::to_owned),
                        description: optional_tool_string(&call, "description")?.map(str::to_owned),
                        status: optional_tool_value(&call, "status")?,
                    },
                )
                .await?
            }
            "task.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::TaskList {
                        session_id,
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                    },
                )
                .await?
            }
            "decision.create" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionCreate {
                        session_id,
                        title: required_tool_string(&call, "title")?.into(),
                        decision: required_tool_string(&call, "decision")?.into(),
                        source: DecisionSource::Agent,
                        priority: optional_tool_value(&call, "priority")?
                            .unwrap_or(DecisionPriority::Normal),
                        intent: optional_tool_string(&call, "intent")?
                            .unwrap_or_default()
                            .into(),
                        applies_when: optional_tool_string(&call, "applies_when")?
                            .unwrap_or_default()
                            .into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                        source_excerpt: optional_tool_string(&call, "source_excerpt")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "decision.update" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionUpdate {
                        id: required_tool_string(&call, "id")?.into(),
                        title: optional_tool_string(&call, "title")?.map(str::to_owned),
                        decision: optional_tool_string(&call, "decision")?.map(str::to_owned),
                        priority: optional_tool_value(&call, "priority")?,
                        intent: optional_tool_string(&call, "intent")?.map(str::to_owned),
                        applies_when: optional_tool_string(&call, "applies_when")?
                            .map(str::to_owned),
                        rationale: optional_tool_string(&call, "rationale")?.map(str::to_owned),
                        source_excerpt: optional_tool_string(&call, "source_excerpt")?
                            .map(str::to_owned),
                    },
                )
                .await?
            }
            "decision.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionList {
                        session_id,
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                    },
                )
                .await?
            }
            "decision.archive" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionArchive {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "decision.supersede" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionSupersede {
                        id: required_tool_string(&call, "id")?.into(),
                        title: required_tool_string(&call, "title")?.into(),
                        decision: required_tool_string(&call, "decision")?.into(),
                        source: DecisionSource::Agent,
                        priority: optional_tool_value(&call, "priority")?
                            .unwrap_or(DecisionPriority::Normal),
                        intent: optional_tool_string(&call, "intent")?
                            .unwrap_or_default()
                            .into(),
                        applies_when: optional_tool_string(&call, "applies_when")?
                            .unwrap_or_default()
                            .into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                        source_excerpt: optional_tool_string(&call, "source_excerpt")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "agent.delegate" => {
                if context.subagent_id.is_some() {
                    return Err(ToolError::Denied(
                        "subagents cannot delegate recursively".into(),
                    ));
                }
                let session_id = Self::current_session(&context)?;
                let parent_run_id = context.run_id.clone().ok_or_else(|| {
                    ToolError::Denied("agent.delegate requires a parent run".into())
                })?;
                let allowed_tools = context.offered_tools.clone();
                let instruction_snapshot_id = if let Some(snapshot) = active_instruction_snapshot()
                {
                    let snapshot = snapshot.with_plugin_selections(&context.skill_ids);
                    self.work
                        .as_ref()
                        .ok_or_else(|| ToolError::Failed("work adapter is unavailable".into()))?
                        .instruction_snapshots
                        .persist(&snapshot)
                        .map_err(|error| ToolError::Failed(error.to_string()))?;
                    Some(snapshot.id().to_owned())
                } else {
                    None
                };
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::SubagentCreate {
                        session_id,
                        parent_run_id,
                        parent_call_id: call.call_id.clone(),
                        task: required_tool_string(&call, "task")?.into(),
                        role: "subagent_default".into(),
                        allowed_tools: Some(allowed_tools),
                        instruction_snapshot_id,
                    },
                )
                .await?
            }
            "agent.result" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::SubagentRead {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "agent.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::SubagentList {
                        session_id,
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                    },
                )
                .await?
            }
            "goal.show" => {
                let id = context.goal_id.clone().ok_or_else(|| {
                    ToolError::Denied(
                        "goal.show is available only during an active goal run".into(),
                    )
                })?;
                self.execute_work_tool(&call, context, WorkOperation::GoalShow { id })
                    .await?
            }
            "goal.update" => {
                let id = context.goal_id.clone().ok_or_else(|| {
                    ToolError::Denied(
                        "goal.update is available only during an active goal run".into(),
                    )
                })?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::GoalUpdate {
                        id,
                        status: optional_tool_value(&call, "status")?.ok_or_else(|| {
                            ToolError::InvalidArguments {
                                tool: call.name.clone(),
                                message: "status is required".into(),
                            }
                        })?,
                        summary: optional_tool_string(&call, "summary")?
                            .unwrap_or_default()
                            .into(),
                        blocked_reason: optional_tool_string(&call, "blocked_reason")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "plan.create" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanCreate {
                        session_id,
                        prompt: required_tool_string(&call, "prompt")?.into(),
                        content: optional_tool_string(&call, "content")?
                            .unwrap_or_default()
                            .into(),
                        steps: tool_plan_steps(&call)?,
                    },
                )
                .await?
            }
            "plan.update" => {
                let id = context.draft_plan_id.clone().ok_or_else(|| {
                    ToolError::Denied(
                        "plan.update requires a runtime-bound Plan Mode draft target".into(),
                    )
                })?;
                let expected_revision = context.draft_plan_revision.ok_or_else(|| {
                    ToolError::Denied("plan.update requires a runtime-bound draft revision".into())
                })?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanUpdate {
                        id,
                        expected_revision,
                        content: required_tool_string(&call, "content")?.into(),
                        steps: tool_plan_steps(&call)?,
                    },
                )
                .await?
            }
            "plan.show" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanShow {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "plan.approve_request" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanApprove {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "memory.create" => {
                let session_id = Self::current_session(&context)?;
                let scope = match optional_tool_string(&call, "scope")?.unwrap_or("session") {
                    "global" => MemoryScope::Global,
                    "repository" => MemoryScope::Repository(self.repository_id.clone()),
                    "session" => MemoryScope::Session(session_id),
                    value => {
                        return Err(ToolError::InvalidArguments {
                            tool: call.name.clone(),
                            message: format!("unknown memory scope {value}"),
                        });
                    }
                };
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Create {
                        scope,
                        kind: required_tool_string(&call, "kind")?.into(),
                        confidence: optional_tool_value(&call, "confidence")?.unwrap_or(1.0),
                        text: required_tool_string(&call, "text")?.into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                        expires_at: optional_tool_string(&call, "expires_at")?.map(str::to_owned),
                    },
                )
                .await?
            }
            "memory.update" => {
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Update {
                        id: required_tool_string(&call, "id")?.into(),
                        text: optional_tool_string(&call, "text")?.map(str::to_owned),
                        rationale: optional_tool_string(&call, "rationale")?.map(str::to_owned),
                        confidence: optional_tool_value(&call, "confidence")?,
                    },
                )
                .await?
            }
            "memory.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::List {
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                        session_id: Some(session_id),
                        repository_id: Some(self.repository_id.clone()),
                    },
                )
                .await?
            }
            "memory.search" => {
                let session_id = Self::current_session(&context)?;
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Search {
                        query: required_tool_string(&call, "query")?.into(),
                        session_id: Some(session_id),
                        repository_id: Some(self.repository_id.clone()),
                        limit: tool_limit(&call, 20)?,
                    },
                )
                .await?
            }
            "memory.archive" => {
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Archive {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "memory.supersede" => {
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Supersede {
                        id: required_tool_string(&call, "id")?.into(),
                        text: required_tool_string(&call, "text")?.into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            name => return Err(ToolError::Unknown(name.into())),
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code,
        })
    }
}
