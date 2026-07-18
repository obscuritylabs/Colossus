use super::*;

impl Runtime {
    pub(super) async fn run_with_subagent_scheduling<F>(
        &self,
        run: F,
    ) -> Result<AgentRunResult, RuntimeError>
    where
        F: Future<Output = Result<AgentRunResult, AgentError>>,
    {
        tokio::pin!(run);
        loop {
            tokio::select! {
                biased;
                _ = self.subagent_notify.notified() => {
                    self.drain_subagents().await?;
                }
                result = &mut run => return result.map_err(Into::into),
            }
        }
    }

    /// Execute the shared durable bounded provider/tool loop.
    pub async fn run_model(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.run_model_with_skills(role, instructions, prompt, None, None, &[], &[])
            .await
    }

    /// Execute the shared loop with a caller-selected bounded turn limit.
    pub async fn run_model_with_max_turns(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.run_model_with_skills(role, instructions, prompt, Some(max_turns), None, &[], &[])
            .await
    }

    /// Execute a run while restoring and appending one exact durable session.
    pub async fn run_model_in_session(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: &str,
    ) -> Result<AgentRunResult, RuntimeError> {
        self.run_model_with_skills(
            role,
            instructions,
            prompt,
            max_turns,
            Some(session_id),
            &[],
            &[],
        )
        .await
    }

    /// Execute a normal run with explicit and sticky declarative skill activation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_model_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: Option<&str>,
        explicit_skills: &[String],
        sticky_skills: &[String],
    ) -> Result<AgentRunResult, RuntimeError> {
        let composition = self.skill_composer.compose(
            instructions,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();
        self.run_with_subagent_scheduling(self.agent.run_in_session_with_skills(
            role,
            &composition.instructions,
            prompt,
            max_turns.unwrap_or(self.agent_max_turns),
            session_id,
            &active,
        ))
        .await
    }

    /// Execute a normal run and forward only policy-released provider events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_model_with_skills_stream(
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
        let composition = self.skill_composer.compose(
            instructions,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();
        self.run_with_subagent_scheduling(self.agent.run_in_session_with_skills_stream(
            role,
            &composition.instructions,
            prompt,
            max_turns.unwrap_or(self.agent_max_turns),
            session_id,
            &active,
            observer,
        ))
        .await
    }

    /// Execute a normal run with ordered events and cooperative cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_model_with_skills_stream_controlled(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: Option<u16>,
        session_id: Option<&str>,
        explicit_skills: &[String],
        sticky_skills: &[String],
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, RuntimeError> {
        let composition = self.skill_composer.compose(
            instructions,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();

        let run = self.agent.run_in_session_with_skills_stream_controlled(
            role,
            &composition.instructions,
            prompt,
            max_turns.unwrap_or(self.agent_max_turns),
            session_id,
            &active,
            observer,
            control,
        );
        tokio::pin!(run);
        loop {
            tokio::select! {
                biased;
                _ = self.subagent_notify.notified() => {
                    self.drain_subagents().await?;
                }
                result = &mut run => return result.map_err(Into::into),
            }
        }
    }

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
        let instructions = format!(
            "{instructions}\n\nYou are Colossus operating in Plan Mode. Inspect context and create durable tasks or a structured draft with plan.create when useful. Do not write files, apply patches, run commands, delegate work, alter decisions or memories, approve plans, or claim implementation is complete."
        );
        let composition = self.skill_composer.compose(
            &instructions,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();
        self.agent
            .run_plan_in_session_with_skills(
                role,
                &composition.instructions,
                prompt,
                max_turns.unwrap_or(self.agent_max_turns),
                session_id,
                &active,
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
        let instructions = format!(
            "{instructions}\n\nYou are Colossus operating in Plan Mode. Inspect context and create durable tasks or a structured draft with plan.create when useful. Do not write files, apply patches, run commands, delegate work, alter decisions or memories, approve plans, or claim implementation is complete."
        );
        let composition = self.skill_composer.compose(
            &instructions,
            prompt,
            explicit_skills,
            sticky_skills,
            self.skills_enabled,
            &self.tools.list_specs(),
        )?;
        let active = composition
            .active_skills
            .iter()
            .map(|skill| skill.name.clone())
            .collect::<Vec<_>>();
        self.agent
            .run_plan_in_session_with_skills_stream(
                role,
                &composition.instructions,
                prompt,
                max_turns.unwrap_or(self.agent_max_turns),
                session_id,
                &active,
                observer,
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
        self.run_with_subagent_scheduling(self.agent.run_approved_plan(
                role,
                "Execute the canonical approved plan using normal tools and policy. Preserve plan lineage and do not expand its scope.",
                &prompt,
                max_turns.unwrap_or(self.agent_max_turns),
                &consumed.session_id,
                &consumed.id,
                &run_id,
            ))
            .await
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
        let started = Instant::now();
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
            goal_objective_from_plan(&plan)
        } else {
            objective.into()
        };
        let goal: GoalRecord = serde_json::from_value(
            self.execute_work_operation(WorkOperation::GoalCreate {
                session_id: session_id.into(),
                objective,
                iteration_budget: max_iterations,
                source_plan_id: source_plan_id.map(str::to_owned),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        let instructions = format!(
            "You are Colossus running bounded Goal Mode.\n\nActive goal id: {}\nObjective: {}\n\nWork in bounded, useful steps using normal tools and policy. When genuinely finished, call goal.update with status complete and a concise summary. If meaningful progress requires user input or an external state change, call goal.update with status blocked and a reason. Otherwise leave the goal active for the next iteration.",
            goal.id, goal.objective
        );
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
            let result = self
                .run_with_subagent_scheduling(self.agent.run_goal_iteration(
                    role,
                    &instructions,
                    &prompt,
                    self.agent_max_turns,
                    session_id,
                    &current.id,
                    current.source_plan_id.as_deref(),
                ))
                .await?;
            iterations.push(GoalIterationResult {
                iteration,
                run_id: result.run_id,
                output: result.output,
                event_count: result.event_count,
                elapsed_seconds: result.elapsed_seconds,
            });
            self.execute_work_operation(WorkOperation::GoalIteration {
                id: current.id.clone(),
            })
            .await?;
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
