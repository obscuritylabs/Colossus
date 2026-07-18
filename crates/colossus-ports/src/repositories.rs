use super::*;

/// Shared behavior for aggregate repositories reconstructed from events.
pub trait AggregateRepository: Send + Sync {
    /// Load the aggregate's current JSON projection.
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError>;

    /// List bounded aggregate projections.
    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError>;
}

/// Canonical event-sourced session and append-only message repository.
pub trait SessionRepository: Send + Sync {
    /// Create an empty durable session with a caller-supplied stable id.
    fn create_session(
        &self,
        id: &str,
        title: Option<&str>,
        actor: Actor,
    ) -> Result<SessionSummary, StoreError>;

    /// Reconstruct one session summary from canonical events.
    fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, StoreError>;

    /// List recent reconstructed sessions, newest first.
    fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError>;

    /// Append one message using optimistic session-stream concurrency.
    fn append_message(
        &self,
        session_id: &str,
        run_id: &str,
        message: colossus_contracts::ModelMessage,
        actor: Actor,
    ) -> Result<SessionMessage, StoreError>;

    /// Reconstruct every append-only message in sequence order.
    fn list_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, StoreError>;

    /// Return a bounded chronological page ending before an optional sequence.
    fn list_messages_page(
        &self,
        session_id: &str,
        before_sequence: Option<u64>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<SessionMessagePage, StoreError> {
        let messages = self.list_messages(session_id)?;
        let upper = before_sequence.unwrap_or(u64::MAX);
        let mut page = Vec::new();
        let mut bytes = 0_usize;
        for message in messages
            .iter()
            .rev()
            .filter(|message| message.sequence < upper)
        {
            let encoded = serde_json::to_vec(message)
                .map_err(|error| StoreError::Adapter(error.to_string()))?;
            if encoded.len() > max_bytes {
                return Err(StoreError::Adapter(format!(
                    "session message {} exceeds the bounded page size",
                    message.sequence
                )));
            }
            if page.len() == limit.max(1) || (!page.is_empty() && bytes + encoded.len() > max_bytes)
            {
                break;
            }
            bytes = bytes.saturating_add(encoded.len());
            page.push(message.clone());
        }
        page.reverse();
        let before_sequence = page.first().map(|message| message.sequence);
        let has_more = before_sequence.is_some_and(|first| {
            messages
                .iter()
                .any(|message| message.sequence < first && message.sequence < upper)
        });
        Ok(SessionMessagePage {
            messages: page,
            before_sequence,
            has_more,
        })
    }
}

/// Canonical immutable context snapshots and explicit activation history.
pub trait ContextRepository: Send + Sync {
    /// Append and activate a new snapshot using session-stream concurrency.
    fn create(
        &self,
        snapshot: ContextSnapshot,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError>;

    /// Reconstruct every snapshot for a session in creation order.
    fn list(&self, session_id: &str) -> Result<Vec<ContextSnapshot>, StoreError>;

    /// Return the explicitly active snapshot, if any.
    fn active(&self, session_id: &str) -> Result<Option<ContextSnapshot>, StoreError>;

    /// Activate an existing snapshot without mutating or deleting later snapshots.
    fn activate(
        &self,
        session_id: &str,
        snapshot_id: &str,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError>;
}

/// Canonical event-sourced presentation preference repository.
pub trait PresentationRepository: Send + Sync {
    /// Reconstruct the current preference profile or defaults before its first mutation.
    fn load(&self) -> Result<TerminalPreferences, StoreError>;

    /// Append one complete replacement profile through optimistic concurrency.
    fn save(
        &self,
        preferences: TerminalPreferences,
        actor: Actor,
    ) -> Result<TerminalPreferences, StoreError>;

    /// Reconstruct the newest bounded terminal submissions in chronological order.
    fn list_history(&self, limit: usize) -> Result<Vec<String>, StoreError>;

    /// Append one encrypted terminal-history entry, deduplicating consecutive submissions.
    fn append_history(&self, entry: String, actor: Actor) -> Result<String, StoreError>;
}

/// Shared context preparation boundary used by every agent provider turn.
#[async_trait]
pub trait ContextPreparer: Send + Sync {
    /// Apply an active snapshot or create one when the configured budget requires it.
    async fn prepare(
        &self,
        session_id: &str,
        instructions: &str,
        messages: Vec<ModelMessage>,
        tools: &[ModelToolDefinition],
        context: ExecutionContext,
        force: bool,
    ) -> Result<PreparedContext, ContextError>;
}
/// Canonical task and key-decision lifecycle repository.
pub trait WorkRepository: Send + Sync {
    /// Create a new session-scoped task.
    fn create_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError>;

    /// Append the complete next task state after validating immutable identity fields.
    fn update_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError>;

    /// Reconstruct one task from canonical events.
    fn get_task(&self, id: &str) -> Result<Option<TaskRecord>, StoreError>;

    /// List bounded tasks with optional session and status filters.
    fn list_tasks(
        &self,
        session_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError>;

    /// Create a new active key decision.
    fn create_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError>;

    /// Append the complete next active key-decision state.
    fn update_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError>;

    /// Reconstruct one key decision from canonical events.
    fn get_decision(&self, id: &str) -> Result<Option<KeyDecision>, StoreError>;

    /// List bounded decisions with optional session and status filters.
    fn list_decisions(
        &self,
        session_id: Option<&str>,
        status: Option<DecisionStatus>,
        limit: usize,
    ) -> Result<Vec<KeyDecision>, StoreError>;

    /// Archive one active decision through a new immutable event.
    fn archive_decision(&self, id: &str, actor: Actor) -> Result<KeyDecision, StoreError>;

    /// Atomically supersede one active decision and create its replacement.
    fn supersede_decision(
        &self,
        id: &str,
        replacement: KeyDecision,
        actor: Actor,
    ) -> Result<(KeyDecision, KeyDecision), StoreError>;

    /// Create a new draft plan.
    fn create_plan(&self, plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError>;

    /// Append a validated draft edit or lifecycle transition.
    fn update_plan(&self, plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError>;

    /// Reconstruct one canonical plan.
    fn get_plan(&self, id: &str) -> Result<Option<PlanRecord>, StoreError>;

    /// List bounded plans with optional session and status filters.
    fn list_plans(
        &self,
        session_id: Option<&str>,
        status: Option<PlanStatus>,
        limit: usize,
    ) -> Result<Vec<PlanRecord>, StoreError>;

    /// Create a new active bounded-autonomy goal.
    fn create_goal(&self, goal: GoalRecord, actor: Actor) -> Result<GoalRecord, StoreError>;

    /// Atomically consume one approved plan and create its linked active goal.
    fn create_goal_from_plan(
        &self,
        goal: GoalRecord,
        executed_plan: PlanRecord,
        actor: Actor,
    ) -> Result<(GoalRecord, PlanRecord), StoreError>;

    /// Append an iteration or terminal goal state transition.
    fn update_goal(&self, goal: GoalRecord, actor: Actor) -> Result<GoalRecord, StoreError>;

    /// Reconstruct one canonical goal.
    fn get_goal(&self, id: &str) -> Result<Option<GoalRecord>, StoreError>;

    /// List bounded goals with optional session and status filters.
    fn list_goals(
        &self,
        session_id: Option<&str>,
        status: Option<GoalStatus>,
        limit: usize,
    ) -> Result<Vec<GoalRecord>, StoreError>;

    /// Create one queued durable child-agent job.
    fn create_subagent(&self, job: SubagentJob, actor: Actor) -> Result<SubagentJob, StoreError>;

    /// Append one validated child-agent lifecycle transition.
    fn update_subagent(&self, job: SubagentJob, actor: Actor) -> Result<SubagentJob, StoreError>;

    /// Reconstruct one child-agent job.
    fn get_subagent(&self, id: &str) -> Result<Option<SubagentJob>, StoreError>;

    /// List bounded child-agent jobs.
    fn list_subagents(
        &self,
        session_id: Option<&str>,
        status: Option<SubagentStatus>,
        limit: usize,
    ) -> Result<Vec<SubagentJob>, StoreError>;
}
/// Canonical event-sourced memory lifecycle repository.
pub trait MemoryRepository: Send + Sync {
    /// Create a new active canonical record.
    fn create(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// Load one reconstructed canonical record.
    fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>;

    /// Append a new active state for one existing record without changing identity or scope.
    fn update(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// List bounded active canonical records before policy filtering.
    fn list_active(&self, limit: usize) -> Result<Vec<MemoryRecord>, StoreError>;

    /// List bounded canonical records with an optional lifecycle filter.
    fn list_memories(
        &self,
        status: Option<colossus_contracts::MemoryStatus>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError>;

    /// Archive a canonical record using a new lifecycle event.
    fn archive(&self, id: &str, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// Atomically supersede one record and create its replacement.
    fn supersede(
        &self,
        id: &str,
        replacement: MemoryRecord,
        actor: Actor,
    ) -> Result<(MemoryRecord, MemoryRecord), StoreError>;
}
/// Canonical event-sourced research runs, evidence, and claims.
pub trait ResearchRepository: Send + Sync {
    /// Create one running research aggregate.
    fn create_run(&self, run: ResearchRun, actor: Actor) -> Result<ResearchRun, StoreError>;

    /// Append a validated lifecycle/progress update.
    fn update_run(&self, run: ResearchRun, actor: Actor) -> Result<ResearchRun, StoreError>;

    /// Reconstruct one canonical run.
    fn get_run(&self, id: &str) -> Result<Option<ResearchRun>, StoreError>;

    /// List bounded runs with optional session filtering.
    fn list_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResearchRun>, StoreError>;

    /// Append one canonical evidence source with a stable label.
    fn add_source(
        &self,
        source: ResearchSource,
        actor: Actor,
    ) -> Result<ResearchSource, StoreError>;

    /// List source records in stable label order.
    fn list_sources(&self, run_id: &str) -> Result<Vec<ResearchSource>, StoreError>;

    /// Append one source-backed canonical claim.
    fn add_claim(&self, claim: ResearchClaim, actor: Actor) -> Result<ResearchClaim, StoreError>;

    /// List claims in durable append order.
    fn list_claims(&self, run_id: &str) -> Result<Vec<ResearchClaim>, StoreError>;
}
/// Canonical integration, pack, resource, and publisher-trust repository.
pub trait ExtensionRepository: AggregateRepository {
    /// Reconstruct one integration connection.
    fn get_integration(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError>;

    /// List integration connections in deterministic name order.
    fn list_integrations(&self, limit: usize) -> Result<Vec<IntegrationConnection>, StoreError>;

    /// Append a validated next connection state using optimistic concurrency.
    fn save_integration(
        &self,
        connection: IntegrationConnection,
        actor: Actor,
    ) -> Result<IntegrationConnection, StoreError>;

    /// Append an explicit disconnection event without deleting history.
    fn disconnect_integration(
        &self,
        name: &str,
        actor: Actor,
        updated_at: &str,
    ) -> Result<IntegrationConnection, StoreError>;

    /// Reconstruct one installed pack lifecycle.
    fn get_pack(&self, name: &str) -> Result<Option<PackInstallation>, StoreError>;

    /// List installed and historically uninstalled packs in deterministic name order.
    fn list_packs(&self, limit: usize) -> Result<Vec<PackInstallation>, StoreError>;

    /// Append one fully verified pack installation event.
    fn install_pack(
        &self,
        installation: PackInstallation,
        actor: Actor,
    ) -> Result<PackInstallation, StoreError>;

    /// Atomically append a verified no-clobber collection of pack installations.
    fn install_packs(
        &self,
        installations: Vec<PackInstallation>,
        actor: Actor,
    ) -> Result<Vec<PackInstallation>, StoreError>;

    /// Append an enable, disable, or uninstall lifecycle transition.
    fn set_pack_status(
        &self,
        name: &str,
        status: PackStatus,
        actor: Actor,
        updated_at: &str,
    ) -> Result<PackInstallation, StoreError>;

    /// Persist a publisher/key binding as an explicit trust decision.
    fn add_publisher_trust(
        &self,
        trust: PublisherTrust,
        actor: Actor,
    ) -> Result<PublisherTrust, StoreError>;

    /// Resolve one exact publisher/key binding.
    fn get_publisher_trust(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Option<PublisherTrust>, StoreError>;

    /// List bounded publisher/key bindings in deterministic order.
    fn list_publisher_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, StoreError>;
}

/// Deterministic discovery for declarative data-only skills.
pub trait SkillRepository: Send + Sync {
    /// List selected skills in deterministic name order.
    fn list_skills(&self) -> Result<Vec<SkillRecord>, StoreError>;

    /// Load one selected skill.
    fn get_skill(&self, name: &str) -> Result<Option<SkillRecord>, StoreError>;

    /// Report every duplicate and the configured winner.
    fn duplicate_names(&self) -> Result<Vec<SkillDuplicate>, StoreError>;
}

/// Workflow definitions and run projections.
pub trait WorkflowRepository: Send + Sync {
    /// Persist a definition and immutable hash/provenance.
    fn register(
        &self,
        definition: &WorkflowDefinition,
        content_hash: &str,
        provenance: &str,
    ) -> Result<(), StoreError>;

    /// Load an exact definition version.
    fn definition(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<(WorkflowDefinition, String)>, StoreError>;

    /// Load a run projection.
    fn run(&self, run_id: &str) -> Result<Option<WorkflowRun>, StoreError>;

    /// List bounded run projections.
    fn runs(&self, limit: usize) -> Result<Vec<WorkflowRun>, StoreError>;

    /// Persist one new hash-pinned workflow schedule.
    fn create_schedule(
        &self,
        schedule: &WorkflowSchedule,
        actor: Actor,
    ) -> Result<WorkflowSchedule, StoreError>;

    /// Persist an explicit enabled/disabled schedule transition.
    fn set_schedule_enabled(
        &self,
        schedule_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowSchedule, StoreError>;

    /// Reconstruct one canonical schedule.
    fn schedule(&self, schedule_id: &str) -> Result<Option<WorkflowSchedule>, StoreError>;

    /// List bounded schedules in deterministic identifier order.
    fn schedules(&self, limit: usize) -> Result<Vec<WorkflowSchedule>, StoreError>;

    /// Persist one new hash-pinned authenticated workflow webhook.
    fn create_webhook(
        &self,
        webhook: &WorkflowWebhook,
        actor: Actor,
    ) -> Result<WorkflowWebhook, StoreError>;

    /// Persist an explicit enabled/disabled webhook transition.
    fn set_webhook_enabled(
        &self,
        webhook_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowWebhook, StoreError>;

    /// Reconstruct one canonical webhook.
    fn webhook(&self, webhook_id: &str) -> Result<Option<WorkflowWebhook>, StoreError>;

    /// List bounded webhooks in deterministic identifier order.
    fn webhooks(&self, limit: usize) -> Result<Vec<WorkflowWebhook>, StoreError>;

    /// Reconstruct one accepted delivery by webhook and replay identifier.
    fn webhook_delivery(
        &self,
        webhook_id: &str,
        delivery_id: &str,
    ) -> Result<Option<WorkflowWebhookDelivery>, StoreError>;

    /// Persist one new hash-pinned repository-event subscription.
    fn create_subscription(
        &self,
        subscription: &WorkflowSubscription,
        actor: Actor,
    ) -> Result<WorkflowSubscription, StoreError>;

    /// Persist an explicit enabled/disabled subscription transition.
    fn set_subscription_enabled(
        &self,
        subscription_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowSubscription, StoreError>;

    /// Reconstruct one canonical repository-event subscription.
    fn subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<WorkflowSubscription>, StoreError>;

    /// List bounded subscriptions in deterministic identifier order.
    fn subscriptions(&self, limit: usize) -> Result<Vec<WorkflowSubscription>, StoreError>;

    /// Reconstruct one accepted source-event delivery for idempotent replay handling.
    fn subscription_delivery(
        &self,
        subscription_id: &str,
        source_event_id: &str,
    ) -> Result<Option<WorkflowSubscriptionDelivery>, StoreError>;
}
