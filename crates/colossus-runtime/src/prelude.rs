//! Private imports shared by the tightly coupled runtime composition modules.

pub(super) use async_trait::async_trait;
pub(super) use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
pub(super) use colossus_access::{
    AccessConfig, AccessContext, AccessDecision, AccessProfile, AccessResolution, ActionClass,
    ActionDescriptor, CapabilitySource, ToolDescriptor, builtin_action_descriptors,
    builtin_tool_descriptor, resolve_access, validate_config as validate_access_config,
};
pub(super) use colossus_agent::{AgentError, AgentService, DEFAULT_MAX_TURNS, MAX_TURNS};
pub(super) use colossus_audit::{
    AuditExportReport, AuditExportService, AuditExportStatus, GatewayDirectoryAuditExporter,
    GatewayWormAuditExporter, evidence,
};
pub(super) use colossus_context::{ContextConfig, ContextService, EventSourcedContextRepository};
pub(super) use colossus_contracts::{
    Actor, ActorType, AgentRunMode, AgentRunOutcome, AgentRunResult, AuditEvidence,
    BundleInstallation, BundleMaterialization, BundleSigningKeyInfo, CollectionInstallation,
    CollectionMaterialization, CollectionVerification, ContextSnapshot, ContextStatus,
    ControlledAgentTerminal, CredentialReference, DecisionOutcome, DecisionPriority,
    DecisionSource, DecisionStatus, EffectRequest, EventClassification, ExecutionContext,
    FilesystemGrant, GoalIterationResult, GoalRecord, GoalRunOutcome, GoalRunResult, GoalStatus,
    IntegrationAuth, IntegrationConnection, IntegrationSummary, KeyDecision, MemoryRecord,
    MemoryScope, MemoryStatus, ModelImageReference, ModelMessage, ModelMessageRole, ModelRequest,
    ModelRoute, ModelToolDefinition, NewEvent, PackInstallation, PackVerification, PlanDraftTarget,
    PlanExecutionOutcome, PlanExecutionStrategy, PlanRecord, PlanStatus, PlanStep, PreparedContext,
    ProjectionStatus, ProviderEvent, ProviderModelInfo, ProviderReadiness, ProviderReadinessCheck,
    ProviderResponseDiagnostic, ProviderRoute, ProviderStreamItem, ProviderTurn, PublisherTrust,
    QuarantinedEffectResult, RegistryPullResult, RegistryPushResult, ResearchClaim, ResearchDepth,
    ResearchRun, ResearchSource, ResearchSourceKind, ResourceAuthority, RiskAssessment,
    RunBranchContextMode, RunEvent, RunEventEnvelope, RunTelemetryDetail, RunTelemetrySummary,
    SandboxBoundaryMode, SearchProfileSummary, SearchRequest, SearchResponse, SearchRoute,
    SecurityPostureFinding, SecurityPostureReport, SecurityPostureSeverity, SessionMessage,
    SessionMessageAppend, SessionMessagePage, SessionSummary, SkillComposition, SkillDuplicate,
    SkillFileRead, SkillInspection, SkillInstallResult, SkillRecord, SkillResourceEntry,
    SkillResourceRead, SkillScaffoldResult, SkillValidationResult, SkillWriteResult,
    StartupVerificationMode, SubagentJob, SubagentQueueStatus, SubagentStatus, TaskRecord,
    TaskStatus, TelemetryMetrics, TerminalPreferences, ToolCall, ToolResult, ToolSpec,
    UserPromptRequest, WorkStateSnapshot, WorkflowWebhookDispatch, validate_model_transcript,
};
pub(super) use colossus_home::ConfinedRoot;
pub(super) use colossus_integrations::{
    EventSourcedExtensionRepository, IntegrationExecutor, IntegrationRequest,
};
pub(super) use colossus_journal_postgres::{PostgresEventJournal, PostgresJournalConfig};
pub(super) use colossus_journal_redb::{
    DisabledCheckpointSigner, Ed25519CheckpointSigner, EnvironmentKeyProvider,
    PlaintextKeyProvider, PlatformKeyProvider, RedbEventJournal, RedbWriterLease, platform_secret,
};
pub(super) use colossus_mcp::{
    MAX_MCP_PAGES, MAX_MCP_TOOLS, McpCallOutput, McpConfig, McpError, McpExecutor,
    McpOAuthCredentialStoreKind, McpOAuthLogin, McpOAuthStatus, McpOperation, McpServerConfig,
    McpServerSummary, McpToolSummary, McpToolsPage, McpValidationContext,
    validate_config as validate_mcp_config, validate_tool_arguments,
};
pub(super) use colossus_media::{
    JournalRunInputMediaResolver, MAX_IMAGE_BYTES, validate_image_bytes,
};
pub(super) use colossus_memory::{
    EventSourcedMemoryRepository, LazyTantivyMemoryIndex, MemoryIndexRegistration, MemoryService,
    TantivyMemoryIndex, UnavailableMemoryIndex,
};
pub(super) use colossus_memory_chroma::{
    ChromaExecutor, ChromaMemoryIndex, ChromaProfile, GatewayOpenAiEmbeddingProvider,
    LocalHashEmbeddingProvider, OpenAiEmbeddingExecutor, OpenAiEmbeddingProfile,
};
pub(super) use colossus_network::AdditionalRootCertificates;
pub(super) use colossus_observability::ObservedEventJournal;
pub(super) use colossus_packs::{PackError, PackExecutor, PackOperation, PackService};
pub(super) use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit,
    GatewayError, MIN_OCI_EFFECT_TIMEOUT_MS, MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS,
    MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS, OpaConfig, OpaPolicy, ReleasedEffectObserver,
    ReleasedEffectResult, SafetyKernel, SandboxBoundaryGate, canonical_network_origin,
    effect_request, network_destination_match, system_actor,
};
pub(super) use colossus_ports::{
    ApprovalProvider, AuditExporter, CheckpointSigner, ContextError, ContextPreparer,
    ContextRepository, EmbeddingProvider, EventJournal, ExtensionRepository, ExternalWorkQueue,
    KeyProvider, MemoryIndex, MemoryRepository, MemoryRetriever, ModelProvider, ModelProviderError,
    PolicyDecisionPoint, PresentationRepository, ProjectionStore, ProviderEventObserver,
    ProviderTurnOptions, ResearchRepository, RiskEvaluationError, RiskEvaluator, RunControl,
    RunEventObserver, RunInputMediaResolver, SearchError, SearchProvider, SessionRepository,
    SkillRepository, StoreError, ToolError, ToolExecutor, ToolRegistry, UserPromptProvider,
    WorkRepository, WorkflowRepository,
};
pub(super) use colossus_presentation::EventSourcedPresentationRepository;
pub(super) use colossus_projection::{
    JournalExternalWorkQueue, ProjectedSessionActivityPage, ProjectedSessionActivityReader,
    ProjectionRunReport, ProjectionWorker, default_handlers,
};
pub(super) use colossus_provider::{
    ModelProfile, ProviderEffectInput, ProviderError, ProviderExecutor, ProviderKind,
    ProviderProfile, ProviderRegistry,
};
pub(super) use colossus_research::{
    EventSourcedResearchRepository, ResearchCollection, ResearchCollector, ResearchLimits,
    ResearchModel, ResearchService, ResearchSourceDraft,
};
pub(super) use colossus_sandbox::{
    FilesystemExecutor, HttpExecutor, ProcessSpec, SandboxDoctorReport, SandboxExecutorConfig,
    SandboxProcessExecutor, sandbox_doctor,
};
pub(super) use colossus_search::{
    SearchAdapterError, SearchEffectInput, SearchExecutor, SearchKind, SearchProfile,
    SearchRegistry, default_search_limit,
};
pub(super) use colossus_session::EventSourcedSessionRepository;
pub(super) use colossus_skills::{
    FilesystemSkillRepository, SkillAuthoringService, SkillComposer, SkillResourceService,
    SkillRoot,
};
pub(super) use colossus_telemetry::TelemetryService;
pub(super) use colossus_tools::{
    MCP_TOOLS_MAX_OUTPUT_BYTES, StaticToolRegistry, ToolCatalogError, builtin_specs,
};
pub(super) use colossus_work::{EventSourcedWorkRepository, WorkService};
pub(super) use colossus_workflow::{
    EventSourcedWorkflowRepository, ValidatedWorkflow, WorkflowEffect, WorkflowEffectRunner,
    WorkflowError, WorkflowService, validate_definition,
};
pub(super) use ignore::WalkBuilder;
pub(super) use serde::{Deserialize, Serialize};
pub(super) use serde_json::{Value, json};
pub(super) use sha2::{Digest, Sha256};
pub(super) use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, Weak},
    time::{Duration, Instant},
};
pub(super) use thiserror::Error;
pub(super) use tokio::sync::{Mutex as TokioMutex, Notify, mpsc};
pub(super) use tokio::task::JoinSet;
pub(super) use tracing::Instrument as _;
pub(super) use url::Url;
pub(super) use uuid::Uuid;
