use super::*;

fn is_false(value: &bool) -> bool {
    !*value
}

/// Opaque capability proving one attached client completed a boundary prompt.
///
/// Clones share zeroizing storage and debug output is always redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct SandboxBoundaryAcknowledgement(Arc<zeroize::Zeroizing<String>>);

impl SandboxBoundaryAcknowledgement {
    pub(super) fn new(value: String) -> Result<Self, WorkerError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(WorkerError::Protocol(
                "sandbox boundary acknowledgement capability is invalid".into(),
            ));
        }
        Ok(Self(Arc::new(zeroize::Zeroizing::new(value))))
    }

    pub(super) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for SandboxBoundaryAcknowledgement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SandboxBoundaryAcknowledgement([REDACTED])")
    }
}

impl Serialize for SandboxBoundaryAcknowledgement {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for SandboxBoundaryAcknowledgement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientHello {
    pub(super) version: u16,
    pub(super) challenge: String,
}

#[derive(Serialize)]
pub(super) struct UnsignedServerHello<'a> {
    pub(super) version: u16,
    pub(super) challenge: &'a str,
    pub(super) server_nonce: &'a str,
    pub(super) timestamp_ms: i128,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServerHello {
    pub(super) version: u16,
    pub(super) challenge: String,
    pub(super) server_nonce: String,
    pub(super) timestamp_ms: i128,
    pub(super) authentication_tag: String,
}

/// Local worker transport or strict-contract failure.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// Public application API configuration or transport failed safely.
    #[error("public API failed: {0}")]
    PublicApi(String),
    /// Caller-owned artifact request failed safely.
    #[error(transparent)]
    Artifact(#[from] colossus_api::ApiError),
    /// Local transport failed.
    #[error("worker transport failed: {0}")]
    Io(#[from] std::io::Error),
    /// Runtime composition or operation failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Journal or repository operation failed.
    #[error(transparent)]
    Store(#[from] colossus_ports::StoreError),
    /// Workflow validation or lifecycle operation failed.
    #[error(transparent)]
    Workflow(#[from] colossus_workflow::WorkflowError),
    /// Strict JSON serialization failed.
    #[error("worker JSON contract failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Request or response violated the authenticated protocol.
    #[error("worker protocol rejected message: {0}")]
    Protocol(String),
    /// The worker returned an application error.
    #[error("worker operation failed: {0}")]
    Remote(String),
    /// An interactive operation was cancelled before its request was dispatched.
    #[error("worker interactive operation was cancelled before dispatch")]
    Cancelled,
    /// No worker answered at the configured endpoint.
    #[error("worker is unavailable at {0}")]
    Unavailable(String),
    /// A live endpoint belongs to a worker that cannot speak this protocol version.
    #[error(
        "worker at {0} is listening without a protocol-v{PROTOCOL_VERSION} authentication secret; stop that worker and start it again with this build"
    )]
    Incompatible(String),
    /// A live worker could not accept another connection before the bounded deadline.
    #[error("worker is busy at {0}")]
    Busy(String),
}

/// One operation carried by the authenticated protocol-v9 interactive duplex channel.
///
/// The request selects application behavior only. Prompts, notices, released run
/// events, and cooperative cancellation remain connection-scoped transport concerns.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InteractiveWorkerRequest {
    /// Prompt for and issue one client-scoped direct-execution acknowledgement.
    SandboxBoundaryAcknowledge {
        /// Exact durable session displayed by the attached client.
        session_id: String,
        /// Exact configured boundary being acknowledged.
        mode: SandboxBoundaryMode,
    },
    /// Run either normal Execute Mode or structurally constrained Plan Mode.
    Run {
        /// Explicit application run mode and optional selected Plan draft.
        mode: AgentRunMode,
        /// Logical model role.
        role: String,
        /// Composed caller instructions.
        instructions: String,
        /// User prompt.
        prompt: String,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Exact durable session.
        session_id: String,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// TUI-sticky declarative skills.
        sticky_skills: Vec<String>,
        /// Include bounded provider evidence on a failed turn for a trusted local client.
        #[serde(default, skip_serializing_if = "is_false")]
        include_provider_response_diagnostics: bool,
    },
    /// Approve one exact optimistic Plan revision.
    PlanApprove {
        /// Exact durable session that selected the Plan.
        session_id: String,
        /// Exact durable Plan identifier.
        plan_id: String,
        /// Expected canonical Plan revision.
        revision: u64,
    },
    /// Discard one exact optimistic Plan revision.
    PlanDiscard {
        /// Exact durable session that selected the Plan.
        session_id: String,
        /// Exact durable Plan identifier.
        plan_id: String,
        /// Expected canonical Plan revision.
        revision: u64,
    },
    /// Atomically consume and execute one approved Plan revision.
    PlanExecute {
        /// Logical model role.
        role: String,
        /// Exact durable session that selected the Plan.
        session_id: String,
        /// Exact durable Plan identifier.
        plan_id: String,
        /// Expected canonical Plan revision.
        revision: u64,
        /// Direct execution or bounded Goal Mode handoff.
        strategy: PlanExecutionStrategy,
        /// Optional bounded turn override for direct execution.
        max_turns: Option<u16>,
    },
    /// Resume the remaining budget of one active Goal.
    GoalResume {
        /// Logical model role.
        role: String,
        /// Exact durable session that owns the Goal.
        session_id: String,
        /// Exact durable Goal identifier.
        goal_id: String,
    },
}

/// Versioned operations exposed by the local worker application API.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerOperation {
    /// Authenticate the endpoint and return bounded readiness metadata.
    Ping,
    /// Verify the authoritative journal chain and anchors.
    AuditVerify,
    /// Verify the journal and report whether secure anchors are enabled.
    AuditAnchorStatus,
    /// Read bounded ciphertext-free audit evidence.
    AuditRead {
        /// First global sequence.
        from: u64,
        /// Maximum records.
        limit: usize,
    },
    /// Inspect configured durable audit-export readiness.
    AuditExportStatus,
    /// Drain queued external audit evidence.
    AuditExportDrain,
    /// Reset and replay the external audit-export consumer.
    AuditExportReset,
    /// Check policy readiness.
    PolicyDoctor,
    /// List projection positions and lag.
    ProjectionStatus,
    /// Drain projection outbox work.
    ProjectionDrain,
    /// Rebuild one or every disposable projection.
    ProjectionRebuild {
        /// Optional exact projection name.
        name: Option<String>,
    },
    /// Inspect storage and writer readiness.
    StateDoctor,
    /// Inspect sandbox readiness.
    SandboxDoctor,
    /// Return the direct-execution boundary acknowledgement pending for one session.
    SandboxBoundaryStatus {
        /// Exact durable session.
        session_id: String,
    },
    /// List provider profile readiness without network access.
    ProviderProfiles,
    /// Exercise one provider diagnostic path.
    ProviderDoctor {
        /// Optional exact profile.
        profile: Option<String>,
        /// Include bounded non-success provider response diagnostics.
        #[serde(default, skip_serializing_if = "is_false")]
        include_provider_response: bool,
    },
    /// List normalized models for one provider.
    ProviderModels {
        /// Optional exact profile.
        profile: Option<String>,
    },
    /// Show configured explicit model profiles.
    ModelProfiles,
    /// Exercise one explicit model generation diagnostic.
    ModelDoctor {
        /// Optional exact model profile.
        profile: Option<String>,
        /// Include bounded non-success provider response diagnostics.
        #[serde(default, skip_serializing_if = "is_false")]
        include_provider_response: bool,
    },
    /// Show role-to-model-profile routing.
    ProviderRoutes,
    /// Resolve one role to bounded provider metadata without network access.
    ProviderRoute {
        /// Logical provider role.
        role: String,
    },
    /// List safe configured search profile metadata.
    SearchProfiles,
    /// Execute one explicit provider-neutral search.
    SearchQuery {
        /// Exact configured search role.
        role: String,
        /// Search query, bounded by the runtime contract.
        query: String,
        /// Requested normalized result count.
        limit: usize,
    },
    /// List active model-visible tool schemas.
    ToolsList,
    /// Show credential-free effective tool and action resolution.
    AccessEffective,
    /// Upload one policy-authorized bounded workspace file into caller-owned artifact storage.
    ArtifactUpload {
        /// Workspace-relative or policy-authorized input path.
        path: String,
        /// Declared intended use.
        purpose: ArtifactPurpose,
        /// Exact caller-selected replay key.
        idempotency_key: String,
    },
    /// Return CLI-owned released artifact metadata.
    ArtifactGet {
        /// Exact opaque artifact identifier.
        artifact_id: String,
    },
    /// Download one CLI-owned artifact through the filesystem policy boundary.
    ArtifactDownload {
        /// Exact opaque artifact identifier.
        artifact_id: String,
        /// Workspace-relative or policy-authorized output path.
        output: String,
    },
    /// Execute the normal audited model application path.
    RunModel {
        /// Logical role.
        role: String,
        /// Composed caller instructions.
        instructions: String,
        /// User prompt.
        prompt: String,
        /// Paths read only by the worker through the runtime filesystem boundary.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<String>,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Optional exact durable session.
        session_id: Option<String>,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// TUI-sticky declarative skills.
        sticky_skills: Vec<String>,
    },
    /// Execute any protocol-v9 interactive operation with authenticated duplex control.
    RunInteractive {
        /// Strict application request carried by the interactive channel.
        request: InteractiveWorkerRequest,
        /// Opaque capability issued by an earlier acknowledgement prompt on this client.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sandbox_boundary_acknowledgement: Option<SandboxBoundaryAcknowledgement>,
    },
    /// Execute structurally read-only Plan Mode.
    RunPlan {
        /// Logical role.
        role: String,
        /// Caller instructions composed with mandatory planning constraints.
        instructions: String,
        /// Planning prompt.
        prompt: String,
        /// Paths read only by the worker through the runtime filesystem boundary.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<String>,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Optional exact durable session.
        session_id: Option<String>,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// TUI-sticky declarative skills.
        sticky_skills: Vec<String>,
    },
    /// Execute the permit-bound offline echo effect.
    Echo {
        /// Exact echo input.
        message: String,
    },
    /// Create an empty durable session.
    SessionCreate {
        /// Optional bounded title.
        title: Option<String>,
    },
    /// Reconstruct one session summary.
    SessionGet {
        /// Exact session identifier.
        session_id: String,
    },
    /// List recent sessions.
    SessionList {
        /// Bounded result limit.
        limit: usize,
    },
    /// Reconstruct append-only messages for one session.
    SessionMessages {
        /// Exact session identifier.
        session_id: String,
    },
    /// Reconstruct one bounded canonical session-message page.
    SessionMessagesPage {
        /// Exact session identifier.
        session_id: String,
        /// Exclusive upper sequence bound for older paging.
        before_sequence: Option<u64>,
        /// Maximum records, clamped to 100.
        limit: usize,
    },
    /// Resolve the newest session.
    SessionLatest,
    /// Refresh bounded actionable work for one session.
    WorkState {
        /// Exact session identifier.
        session_id: String,
    },
    /// Reconstruct the canonical presentation profile.
    PresentationGet,
    /// Reconstruct newest encrypted terminal history entries.
    PresentationHistory {
        /// Maximum entries in chronological order.
        limit: usize,
    },
    /// Persist a complete presentation profile through the runtime gateway.
    PresentationSave {
        /// Strict complete replacement profile.
        preferences: TerminalPreferences,
    },
    /// Append one encrypted terminal history entry through the runtime gateway.
    PresentationHistoryAppend {
        /// Exact submitted entry.
        entry: String,
    },
    /// Show context budget status.
    ContextStatus {
        /// Exact session identifier.
        session_id: String,
        /// Logical model role.
        role: String,
    },
    /// List immutable context snapshots.
    ContextList {
        /// Exact session identifier.
        session_id: String,
    },
    /// Force context compaction.
    ContextCompact {
        /// Exact session identifier.
        session_id: String,
        /// Logical model role.
        role: String,
    },
    /// Activate one context snapshot.
    ContextRestore {
        /// Exact session identifier.
        session_id: String,
        /// Exact snapshot identifier.
        snapshot_id: String,
    },
    /// List metadata-only run telemetry.
    TelemetryRuns {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum runs.
        limit: usize,
    },
    /// Show one metadata-only run timeline.
    TelemetryShow {
        /// Full or uniquely prefixed run identifier.
        id_or_prefix: String,
        /// Maximum timeline records.
        limit: usize,
    },
    /// Aggregate metadata-only run metrics.
    TelemetryMetrics {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum runs.
        limit: usize,
    },
    /// List canonical tasks.
    TaskList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<TaskStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one task.
    TaskGet {
        /// Exact task identifier.
        task_id: String,
    },
    /// Create a task through the effect gateway.
    TaskCreate {
        /// Owning session.
        session_id: String,
        /// Task title.
        title: String,
        /// Task detail.
        description: String,
        /// Initial lifecycle status.
        status: TaskStatus,
    },
    /// Update supplied task fields.
    TaskUpdate {
        /// Exact task identifier.
        task_id: String,
        /// Optional replacement title.
        title: Option<String>,
        /// Optional replacement detail.
        description: Option<String>,
        /// Optional replacement status.
        status: Option<TaskStatus>,
    },
    /// List canonical key decisions.
    DecisionList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<DecisionStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one key decision.
    DecisionGet {
        /// Exact decision identifier.
        decision_id: String,
    },
    /// Create an active key decision.
    DecisionCreate {
        /// Owning session.
        session_id: String,
        /// Decision title.
        title: String,
        /// Binding decision content.
        decision: String,
        /// Decision priority.
        priority: DecisionPriority,
        /// Future intent.
        intent: String,
        /// Applicability condition.
        applies_when: String,
        /// Supporting rationale.
        rationale: String,
        /// Bounded source excerpt.
        source_excerpt: String,
    },
    /// Update supplied decision fields.
    DecisionUpdate {
        /// Exact decision identifier.
        decision_id: String,
        /// Optional replacement title.
        title: Option<String>,
        /// Optional replacement decision.
        decision: Option<String>,
        /// Optional replacement priority.
        priority: Option<DecisionPriority>,
        /// Optional replacement intent.
        intent: Option<String>,
        /// Optional replacement applicability.
        applies_when: Option<String>,
        /// Optional replacement rationale.
        rationale: Option<String>,
        /// Optional replacement source excerpt.
        source_excerpt: Option<String>,
    },
    /// Archive one active decision.
    DecisionArchive {
        /// Exact decision identifier.
        decision_id: String,
    },
    /// Atomically supersede one active decision.
    DecisionSupersede {
        /// Exact decision identifier.
        decision_id: String,
        /// Replacement title.
        title: String,
        /// Replacement decision.
        decision: String,
        /// Replacement priority.
        priority: DecisionPriority,
        /// Replacement intent.
        intent: String,
        /// Replacement applicability.
        applies_when: String,
        /// Replacement rationale.
        rationale: String,
        /// Replacement source excerpt.
        source_excerpt: String,
    },
    /// List canonical plans.
    PlanList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<PlanStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one plan.
    PlanGet {
        /// Exact plan identifier.
        plan_id: String,
    },
    /// Create a draft plan.
    PlanCreate {
        /// Owning session.
        session_id: String,
        /// Source prompt.
        prompt: String,
        /// Plan content.
        content: String,
        /// Ordered steps.
        steps: Vec<PlanStep>,
    },
    /// Approve one draft plan.
    PlanApprove {
        /// Exact plan identifier.
        plan_id: String,
    },
    /// Atomically consume and execute one approved plan.
    PlanRun {
        /// Logical model role.
        role: String,
        /// Exact approved plan identifier.
        plan_id: String,
        /// Optional bounded model-turn override.
        max_turns: Option<u16>,
    },
    /// List canonical goals.
    GoalList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<GoalStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one goal.
    GoalGet {
        /// Exact goal identifier.
        goal_id: String,
    },
    /// Execute bounded Goal Mode.
    GoalRun {
        /// Logical model role.
        role: String,
        /// Goal objective.
        objective: String,
        /// Existing session.
        session_id: String,
        /// Iteration ceiling.
        max_iterations: u16,
        /// Optional approved source plan.
        source_plan_id: Option<String>,
    },
    /// Queue a durable child-agent job.
    AgentQueue {
        /// Owning session.
        session_id: String,
        /// Delegated task.
        task: String,
        /// Child model role.
        role: String,
    },
    /// List durable child-agent jobs.
    AgentList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<SubagentStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one child-agent job.
    AgentGet {
        /// Exact job identifier.
        job_id: String,
    },
    /// Show child-agent scheduler status.
    AgentStatus {
        /// Optional session filter.
        session_id: Option<String>,
    },
    /// Drain queued child-agent jobs.
    AgentDrain,
    /// Cancel one child-agent job.
    AgentCancel {
        /// Exact job identifier.
        job_id: String,
    },
    /// Requeue one stopped child-agent job.
    AgentRequeue {
        /// Exact job identifier.
        job_id: String,
    },
    /// List canonical memories.
    MemoryList {
        /// Optional status filter.
        status: Option<MemoryStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Read one canonical memory.
    MemoryGet {
        /// Exact memory identifier.
        memory_id: String,
    },
    /// Search canonical re-filtered memories.
    MemorySearch {
        /// Search query.
        query: String,
        /// Optional session scope.
        session_id: Option<String>,
        /// Optional repository scope.
        repository_id: Option<String>,
        /// Maximum records.
        limit: usize,
    },
    /// Create one canonical memory.
    MemoryCreate {
        /// Canonical scope.
        scope: MemoryScope,
        /// Memory kind.
        memory_kind: String,
        /// Confidence in zero through one.
        confidence: f32,
        /// Canonical text.
        text: String,
        /// Supporting rationale.
        rationale: String,
        /// Optional UTC expiry.
        expires_at: Option<String>,
    },
    /// Archive one active memory.
    MemoryArchive {
        /// Exact memory identifier.
        memory_id: String,
    },
    /// Supersede one active memory.
    MemorySupersede {
        /// Exact memory identifier.
        memory_id: String,
        /// Replacement text.
        text: String,
        /// Replacement rationale.
        rationale: String,
    },
    /// Show memory-index readiness.
    MemoryIndexStatus,
    /// Retry queued memory-index work.
    MemoryIndexSync,
    /// Rebuild the disposable memory index.
    MemoryIndexRebuild,
    /// Run bounded durable research.
    ResearchRun {
        /// Research question.
        question: String,
        /// Existing session or none to create one.
        session_id: Option<String>,
        /// Research depth.
        depth: ResearchDepth,
        /// Enabled source lanes.
        source_kinds: Vec<ResearchSourceKind>,
    },
    /// List canonical research runs.
    ResearchList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one research run.
    ResearchGet {
        /// Exact run identifier.
        run_id: String,
    },
    /// List research evidence sources.
    ResearchSources {
        /// Exact run identifier.
        run_id: String,
    },
    /// List source-backed research claims.
    ResearchClaims {
        /// Exact run identifier.
        run_id: String,
    },
    /// Execute one exact process through the sandbox boundary.
    ProcessRun {
        /// Exact executable path.
        executable: String,
        /// Exact working directory.
        cwd: String,
        /// Literal argument vector.
        args: Vec<String>,
        /// Explicit environment values.
        environment: BTreeMap<String, String>,
    },
    /// Fetch one exact URL through policy and quarantine.
    NetworkGet {
        /// Exact URL.
        url: String,
    },
    /// List configured MCP servers without launching them.
    McpServers,
    /// Discover allowlisted MCP tools.
    McpTools {
        /// Optional exact server filter.
        server: Option<String>,
    },
    /// Invoke one allowlisted MCP tool.
    McpCall {
        /// Exact server name.
        server: String,
        /// Exact tool name.
        tool: String,
        /// Inline JSON or a server-local `@path` reference.
        arguments_source: String,
    },
    /// Begin an interactive MCP OAuth login.
    McpAuthBegin {
        /// Exact configured server.
        server: String,
    },
    /// Complete a pending MCP OAuth login.
    McpAuthComplete {
        /// Exact configured server.
        server: String,
        /// Final loopback redirect URL.
        callback_url: String,
    },
    /// Inspect local MCP OAuth credential status.
    McpAuthStatus {
        /// Exact configured server.
        server: String,
    },
    /// Clear local MCP OAuth credentials.
    McpAuthLogout {
        /// Exact configured server.
        server: String,
    },
    /// List selected declarative skill summaries.
    SkillList,
    /// Read one selected declarative skill.
    SkillGet {
        /// Exact skill name.
        name: String,
    },
    /// Report duplicate skill names and winners.
    SkillDuplicates,
    /// Preview deterministic skill composition.
    SkillCompose {
        /// User prompt.
        prompt: String,
        /// Explicit skill names.
        skills: Vec<String>,
    },
    /// Scaffold one installed user skill.
    SkillScaffold {
        /// Skill name.
        name: String,
        /// Skill description.
        description: String,
        /// Data-only instructions.
        instructions: String,
        /// Declared resource directories.
        resource_dirs: Vec<String>,
    },
    /// Inspect installed skill metadata and hashes.
    SkillInspect {
        /// Exact skill name.
        name: String,
    },
    /// Read one authorable skill file.
    SkillFileRead {
        /// Exact skill name.
        name: String,
        /// Relative authorable path.
        path: String,
    },
    /// Write one authorable skill file.
    SkillWrite {
        /// Exact skill name.
        name: String,
        /// Relative authorable path.
        path: String,
        /// Replacement content.
        content: String,
        /// Optional optimistic content hash.
        expected_sha256: Option<String>,
    },
    /// Validate an installed or workspace-local skill.
    SkillValidate {
        /// Installed name or local path.
        target: String,
        /// Whether target is a local path.
        local: bool,
    },
    /// Install one validated workspace-local skill.
    SkillInstall {
        /// Server-local skill directory.
        path: String,
    },
    /// List bounded resources for one explicitly active skill.
    SkillResources {
        /// Exact skill name.
        name: String,
    },
    /// Read one bounded skill resource.
    SkillResourceRead {
        /// Exact skill name.
        name: String,
        /// Relative resource path.
        path: String,
    },
    /// List canonical pack lifecycles.
    PackList {
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one pack lifecycle.
    PackGet {
        /// Exact pack name.
        name: String,
    },
    /// Verify one server-local pack.
    PackVerify {
        /// Server-local pack path.
        path: String,
    },
    /// Install one verified pack.
    PackInstall {
        /// Server-local pack path.
        path: String,
        /// Explicit development override.
        allow_untrusted: bool,
    },
    /// Enable one installed pack.
    PackEnable {
        /// Exact pack name.
        name: String,
    },
    /// Disable one installed pack.
    PackDisable {
        /// Exact pack name.
        name: String,
    },
    /// Uninstall one installed pack.
    PackUninstall {
        /// Exact pack name.
        name: String,
    },
    /// Invoke one active fixed-argument pack tool.
    PackCall {
        /// Exact generated tool name.
        tool: String,
    },
    /// List pack publisher trust bindings.
    PackTrustList {
        /// Maximum records.
        limit: usize,
    },
    /// Add a pack publisher trust binding.
    PackTrustAdd {
        /// Publisher identifier.
        publisher: String,
        /// Base64 Ed25519 public key.
        public_key: String,
    },
    /// Verify a signed server-local pack and skill collection.
    CollectionVerify {
        /// Server-local collection directory.
        path: String,
    },
    /// Build and sign a deterministic server-local collection.
    CollectionBuild {
        /// Staged payload containing `packs/` and `skills/`.
        source: String,
        /// Clean destination directory.
        destination: String,
        /// Stable collection name.
        name: String,
        /// Immutable collection version.
        version: String,
        /// Trusted publisher identity.
        publisher: String,
        /// Explicit RFC3339 UTC timestamp.
        created_at: String,
        /// Environment reference for the signing seed.
        signing_key_reference: String,
    },
    /// Install every artifact from a trusted collection without clobbering.
    CollectionInstall {
        /// Server-local collection directory.
        path: String,
    },
    /// Pull an authenticated signed collection transport to a clean local directory.
    RegistryPull {
        /// Credential-free HTTPS URL or explicit loopback HTTP URL.
        url: String,
        /// Clean server-local destination directory.
        destination: String,
        /// Optional environment-backed bearer credential reference.
        credential_reference: Option<String>,
    },
    /// Push a verified collection using a create-only authenticated request.
    RegistryPush {
        /// Server-local collection directory.
        path: String,
        /// Credential-free HTTPS URL or explicit loopback HTTP URL.
        url: String,
        /// Optional environment-backed bearer credential reference.
        credential_reference: Option<String>,
    },
    /// Verify a signed offline release bundle.
    BundleVerify {
        /// Server-local bundle path.
        path: String,
    },
    /// Derive the public identity for a referenced bundle signing seed.
    BundleKeyInfo {
        /// Environment reference for the signing seed.
        signing_key_reference: String,
    },
    /// Build and sign a server-local staged bundle payload.
    BundleBuild {
        /// Server-local staged payload path.
        source: String,
        /// Server-local destination path.
        destination: String,
        /// Bundle identity.
        name: String,
        /// Release version.
        version: String,
        /// Trusted publisher identity.
        publisher: String,
        /// Explicit RFC3339 UTC timestamp.
        created_at: String,
        /// Optional source revision.
        source_revision: Option<String>,
        /// Environment reference for the signing seed.
        signing_key_reference: String,
    },
    /// Verify and install the current-target bundle artifact.
    BundleInstall {
        /// Server-local bundle path.
        path: String,
        /// Server-local clean installation prefix.
        prefix: String,
    },
    /// List safe integration summaries.
    IntegrationList {
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one integration without credentials.
    IntegrationGet {
        /// Exact integration name.
        name: String,
    },
    /// Connect one first-party integration.
    IntegrationConnect {
        /// Exact integration name.
        name: String,
        /// Optional base URL.
        base_url: Option<String>,
        /// Authentication shape with no values.
        auth: IntegrationAuth,
        /// Optional primary credential reference.
        credential_reference: Option<String>,
        /// Named credential references.
        credential_references: BTreeMap<String, String>,
        /// Declared scopes.
        scopes: Vec<String>,
    },
    /// Import one OpenAPI document.
    IntegrationImportOpenApi {
        /// Exact integration name.
        name: String,
        /// Inline JSON or a server-local `@path` reference.
        document_source: String,
        /// Optional base URL override.
        base_url: Option<String>,
        /// Authentication shape with no values.
        auth: IntegrationAuth,
        /// Optional credential reference.
        credential_reference: Option<String>,
        /// Declared scopes.
        scopes: Vec<String>,
    },
    /// Disconnect one integration.
    IntegrationDisconnect {
        /// Exact integration name.
        name: String,
    },
    /// Invoke one connected integration tool.
    IntegrationCall {
        /// Exact dynamic tool name.
        tool: String,
        /// Inline JSON or a server-local `@path` reference.
        arguments_source: String,
    },
    /// Validate a workflow file through the normal filesystem policy boundary.
    WorkflowValidate {
        /// Server-local workflow path.
        path: String,
    },
    /// Register a workflow file through the normal filesystem policy boundary.
    WorkflowRegister {
        /// Server-local workflow path.
        path: String,
    },
    /// List registered workflow definition audit metadata.
    WorkflowList,
    /// Show one exact registered workflow.
    WorkflowShow {
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
    },
    /// Start a workflow run.
    WorkflowStart {
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Inline JSON or a server-local `@path` reference.
        inputs_source: String,
        /// Leave the run queued for worker drain instead of executing immediately.
        queued: bool,
    },
    /// Create one persisted hash-pinned cadence schedule.
    WorkflowScheduleCreate {
        /// Stable schedule identifier.
        schedule_id: String,
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Inline JSON or a server-local `@path` reference.
        inputs_source: String,
        /// Fixed bounded cadence in seconds.
        cadence_seconds: u64,
        /// Explicit multiple-occurrence behavior.
        misfire_policy: WorkflowScheduleMisfirePolicy,
        /// Initial enabled state.
        enabled: bool,
        /// Optional UTC RFC3339 first occurrence boundary.
        starts_at: Option<String>,
    },
    /// List persisted workflow schedules.
    WorkflowScheduleList {
        /// Maximum schedules.
        limit: usize,
    },
    /// Show one persisted workflow schedule.
    WorkflowScheduleShow {
        /// Exact schedule identifier.
        schedule_id: String,
    },
    /// Explicitly enable or disable one persisted schedule.
    WorkflowScheduleSetEnabled {
        /// Exact schedule identifier.
        schedule_id: String,
        /// Requested enabled state.
        enabled: bool,
    },
    /// Evaluate due workflow schedules against a real or explicit clock.
    WorkflowScheduleTick {
        /// Optional UTC RFC3339 clock used for deterministic operation.
        at: Option<String>,
    },
    /// Create one persisted hash-pinned authenticated workflow webhook.
    WorkflowWebhookCreate {
        /// Stable webhook identifier.
        webhook_id: String,
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Late-bound `env:VARIABLE` HMAC secret reference.
        secret_reference: String,
        /// Maximum accepted delivery age.
        replay_window_seconds: u64,
        /// Maximum accepted raw JSON body size.
        max_body_bytes: u64,
        /// Initial enabled state.
        enabled: bool,
    },
    /// List persisted authenticated workflow webhooks.
    WorkflowWebhookList {
        /// Maximum webhooks.
        limit: usize,
    },
    /// Show one persisted authenticated workflow webhook.
    WorkflowWebhookShow {
        /// Exact webhook identifier.
        webhook_id: String,
    },
    /// Explicitly enable or disable one persisted webhook.
    WorkflowWebhookSetEnabled {
        /// Exact webhook identifier.
        webhook_id: String,
        /// Requested enabled state.
        enabled: bool,
    },
    /// Authenticate and durably ingest one workflow webhook delivery.
    WorkflowWebhookIngest {
        /// Exact webhook identifier.
        webhook_id: String,
        /// Sender-supplied replay identifier.
        delivery_id: String,
        /// Sender-supplied signed UTC RFC3339 timestamp.
        timestamp: String,
        /// Sender-supplied HMAC-SHA256 signature.
        signature: String,
        /// Lowercase application header fields, excluding authentication headers.
        headers: BTreeMap<String, String>,
        /// Inline JSON or a server-local `@path` reference.
        body_source: String,
    },
    /// Create one persisted hash-pinned repository-event subscription.
    WorkflowSubscriptionCreate {
        /// Stable subscription identifier.
        subscription_id: String,
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Exact versioned domain event type.
        event_type: String,
        /// Optional aggregate stream prefix.
        stream_prefix: Option<String>,
        /// Initial enabled state.
        enabled: bool,
        /// Optional global sequence after which delivery begins.
        after_sequence: Option<u64>,
    },
    /// List persisted repository-event subscriptions.
    WorkflowSubscriptionList {
        /// Maximum subscriptions.
        limit: usize,
    },
    /// Show one persisted repository-event subscription.
    WorkflowSubscriptionShow {
        /// Exact subscription identifier.
        subscription_id: String,
    },
    /// Explicitly enable or disable one persisted subscription.
    WorkflowSubscriptionSetEnabled {
        /// Exact subscription identifier.
        subscription_id: String,
        /// Requested enabled state.
        enabled: bool,
    },
    /// Evaluate bounded canonical journal work for subscriptions.
    WorkflowSubscriptionTick,
    /// Reconstruct one workflow run.
    WorkflowStatus {
        /// Exact run identifier.
        run_id: String,
    },
    /// Resume one safe interrupted/waiting workflow run.
    WorkflowResume {
        /// Exact run identifier.
        run_id: String,
    },
    /// Provide structured input to a waiting run.
    WorkflowInput {
        /// Exact run identifier.
        run_id: String,
        /// Inline JSON or a server-local `@path` reference.
        input_source: String,
    },
    /// Cancel a non-terminal workflow run.
    WorkflowCancel {
        /// Exact run identifier.
        run_id: String,
    },
    /// Drain safe queued work and projections once.
    Drain,
    /// Request clean checkpoint and worker shutdown.
    Shutdown,
}
