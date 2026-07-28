use super::*;

pub(super) struct PresentationEffectExecutor {
    pub(super) repository: Arc<dyn PresentationRepository>,
}

#[async_trait]
impl EffectExecutor for PresentationEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: PresentationOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(ExecutionError::Failed(
                "presentation request does not match its authorized content".into(),
            ));
        }
        let output = match operation {
            PresentationOperation::Save { preferences } => serde_json::to_vec(
                &self
                    .repository
                    .save(preferences, request.actor.clone())
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
            PresentationOperation::AppendHistory { entry } => serde_json::to_vec(
                &self
                    .repository
                    .append_history(entry, request.actor.clone())
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            ),
        }
        .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: output,
            effect_succeeded: true,
        })
    }
}

pub(super) struct WorkEffectExecutor {
    pub(super) service: Arc<WorkService>,
    pub(super) repository: Arc<dyn WorkRepository>,
}

impl WorkEffectExecutor {
    fn validate_scope(
        &self,
        request: &EffectRequest,
        operation: &WorkOperation,
    ) -> Result<(), ExecutionError> {
        if !matches!(
            request.actor.actor_type,
            ActorType::Model | ActorType::Workflow | ActorType::Subagent
        ) {
            return Ok(());
        }
        let requested_session =
            request.context.session_id.as_deref().ok_or_else(|| {
                ExecutionError::Failed("work tool session context is absent".into())
            })?;
        let operation_session = match operation {
            WorkOperation::TaskCreate { session_id, .. }
            | WorkOperation::TaskList { session_id, .. }
            | WorkOperation::DecisionCreate { session_id, .. }
            | WorkOperation::DecisionList { session_id, .. }
            | WorkOperation::PlanCreate { session_id, .. }
            | WorkOperation::GoalCreate { session_id, .. }
            | WorkOperation::SubagentCreate { session_id, .. }
            | WorkOperation::SubagentList { session_id, .. } => session_id.clone(),
            WorkOperation::TaskUpdate { id, .. } => {
                self.repository
                    .get_task(id)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?
                    .ok_or_else(|| ExecutionError::Failed(format!("task {id} was not found")))?
                    .session_id
            }
            WorkOperation::PlanShow { id }
            | WorkOperation::PlanApprove { id }
            | WorkOperation::PlanExecute { id, .. } => {
                self.repository
                    .get_plan(id)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?
                    .ok_or_else(|| ExecutionError::Failed(format!("plan {id} was not found")))?
                    .session_id
            }
            WorkOperation::GoalShow { id }
            | WorkOperation::GoalUpdate { id, .. }
            | WorkOperation::GoalIteration { id } => {
                let context_goal = request.context.goal_id.as_deref().ok_or_else(|| {
                    ExecutionError::Failed("goal tools require an active goal context".into())
                })?;
                if id != context_goal {
                    return Err(ExecutionError::Failed(
                        "goal tool cannot access another active goal".into(),
                    ));
                }
                self.repository
                    .get_goal(id)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?
                    .ok_or_else(|| ExecutionError::Failed(format!("goal {id} was not found")))?
                    .session_id
            }
            WorkOperation::DecisionUpdate { id, .. }
            | WorkOperation::DecisionArchive { id }
            | WorkOperation::DecisionSupersede { id, .. } => {
                self.repository
                    .get_decision(id)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?
                    .ok_or_else(|| ExecutionError::Failed(format!("decision {id} was not found")))?
                    .session_id
            }
            WorkOperation::SubagentRead { id }
            | WorkOperation::SubagentStart { id }
            | WorkOperation::SubagentComplete { id, .. }
            | WorkOperation::SubagentStop { id, .. }
            | WorkOperation::SubagentRequeue { id } => {
                self.repository
                    .get_subagent(id)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?
                    .ok_or_else(|| ExecutionError::Failed(format!("subagent {id} was not found")))?
                    .session_id
            }
        };
        if request.context.subagent_id.is_some()
            && matches!(operation, WorkOperation::SubagentCreate { .. })
        {
            return Err(ExecutionError::Failed(
                "subagents cannot delegate recursively".into(),
            ));
        }
        if operation_session != requested_session {
            return Err(ExecutionError::Failed(
                "work tool cannot access another session".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl EffectExecutor for WorkEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let mutation: WorkOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != mutation.action() {
            return Err(ExecutionError::Failed(
                "work mutation action does not match its validated content".into(),
            ));
        }
        self.validate_scope(request, &mutation)?;
        let actor = request.actor.clone();
        let value = match mutation {
            WorkOperation::TaskCreate {
                session_id,
                title,
                description,
                status,
            } => work_result(self.service.create_task(
                &session_id,
                &title,
                &description,
                status,
                actor,
            )),
            WorkOperation::TaskUpdate {
                id,
                title,
                description,
                status,
            } => work_result(self.service.update_task(
                &id,
                title.as_deref(),
                description.as_deref(),
                status,
                actor,
            )),
            WorkOperation::TaskList {
                session_id,
                status,
                limit,
            } => work_result(self.repository.list_tasks(Some(&session_id), status, limit)),
            WorkOperation::DecisionCreate {
                session_id,
                title,
                decision,
                source,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => {
                validate_decision_source(&actor, source)?;
                work_result(self.service.create_decision(
                    &session_id,
                    &title,
                    &decision,
                    source,
                    priority,
                    &intent,
                    &applies_when,
                    &rationale,
                    &source_excerpt,
                    None,
                    None,
                    None,
                    actor,
                ))
            }
            WorkOperation::DecisionUpdate {
                id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => work_result(self.service.update_decision(
                &id,
                title.as_deref(),
                decision.as_deref(),
                priority,
                intent.as_deref(),
                applies_when.as_deref(),
                rationale.as_deref(),
                source_excerpt.as_deref(),
                actor,
            )),
            WorkOperation::DecisionArchive { id } => {
                work_result(self.service.archive_decision(&id, actor))
            }
            WorkOperation::DecisionSupersede {
                id,
                title,
                decision,
                source,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => {
                validate_decision_source(&actor, source)?;
                work_result(self.service.supersede_decision(
                    &id,
                    &title,
                    &decision,
                    source,
                    priority,
                    &intent,
                    &applies_when,
                    &rationale,
                    &source_excerpt,
                    actor,
                ))
            }
            WorkOperation::DecisionList {
                session_id,
                status,
                limit,
            } => work_result(
                self.repository
                    .list_decisions(Some(&session_id), status, limit),
            ),
            WorkOperation::PlanCreate {
                session_id,
                prompt,
                content,
                steps,
            } => {
                work_result(
                    self.service
                        .create_plan(&session_id, &prompt, &content, steps, actor),
                )
            }
            WorkOperation::PlanShow { id } => {
                work_result(self.repository.get_plan(&id).and_then(|plan| {
                    plan.ok_or_else(|| StoreError::NotFound(format!("plan {id}")))
                }))
            }
            WorkOperation::PlanApprove { id } => work_result(self.service.approve_plan(&id, actor)),
            WorkOperation::PlanExecute { id, run_id } => {
                work_result(self.service.execute_plan(&id, &run_id, actor))
            }
            WorkOperation::GoalCreate {
                session_id,
                objective,
                iteration_budget,
                source_plan_id,
            } => work_result(self.service.create_goal(
                &session_id,
                &objective,
                iteration_budget,
                source_plan_id,
                actor,
            )),
            WorkOperation::GoalShow { id } => {
                work_result(self.repository.get_goal(&id).and_then(|goal| {
                    goal.ok_or_else(|| StoreError::NotFound(format!("goal {id}")))
                }))
            }
            WorkOperation::GoalUpdate {
                id,
                status,
                summary,
                blocked_reason,
            } => work_result(self.service.update_goal_status(
                &id,
                status,
                &summary,
                &blocked_reason,
                actor,
            )),
            WorkOperation::GoalIteration { id } => {
                work_result(self.service.record_goal_iteration(&id, actor))
            }
            WorkOperation::SubagentCreate {
                session_id,
                parent_run_id,
                parent_call_id,
                task,
                role,
                allowed_tools,
            } => work_result(self.service.create_subagent(
                colossus_work::CreateSubagentRequest {
                    session_id,
                    parent_run_id,
                    parent_call_id,
                    task,
                    role,
                    allowed_tools,
                },
                actor,
            )),
            WorkOperation::SubagentRead { id } => {
                work_result(self.repository.get_subagent(&id).and_then(|job| {
                    job.ok_or_else(|| StoreError::NotFound(format!("subagent {id}")))
                }))
            }
            WorkOperation::SubagentList {
                session_id,
                status,
                limit,
            } => work_result(
                self.repository
                    .list_subagents(Some(&session_id), status, limit),
            ),
            WorkOperation::SubagentStart { id } => {
                work_result(self.service.start_subagent(&id, actor))
            }
            WorkOperation::SubagentComplete {
                id,
                child_run_id,
                output,
            } => work_result(
                self.service
                    .complete_subagent(&id, &child_run_id, &output, actor),
            ),
            WorkOperation::SubagentStop { id, status, error } => {
                work_result(self.service.stop_subagent(&id, status, &error, actor))
            }
            WorkOperation::SubagentRequeue { id } => {
                work_result(self.service.requeue_subagent(&id, actor))
            }
        }?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}
