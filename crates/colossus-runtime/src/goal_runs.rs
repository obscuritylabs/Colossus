//! Bounded Goal execution and durable resume orchestration.

use super::*;

impl Runtime {
    /// Resume the remaining iteration budget of one active durable Goal.
    pub async fn resume_goal_stream_controlled(
        &self,
        role: &str,
        expected_session_id: &str,
        goal_id: &str,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<GoalRunOutcome, RuntimeError> {
        let goal = self
            .work
            .get_goal(goal_id)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {goal_id}")))?;
        validate_goal_resume_selection(&goal, expected_session_id)?;
        let mode = goal_mode_instructions(&goal);
        let prepared = self.prepare_agent_instructions("", &mode)?;
        Ok(self
            .run_existing_goal_stream_controlled(
                role, goal, prepared, None, None, observer, control,
            )
            .await)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_existing_goal_stream_controlled(
        &self,
        role: &str,
        goal: GoalRecord,
        prepared: PreparedAgentInstructions,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> GoalRunOutcome {
        let started = Instant::now();
        let instructions = prepared.text.clone();
        let mut iterations = Vec::new();
        let first_iteration = goal.iterations_completed.saturating_add(1);
        for iteration in first_iteration..=goal.iteration_budget {
            let current = match self.work.get_goal(&goal.id) {
                Ok(Some(current)) => current,
                Ok(None) => {
                    return failed_goal_outcome(
                        goal,
                        iterations,
                        None,
                        "the active goal disappeared".into(),
                        false,
                        started.elapsed().as_secs_f64(),
                    );
                }
                Err(error) => {
                    return failed_goal_outcome(
                        goal,
                        iterations,
                        None,
                        bounded_execution_error(&error.to_string()),
                        matches!(error, StoreError::OutcomeUnknown(_)),
                        started.elapsed().as_secs_f64(),
                    );
                }
            };
            if current.status != GoalStatus::Active {
                return GoalRunOutcome::Completed {
                    result: goal_run_result(current, iterations, started.elapsed().as_secs_f64()),
                };
            }
            if control.is_cancelled() {
                return cancelled_goal_outcome(
                    current,
                    iterations,
                    None,
                    started.elapsed().as_secs_f64(),
                );
            }
            let prompt = if iteration == 1 {
                format!("Start Goal Mode for {}: {}", current.id, current.objective)
            } else {
                format!(
                    "Continue Goal Mode for {}. Objective: {}. Use session history and update the goal only when complete or blocked.",
                    current.id, current.objective
                )
            };
            let run_id = Uuid::now_v7().to_string();
            if let Err(error) = self
                .execute_work_operation(WorkOperation::GoalIteration {
                    id: current.id.clone(),
                })
                .await
            {
                let latest = self
                    .work
                    .get_goal(&current.id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| current.clone());
                return failed_goal_outcome(
                    latest,
                    iterations,
                    Some(run_id),
                    bounded_execution_error(&error.to_string()),
                    error.outcome_unknown(),
                    started.elapsed().as_secs_f64(),
                );
            }
            let (events, receiver) = mpsc::channel(64);
            let mut buffered_observer = self.buffered_run_observer(events.clone());
            let run = self.agent.run_goal_iteration_stream_controlled(
                &run_id,
                role,
                &instructions,
                &prompt,
                self.agent_max_turns,
                &current.session_id,
                &current.id,
                current.source_plan_id.as_deref(),
                end_user_id,
                remote_trace_context,
                &mut buffered_observer,
                control,
            );
            let run = scope_instruction_snapshot(prepared.snapshot.clone(), run);
            let outcome = self
                .forward_run_with_subagent_scheduling(run, events, receiver, observer)
                .await;
            let result = match outcome {
                Ok(AgentRunOutcome::Completed { result }) => result,
                Ok(AgentRunOutcome::Cancelled { result }) => {
                    let latest = self
                        .work
                        .get_goal(&current.id)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| current.clone());
                    return cancelled_goal_outcome(
                        latest,
                        iterations,
                        Some(Box::new(result)),
                        started.elapsed().as_secs_f64(),
                    );
                }
                Err(error) => {
                    let latest = self
                        .work
                        .get_goal(&current.id)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| current.clone());
                    return failed_goal_outcome(
                        latest,
                        iterations,
                        Some(run_id.clone()),
                        bounded_execution_error(&error.to_string()),
                        error.outcome_unknown(),
                        started.elapsed().as_secs_f64(),
                    );
                }
            };
            iterations.push(GoalIterationResult {
                iteration,
                run_id: result.run_id,
                output: result.output,
                event_count: result.event_count,
                elapsed_seconds: result.elapsed_seconds,
            });
        }
        let final_goal = self.work.get_goal(&goal.id).ok().flatten().unwrap_or(goal);
        GoalRunOutcome::Completed {
            result: goal_run_result(final_goal, iterations, started.elapsed().as_secs_f64()),
        }
    }

    /// Run bounded autonomous iterations using the normal agent, session, policy, and tools.
    pub async fn run_goal(
        &self,
        role: &str,
        objective: &str,
        session_id: &str,
        max_iterations: u16,
        source_plan_id: Option<&str>,
    ) -> Result<GoalRunResult, RuntimeError> {
        if !(1..=50).contains(&max_iterations) {
            return Err(RuntimeError::Config(
                "goal iterations must be in 1..=50".into(),
            ));
        }
        let captured = self.capture_agent_instructions("")?;
        let started = Instant::now();
        let mut source_plan_revision = None;
        let objective = if let Some(plan_id) = source_plan_id {
            let plan = self
                .work
                .get_plan(plan_id)?
                .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
            if plan.session_id != session_id || plan.status != PlanStatus::Approved {
                return Err(RuntimeError::Config(
                    "goal handoff requires an approved same-session plan".into(),
                ));
            }
            source_plan_revision = Some(plan.revision);
            goal_objective_from_plan(&plan)
        } else {
            objective.into()
        };
        let created: GoalCreationResult = serde_json::from_value(
            self.execute_work_operation(WorkOperation::GoalCreate {
                session_id: session_id.into(),
                objective,
                iteration_budget: max_iterations,
                source_plan_id: source_plan_id.map(str::to_owned),
                source_plan_revision,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        let goal = created.goal;
        let mode = goal_mode_instructions(&goal);
        let prepared = captured.finalize(&mode);
        let instructions = prepared.text.clone();
        let mut iterations = Vec::new();
        for iteration in 1..=max_iterations {
            let current = self
                .work
                .get_goal(&goal.id)?
                .ok_or_else(|| StoreError::NotFound(format!("goal {}", goal.id)))?;
            if current.status != GoalStatus::Active {
                break;
            }
            let prompt = if iteration == 1 {
                format!("Start Goal Mode for {}: {}", current.id, current.objective)
            } else {
                format!(
                    "Continue Goal Mode for {}. Objective: {}. Use session history and update the goal only when complete or blocked.",
                    current.id, current.objective
                )
            };
            self.execute_work_operation(WorkOperation::GoalIteration {
                id: current.id.clone(),
            })
            .await?;
            let result = self
                .run_with_subagent_scheduling(scope_instruction_snapshot(
                    prepared.snapshot.clone(),
                    self.agent.run_goal_iteration(
                        role,
                        &instructions,
                        &prompt,
                        self.agent_max_turns,
                        session_id,
                        &current.id,
                        current.source_plan_id.as_deref(),
                    ),
                ))
                .await?;
            iterations.push(GoalIterationResult {
                iteration,
                run_id: result.run_id,
                output: result.output,
                event_count: result.event_count,
                elapsed_seconds: result.elapsed_seconds,
            });
        }
        let final_goal = self
            .work
            .get_goal(&goal.id)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {}", goal.id)))?;
        Ok(GoalRunResult {
            iteration_budget_exhausted: final_goal.status == GoalStatus::Active
                && final_goal.iterations_completed >= final_goal.iteration_budget,
            goal: final_goal,
            iterations,
            elapsed_seconds: started.elapsed().as_secs_f64(),
        })
    }
}
