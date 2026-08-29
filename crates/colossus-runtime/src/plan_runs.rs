//! Plan creation, approval, consumption, and execution orchestration.

use super::*;

impl Runtime {
    /// Execute structurally read-only Plan Mode with only inspection, task, and plan tools.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: Option<&str>,
        explicit_skills: &[String],
        sticky_skills: &[String],
    ) -> Result<AgentRunResult, RuntimeError> {
        self.run_plan_target_with_skills(
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
            PlanDraftTarget::Create,
        )
        .await
    }

    /// Execute one exact non-interactive Plan Mode create or update target.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_target_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: Option<&str>,
        explicit_skills: &[String],
        sticky_skills: &[String],
        target: PlanDraftTarget,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.validate_plan_target(session_id, &target)?;
        let runtime_mode = plan_mode_instructions(&target);
        let prepared = self.prepare_agent_instructions(instructions, &runtime_mode)?;
        let composition = self.skill_composer.compose(
            &prepared.base_text,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let instructions = prepared.complete_composed_base(&composition.instructions);
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();
        scope_instruction_snapshot(
            prepared.snapshot,
            self.agent.run_plan_target_in_session_with_skills(
                role,
                &instructions,
                prompt,
                max_turns.unwrap_or(self.agent_max_turns),
                session_id,
                &active,
                target,
            ),
        )
        .await
        .map_err(Into::into)
    }

    /// Execute Plan Mode while forwarding policy-released provider events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: Option<&str>,
        explicit_skills: &[String],
        sticky_skills: &[String],
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.run_plan_target_with_skills_stream(
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
            PlanDraftTarget::Create,
            observer,
        )
        .await
    }

    /// Execute one exact Plan Mode target while forwarding released provider events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_target_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: Option<&str>,
        explicit_skills: &[String],
        sticky_skills: &[String],
        target: PlanDraftTarget,
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.validate_plan_target(session_id, &target)?;
        let runtime_mode = plan_mode_instructions(&target);
        let prepared = self.prepare_agent_instructions(instructions, &runtime_mode)?;
        let composition = self.skill_composer.compose(
            &prepared.base_text,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let instructions = prepared.complete_composed_base(&composition.instructions);
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();
        scope_instruction_snapshot(
            prepared.snapshot,
            Box::pin(self.agent.run_plan_target_in_session_with_skills_stream(
                role,
                &instructions,
                prompt,
                max_turns.unwrap_or(self.agent_max_turns),
                session_id,
                &active,
                target,
                observer,
            )),
        )
        .await
        .map_err(Into::into)
    }

    /// Atomically consume and execute one approved plan through a normal agent run.
    pub async fn run_approved_plan(
        &self,
        role: &str,
        plan_id: &str,
        max_turns: Option<u16>,
    ) -> Result<AgentRunResult, RuntimeError> {
        let prepared = self.prepare_agent_instructions("", APPROVED_PLAN_INSTRUCTIONS)?;
        let plan = self
            .work
            .get_plan(plan_id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
        if plan.status != PlanStatus::Approved {
            return Err(RuntimeError::Config(
                "plan execution requires one approved plan".into(),
            ));
        }
        let prompt = goal_objective_from_plan(&plan);
        let run_id = Uuid::now_v7().to_string();
        let consumed: PlanRecord = serde_json::from_value(
            self.execute_work_operation(WorkOperation::PlanExecute {
                id: plan.id.clone(),
                run_id: run_id.clone(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        scope_instruction_snapshot(
            prepared.snapshot,
            Box::pin(
                self.run_with_subagent_scheduling(self.agent.run_approved_plan(
                    role,
                    &prepared.text,
                    &prompt,
                    max_turns.unwrap_or(self.agent_max_turns),
                    &consumed.session_id,
                    &consumed.id,
                    &run_id,
                )),
            ),
        )
        .await
    }

    /// Execute one exact approved Plan revision with streaming and cooperative cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_plan_stream_controlled(
        &self,
        role: &str,
        expected_session_id: &str,
        plan_id: &str,
        revision: u64,
        strategy: PlanExecutionStrategy,
        max_turns: Option<u16>,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<PlanExecutionOutcome, RuntimeError> {
        let captured = self.capture_agent_instructions("")?;
        self.execute_plan_stream_controlled_with_run_id(
            role,
            expected_session_id,
            plan_id,
            revision,
            strategy,
            max_turns,
            captured,
            None,
            true,
            None,
            None,
            observer,
            control,
        )
        .await
    }

    /// Approve and consume one exact draft Plan revision as an already-durable public run.
    ///
    /// Direct execution consumes the Plan with the public run identity so emitted
    /// events remain bound to the durable caller-owned run. Goal execution retains
    /// its canonical per-iteration run identities under the outer public Goal run.
    #[allow(clippy::too_many_arguments)]
    pub async fn approve_and_execute_public_plan_stream_controlled(
        &self,
        role: &str,
        expected_session_id: &str,
        plan_id: &str,
        revision: u64,
        strategy: PlanExecutionStrategy,
        max_turns: Option<u16>,
        public_run_id: &str,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<PlanExecutionOutcome, RuntimeError> {
        let selected = self
            .work
            .get_plan(plan_id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
        validate_public_plan_execution_selection(&selected, expected_session_id, revision)?;
        if control.is_cancelled() {
            return Ok(PlanExecutionOutcome::CancelledBeforeStart { plan: selected });
        }
        let captured = self.capture_agent_instructions("")?;
        let approved = self
            .approve_plan_at_revision(expected_session_id, plan_id, revision)
            .await?;
        self.execute_plan_stream_controlled_with_run_id(
            role,
            expected_session_id,
            &approved.id,
            approved.revision,
            strategy,
            max_turns,
            captured,
            Some(public_run_id),
            false,
            end_user_id,
            remote_trace_context,
            observer,
            control,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_plan_stream_controlled_with_run_id(
        &self,
        role: &str,
        expected_session_id: &str,
        plan_id: &str,
        revision: u64,
        strategy: PlanExecutionStrategy,
        max_turns: Option<u16>,
        captured: CapturedAgentInstructions,
        public_run_id: Option<&str>,
        cancel_before_consumption: bool,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<PlanExecutionOutcome, RuntimeError> {
        let plan = self
            .work
            .get_plan(plan_id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
        validate_plan_execution_selection(&plan, expected_session_id, revision)?;
        if cancel_before_consumption && control.is_cancelled() {
            return Ok(PlanExecutionOutcome::CancelledBeforeStart { plan });
        }
        match strategy {
            PlanExecutionStrategy::Direct => {
                let prepared = captured.finalize(APPROVED_PLAN_INSTRUCTIONS);
                let prompt = goal_objective_from_plan(&plan);
                let run_id = public_run_id
                    .map(str::to_owned)
                    .unwrap_or_else(|| Uuid::now_v7().to_string());
                let committed = match self
                    .execute_work_operation(WorkOperation::PlanExecuteAtRevision {
                        id: plan.id.clone(),
                        expected_revision: revision,
                        run_id: run_id.clone(),
                    })
                    .await
                {
                    Ok(committed) => committed,
                    Err(error) => {
                        let Some(consumed) = self.persisted_consumed_plan(&plan, Some(&run_id))
                        else {
                            return Err(error);
                        };
                        return Ok(PlanExecutionOutcome::Direct {
                            plan: consumed,
                            terminal: ControlledAgentTerminal::Failed {
                                run_id,
                                message: bounded_execution_error(&error.to_string()),
                                outcome_unknown: error.outcome_unknown(),
                            },
                        });
                    }
                };
                let consumed: PlanRecord = match serde_json::from_value(committed.clone()) {
                    Ok(consumed) => consumed,
                    Err(error) => {
                        let evidence =
                            self.consumed_plan_evidence(&plan, Some(&run_id), &committed);
                        return Ok(PlanExecutionOutcome::Direct {
                            plan: evidence,
                            terminal: ControlledAgentTerminal::Failed {
                                run_id,
                                message: bounded_execution_error(&format!(
                                    "consumed Plan result was invalid: {error}"
                                )),
                                outcome_unknown: false,
                            },
                        });
                    }
                };
                let run = Box::pin(self.agent.run_approved_plan_stream_controlled(
                    role,
                    &prepared.text,
                    &prompt,
                    max_turns.unwrap_or(self.agent_max_turns),
                    &consumed.session_id,
                    &consumed.id,
                    &run_id,
                    end_user_id,
                    remote_trace_context,
                    observer,
                    control,
                ));
                let run = scope_instruction_snapshot(prepared.snapshot, run);
                tokio::pin!(run);
                let terminal = loop {
                    tokio::select! {
                        biased;
                        _ = self.subagent_notify.notified() => {
                            if let Err(error) = self.drain_subagents().await {
                                break ControlledAgentTerminal::Failed {
                                    run_id: run_id.clone(),
                                    message: bounded_execution_error(&error.to_string()),
                                    outcome_unknown: error.outcome_unknown(),
                                };
                            }
                        }
                        result = &mut run => {
                            break match result {
                                Ok(AgentRunOutcome::Completed { result }) => {
                                    ControlledAgentTerminal::Completed { result }
                                }
                                Ok(AgentRunOutcome::Cancelled { result }) => {
                                    ControlledAgentTerminal::Cancelled { result }
                                }
                                Err(error) => ControlledAgentTerminal::Failed {
                                    run_id: run_id.clone(),
                                    message: bounded_execution_error(&error.to_string()),
                                    outcome_unknown: error.outcome_unknown(),
                                },
                            };
                        }
                    }
                };
                Ok(PlanExecutionOutcome::Direct {
                    plan: consumed.clone(),
                    terminal,
                })
            }
            PlanExecutionStrategy::Goal { max_iterations } => {
                if !(1..=50).contains(&max_iterations) {
                    return Err(RuntimeError::Config(
                        "goal iterations must be in 1..=50".into(),
                    ));
                }
                let objective = goal_objective_from_plan(&plan);
                let committed = match self
                    .execute_work_operation(WorkOperation::GoalCreate {
                        session_id: plan.session_id.clone(),
                        objective: objective.clone(),
                        iteration_budget: max_iterations,
                        source_plan_id: Some(plan.id.clone()),
                        source_plan_revision: Some(revision),
                    })
                    .await
                {
                    Ok(committed) => committed,
                    Err(error) => {
                        let Some(consumed) = self.persisted_consumed_plan(&plan, None) else {
                            return Err(error);
                        };
                        let goal = self.goal_evidence(
                            &plan,
                            &consumed,
                            &objective,
                            max_iterations,
                            &Value::Null,
                        );
                        return Ok(PlanExecutionOutcome::Goal {
                            plan: consumed,
                            terminal: failed_goal_outcome(
                                goal,
                                Vec::new(),
                                None,
                                bounded_execution_error(&error.to_string()),
                                error.outcome_unknown(),
                                0.0,
                            ),
                        });
                    }
                };
                let (goal, consumed) =
                    match serde_json::from_value::<GoalCreationResult>(committed.clone()) {
                        Ok(GoalCreationResult {
                            goal,
                            consumed_plan: Some(consumed),
                        }) => (goal, consumed),
                        Ok(GoalCreationResult {
                            goal,
                            consumed_plan: None,
                        }) => {
                            let consumed =
                                self.consumed_plan_evidence(&plan, Some(&goal.id), &committed);
                            return Ok(PlanExecutionOutcome::Goal {
                                plan: consumed,
                                terminal: failed_goal_outcome(
                                    goal,
                                    Vec::new(),
                                    None,
                                    "approved Plan Goal handoff omitted its consumed Plan".into(),
                                    false,
                                    0.0,
                                ),
                            });
                        }
                        Err(error) => {
                            let expected_goal_id =
                                committed.pointer("/goal/id").and_then(Value::as_str);
                            let consumed =
                                self.consumed_plan_evidence(&plan, expected_goal_id, &committed);
                            let goal = self.goal_evidence(
                                &plan,
                                &consumed,
                                &objective,
                                max_iterations,
                                &committed,
                            );
                            return Ok(PlanExecutionOutcome::Goal {
                                plan: consumed,
                                terminal: failed_goal_outcome(
                                    goal,
                                    Vec::new(),
                                    None,
                                    bounded_execution_error(&format!(
                                        "consumed Goal result was invalid: {error}"
                                    )),
                                    false,
                                    0.0,
                                ),
                            });
                        }
                    };
                let mode = approved_goal_mode_instructions(&goal);
                let prepared = captured.finalize(&mode);
                let terminal = self
                    .run_existing_goal_stream_controlled(
                        role,
                        goal,
                        prepared,
                        end_user_id,
                        remote_trace_context,
                        observer,
                        control,
                    )
                    .await;
                Ok(PlanExecutionOutcome::Goal {
                    plan: consumed,
                    terminal,
                })
            }
        }
    }
}
