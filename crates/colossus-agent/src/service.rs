use super::*;

/// Reusable application service implementing the durable model/tool loop.
pub struct AgentService {
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) provider: Arc<dyn ModelProvider>,
    pub(super) tools: Arc<dyn ToolRegistry>,
    pub(super) executor: Arc<dyn ToolExecutor>,
    pub(super) sessions: Arc<dyn SessionRepository>,
    pub(super) context_preparer: Option<Arc<dyn ContextPreparer>>,
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
        }
    }

    /// Attach the shared durable context boundary used before every provider turn.
    pub fn with_context_preparer(mut self, preparer: Arc<dyn ContextPreparer>) -> Self {
        self.context_preparer = Some(preparer);
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
        self.run_in_session_with_skills_stream_controlled_as(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            active_skills,
            terminal_actor(),
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
            Err(AgentError::Cancelled { result }) => Ok(AgentRunOutcome::Cancelled { result }),
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
        match self
            .run_with_lineage(
                role,
                instructions,
                prompt,
                max_turns,
                Some(session_id),
                RunScope {
                    requested_run_id: Some(run_id),
                    active_skills,
                    allowed_tools: Some(allowed_tools),
                    plan_mode,
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
            Err(AgentError::Cancelled { result }) => Ok(AgentRunOutcome::Cancelled { result }),
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
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                plan_mode: true,
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
        self.run_with_lineage(
            role,
            instructions,
            prompt,
            max_turns,
            requested_session_id,
            RunScope {
                active_skills,
                plan_mode: true,
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
        self.run_with_lineage(
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
        )
        .await
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
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_lineage(
            role,
            instructions,
            task,
            max_turns,
            Some(child_session_id),
            RunScope {
                subagent_id: Some(subagent_id),
                ..RunScope::default()
            },
            terminal_actor(),
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
