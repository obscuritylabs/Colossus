use super::*;

/// Reusable application service implementing the durable model/tool loop.
pub struct AgentService {
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) provider: Arc<dyn ModelProvider>,
    pub(super) tools: Arc<dyn ToolRegistry>,
    pub(super) executor: Arc<dyn ToolExecutor>,
    pub(super) sessions: Arc<dyn SessionRepository>,
    pub(super) context_preparer: Option<Arc<dyn ContextPreparer>>,
    pub(super) provenance: Option<Arc<dyn colossus_ports::RunProvenanceProvider>>,
}

impl AgentService {
    /// Compose the service from ports; no interface logic is accepted here.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        provider: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
        executor: Arc<dyn ToolExecutor>,
        sessions: Arc<dyn SessionRepository>,
    ) -> Self {
        Self {
            journal,
            provider,
            tools,
            executor,
            sessions,
            context_preparer: None,
            provenance: None,
        }
    }

    /// Attach the shared durable context boundary used before every provider turn.
    pub fn with_context_preparer(mut self, preparer: Arc<dyn ContextPreparer>) -> Self {
        self.context_preparer = Some(preparer);
        self
    }

    /// Attach immutable host catalog evidence to each run and its effects.
    #[must_use]
    pub fn with_run_provenance(
        mut self,
        provider: Arc<dyn colossus_ports::RunProvenanceProvider>,
    ) -> Self {
        self.provenance = Some(provider);
        self
    }

    /// Execute one durable bounded run.
    pub async fn run(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_in_session(role, instructions, prompt, max_turns, None)
            .await
    }

    /// Execute a run attached to an exact existing session, or create a new session.
    pub async fn run_in_session(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope::default(),
            terminal_actor(),
            None,
            None,
        )
        .await
    }

    /// Execute a run with declarative active-skill lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                ..RunScope::default()
            },
            terminal_actor(),
            None,
            None,
        )
        .await
    }

    /// Execute a skilled run and forward ordered policy-released run events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                ..RunScope::default()
            },
            terminal_actor(),
            Some(observer),
            None,
        )
        .await
    }

    /// Execute a skilled run with ordered events and cooperative cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills_stream_controlled(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        self.run_in_session_with_mode_stream_controlled(
            AgentRunMode::Execute,
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            active_skills,
            false,
            observer,
            control,
        )
        .await
    }

    /// Execute one typed run mode with ordered events and cooperative cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_mode_stream_controlled(
        &self,
        mode: AgentRunMode,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        include_provider_response_diagnostics: bool,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        let prompt = ModelContent::from(prompt);
        self.run_in_session_with_mode_stream_controlled_content(
            mode,
            role,
            instructions,
            &prompt,
            max_turns,
            requested_session_id,
            active_skills,
            include_provider_response_diagnostics,
            observer,
            control,
        )
        .await
    }

    /// Execute one typed local run mode with ordered multipart user content.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_mode_stream_controlled_content(
        &self,
        mode: AgentRunMode,
        role: &str,
        instructions: &str,
        prompt: &ModelContent,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        include_provider_response_diagnostics: bool,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        match self
            .run_with_lineage_content(
                role,
                instructions,
                prompt,
                max_turns,
                requested_session_id,
                RunScope {
                    active_skills,
                    mode,
                    include_provider_response_diagnostics,
                    ..RunScope::default()
                },
                terminal_actor(),
                Some(observer),
                Some(control),
            )
            .await
        {
            Ok(result) => Ok(AgentRunOutcome::Completed { result }),
            Err(AgentError::Cancelled { result }) => {
                Ok(AgentRunOutcome::Cancelled { result: *result })
            }
            Err(error) => Err(error),
        }
    }

    /// Execute a trusted local run with explicit provider response diagnostics.
    ///
    /// The diagnostic remains typed on a terminal provider error. Durable run events and
    /// ordinary error display stay body-free.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills_stream_controlled_with_provider_diagnostics(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        self.run_in_session_with_mode_stream_controlled(
            AgentRunMode::Execute,
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            active_skills,
            true,
            observer,
            control,
        )
        .await
    }

    /// Execute a skilled run for one immutable authenticated initiator.
    ///
    /// Interfaces must construct the actor from authenticated caller context. The actor
    /// is used for session creation, the user message, and request-preparation audit
    /// events; model and tool activity retain their own provenance.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_in_session_with_skills_stream_controlled_as(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        initiator: Actor,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        match self
            .run_with_lineage(
                role,
                instructions,
                prompt,
                max_turns,
                requested_session_id,
                RunScope {
                    active_skills,
                    ..RunScope::default()
                },
                initiator,
                Some(observer),
                Some(control),
            )
            .await
        {
            Ok(result) => Ok(AgentRunOutcome::Completed { result }),
            Err(AgentError::Cancelled { result }) => {
                Ok(AgentRunOutcome::Cancelled { result: *result })
            }
            Err(error) => Err(error),
        }
    }

    /// Execute a public application run with server-assigned durable lineage.
    ///
    /// The caller must allocate and persist the public run before invoking this method.
    /// A server-assigned session may be created exactly once when `create_session` is
    /// true; an explicitly requested session must already exist.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_public_with_skills_stream_controlled(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        run_id: &str,
        session_id: &str,
        create_session: bool,
        active_skills: &[String],
        allowed_tools: &[String],
        plan_mode: bool,
        initiator: Actor,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        self.run_public_with_mode_and_skills_stream_controlled(
            role,
            instructions,
            prompt,
            max_turns,
            run_id,
            session_id,
            create_session,
            active_skills,
            allowed_tools,
            if plan_mode {
                AgentRunMode::Plan(PlanDraftTarget::Create)
            } else {
                AgentRunMode::Execute
            },
            None,
            None,
            initiator,
            observer,
            control,
        )
        .await
    }

    /// Execute a typed public application mode with server-assigned durable lineage.
    ///
    /// The Plan target is resolved by the trusted public runtime adapter. Public
    /// callers never supply a Plan identifier directly to the agent engine.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_public_with_mode_and_skills_stream_controlled(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        run_id: &str,
        session_id: &str,
        create_session: bool,
        active_skills: &[String],
        allowed_tools: &[String],
        mode: AgentRunMode,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        initiator: Actor,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        let prompt = ModelContent::from(prompt);
        self.run_public_with_mode_and_skills_stream_controlled_content(
            role,
            instructions,
            &prompt,
            max_turns,
            run_id,
            session_id,
            create_session,
            active_skills,
            allowed_tools,
            mode,
            end_user_id,
            remote_trace_context,
            initiator,
            observer,
            control,
        )
        .await
    }

    /// Execute a typed public application mode with ordered multipart user content.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_public_with_mode_and_skills_stream_controlled_content(
        &self,
        role: &str,
        instructions: &str,
        prompt: &ModelContent,
        max_turns: u16,
        run_id: &str,
        session_id: &str,
        create_session: bool,
        active_skills: &[String],
        allowed_tools: &[String],
        mode: AgentRunMode,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        initiator: Actor,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        match self
            .run_with_lineage_content(
                role,
                instructions,
                prompt,
                max_turns,
                Some(session_id),
                RunScope {
                    requested_run_id: Some(run_id),
                    active_skills,
                    allowed_tools: Some(allowed_tools),
                    mode,
                    end_user_id,
                    remote_trace_context,
                    create_requested_session: create_session,
                    ..RunScope::default()
                },
                initiator,
                Some(observer),
                Some(control),
            )
            .await
        {
            Ok(result) => Ok(AgentRunOutcome::Completed { result }),
            Err(AgentError::Cancelled { result }) => {
                Ok(AgentRunOutcome::Cancelled { result: *result })
            }
            Err(error) => Err(error),
        }
    }

    /// Execute without implementation/external mutation; local planning writes remain available.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_in_session_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
    ) -> Result<AgentRunResult, AgentError> {
        self.run_plan_target_in_session_with_skills(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            active_skills,
            PlanDraftTarget::Create,
        )
        .await
    }

    /// Execute one exact Plan Mode create or update target without implementation effects.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_target_in_session_with_skills(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        target: PlanDraftTarget,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                mode: AgentRunMode::Plan(target),
                ..RunScope::default()
            },
            terminal_actor(),
            None,
            None,
        )
        .await
    }

    /// Execute a planning run and forward ordered policy-released events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_in_session_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_plan_target_in_session_with_skills_stream(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            active_skills,
            PlanDraftTarget::Create,
            observer,
        )
        .await
    }

    /// Execute one exact Plan Mode target and forward ordered released events.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_plan_target_in_session_with_skills_stream(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        requested_session_id: Option<&str>,
        active_skills: &[String],
        target: PlanDraftTarget,
        observer: &mut dyn RunEventObserver,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                mode: AgentRunMode::Plan(target),
                ..RunScope::default()
            },
            terminal_actor(),
            Some(observer),
            None,
        )
        .await
    }

    /// Execute one already-consumed approved plan with fixed run and plan lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_approved_plan(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        session_id: &str,
        plan_id: &str,
        run_id: &str,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            Some(session_id),
            RunScope {
                requested_run_id: Some(run_id),
                plan_id: Some(plan_id),
                ..RunScope::default()
            },
            terminal_actor(),
            None,
            None,
        )
        .await
    }

    /// Execute one consumed approved plan with ordered events and cooperative cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_approved_plan_stream_controlled(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        session_id: &str,
        plan_id: &str,
        run_id: &str,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        match self
            .run_with_lineage(
                role,
                instructions,
                prompt,
                max_turns,
                Some(session_id),
                RunScope {
                    requested_run_id: Some(run_id),
                    plan_id: Some(plan_id),
                    end_user_id,
                    remote_trace_context,
                    ..RunScope::default()
                },
                terminal_actor(),
                Some(observer),
                Some(control),
            )
            .await
        {
            Ok(result) => Ok(AgentRunOutcome::Completed { result }),
            Err(AgentError::Cancelled { result }) => {
                Ok(AgentRunOutcome::Cancelled { result: *result })
            }
            Err(error) => Err(error),
        }
    }

    /// Execute one goal-mode iteration with goal-only tools and durable lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_goal_iteration(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        session_id: &str,
        goal_id: &str,
        plan_id: Option<&str>,
    ) -> Result<AgentRunResult, AgentError> {
        Box::pin(self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            Some(session_id),
            RunScope {
                goal_id: Some(goal_id),
                plan_id,
                ..RunScope::default()
            },
            terminal_actor(),
            None,
            None,
        ))
        .await
    }

    /// Execute one Goal Mode iteration with ordered events and cooperative cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_goal_iteration_stream_controlled(
        &self,
        run_id: &str,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        session_id: &str,
        goal_id: &str,
        plan_id: Option<&str>,
        end_user_id: Option<&str>,
        remote_trace_context: Option<&colossus_contracts::RemoteTraceContext>,
        observer: &mut dyn RunEventObserver,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, AgentError> {
        match Box::pin(self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            Some(session_id),
            RunScope {
                requested_run_id: Some(run_id),
                goal_id: Some(goal_id),
                plan_id,
                end_user_id,
                remote_trace_context,
                ..RunScope::default()
            },
            terminal_actor(),
            Some(observer),
            Some(control),
        ))
        .await
        {
            Ok(result) => Ok(AgentRunOutcome::Completed { result }),
            Err(AgentError::Cancelled { result }) => {
                Ok(AgentRunOutcome::Cancelled { result: *result })
            }
            Err(error) => Err(error),
        }
    }

    /// Execute one durable child-agent job without exposing nested delegation.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_subagent(
        &self,
        role: &str,
        instructions: &str,
        task: &str,
        max_turns: u16,
        child_session_id: &str,
        subagent_id: &str,
        allowed_tools: Option<&[String]>,
    ) -> Result<AgentRunResult, AgentError> {
        self.run_subagent_with_skills(
            role,
            instructions,
            task,
            max_turns,
            child_session_id,
            subagent_id,
            allowed_tools,
            &[],
        )
        .await
    }

    /// Execute a child with its parent's exact declarative skill selections.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_subagent_with_skills(
        &self,
        role: &str,
        instructions: &str,
        task: &str,
        max_turns: u16,
        child_session_id: &str,
        subagent_id: &str,
        allowed_tools: Option<&[String]>,
        active_skills: &[String],
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            task,
            max_turns,
            Some(child_session_id),
            RunScope {
                subagent_id: Some(subagent_id),
                allowed_tools,
                active_skills,
                ..RunScope::default()
            },
            terminal_actor(),
            None,
            None,
        )
        .await
    }

    /// Execute one policy-authorized declarative workflow agent step.
    ///
    /// The workflow effect itself is authorized before this method is entered. Every
    /// provider turn and tool invoked by the model still crosses its ordinary gateway
    /// with immutable workflow lineage, so this is not an alternate effect path.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_workflow_step(
        &self,
        role: &str,
        instructions: &str,
        prompt: &str,
        max_turns: u16,
        workflow_id: &str,
        workflow_hash: &str,
        step_id: &str,
        attempt: u32,
        allowed_tools: &[String],
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            None,
            RunScope {
                workflow_id: Some(workflow_id),
                workflow_hash: Some(workflow_hash),
                step_id: Some(step_id),
                attempt: Some(attempt),
                allowed_tools: Some(allowed_tools),
                ..RunScope::default()
            },
            Actor {
                actor_type: ActorType::Workflow,
                id: workflow_id.into(),
            },
            None,
            None,
        )
        .await
    }
}

fn terminal_actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "terminal-user".into(),
    }
}
