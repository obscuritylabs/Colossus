//! Runtime composition root. Interfaces call this layer and own no product logic.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_access::{
    AccessConfig, AccessContext, AccessDecision, AccessProfile, AccessResolution, ActionClass,
    ActionDescriptor, CapabilitySource, ToolDescriptor, builtin_action_descriptors,
    builtin_tool_descriptor, resolve_access, validate_config as validate_access_config,
};
use colossus_agent::{AgentError, AgentService, DEFAULT_MAX_TURNS, MAX_TURNS};
use colossus_audit::{
    AuditExportReport, AuditExportService, AuditExportStatus, GatewayDirectoryAuditExporter,
    GatewayWormAuditExporter,
};
use colossus_context::{ContextConfig, ContextService, EventSourcedContextRepository};
use colossus_contracts::{
    Actor, ActorType, AgentRunMode, AgentRunOutcome, AgentRunResult, BundleInstallation,
    BundleMaterialization, BundleSigningKeyInfo, CollectionInstallation, CollectionMaterialization,
    CollectionVerification, ContextSnapshot, ContextStatus, ControlledAgentTerminal,
    CredentialReference, DecisionOutcome, DecisionPriority, DecisionSource, DecisionStatus,
    EffectRequest, EventClassification, ExecutionContext, FilesystemGrant, GoalIterationResult,
    GoalRecord, GoalRunOutcome, GoalRunResult, GoalStatus, IntegrationAuth, IntegrationConnection,
    IntegrationSummary, KeyDecision, MemoryRecord, MemoryScope, MemoryStatus, ModelMessage,
    ModelMessageRole, ModelRequest, ModelRoute, ModelToolDefinition, NewEvent, PackInstallation,
    PackVerification, PlanDraftTarget, PlanExecutionOutcome, PlanExecutionStrategy, PlanRecord,
    PlanStatus, PlanStep, PreparedContext, ProjectionStatus, ProviderEvent, ProviderModelInfo,
    ProviderReadiness, ProviderReadinessCheck, ProviderResponseDiagnostic, ProviderRoute,
    ProviderStreamItem, ProviderTurn, PublisherTrust, QuarantinedEffectResult, RegistryPullResult,
    RegistryPushResult, ResearchClaim, ResearchDepth, ResearchRun, ResearchSource,
    ResearchSourceKind, RiskAssessment, RunTelemetryDetail, RunTelemetrySummary,
    SearchProfileSummary, SearchRequest, SearchResponse, SearchRoute, SessionMessage,
    SessionMessagePage, SessionSummary, SkillComposition, SkillDuplicate, SkillFileRead,
    SkillInspection, SkillInstallResult, SkillRecord, SkillResourceEntry, SkillResourceRead,
    SkillScaffoldResult, SkillValidationResult, SkillWriteResult, StartupVerificationMode,
    SubagentJob, SubagentQueueStatus, SubagentStatus, TaskRecord, TaskStatus, TelemetryMetrics,
    TerminalPreferences, ToolCall, ToolResult, ToolSpec, UserPromptRequest, WorkStateSnapshot,
    WorkflowWebhookDispatch,
};
use colossus_integrations::{
    EventSourcedExtensionRepository, IntegrationExecutor, IntegrationRequest,
};
use colossus_journal_postgres::{PostgresEventJournal, PostgresJournalConfig};
use colossus_journal_redb::{
    Ed25519CheckpointSigner, EnvironmentKeyProvider, PlatformKeyProvider, RedbEventJournal,
    RedbWriterLease, platform_secret,
};
use colossus_mcp::{
    MAX_MCP_PAGES, MAX_MCP_TOOLS, McpCallOutput, McpConfig, McpError, McpExecutor,
    McpOAuthCredentialStoreKind, McpOAuthLogin, McpOAuthStatus, McpOperation, McpServerConfig,
    McpServerSummary, McpToolSummary, McpToolsPage, validate_config as validate_mcp_config,
    validate_tool_arguments,
};
use colossus_memory::{
    EventSourcedMemoryRepository, MemoryIndexRegistration, MemoryService, TantivyMemoryIndex,
    UnavailableMemoryIndex,
};
use colossus_memory_chroma::{
    ChromaExecutor, ChromaMemoryIndex, ChromaProfile, GatewayOpenAiEmbeddingProvider,
    LocalHashEmbeddingProvider, OpenAiEmbeddingExecutor, OpenAiEmbeddingProfile,
};
use colossus_network::AdditionalRootCertificates;
use colossus_packs::{PackError, PackExecutor, PackOperation, PackService};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit,
    GatewayError, MIN_OCI_EFFECT_TIMEOUT_MS, MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS,
    MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS, OpaConfig, OpaPolicy, ReleasedEffectObserver,
    ReleasedEffectResult, SafetyKernel, canonical_network_origin, effect_request,
    network_destination_match, system_actor,
};
use colossus_ports::{
    ApprovalProvider, AuditExporter, ContextError, ContextPreparer, ContextRepository,
    EmbeddingProvider, EventJournal, ExtensionRepository, ExternalWorkQueue, KeyProvider,
    MemoryIndex, MemoryRepository, MemoryRetriever, ModelProvider, ModelProviderError,
    PolicyDecisionPoint, PresentationRepository, ProjectionStore, ProviderEventObserver,
    ProviderTurnOptions, ResearchRepository, RiskEvaluationError, RiskEvaluator, RunControl,
    RunEventObserver, SearchError, SearchProvider, SessionRepository, SkillRepository, StoreError,
    ToolError, ToolExecutor, ToolRegistry, UserPromptProvider, WorkRepository, WorkflowRepository,
};

const SESSION_MESSAGE_PAGE_LIMIT: usize = 100;
const SESSION_MESSAGE_PAGE_MAX_BYTES: usize = 2 * 1024 * 1024;
use colossus_presentation::EventSourcedPresentationRepository;
use colossus_projection::{
    JournalExternalWorkQueue, ProjectionRunReport, ProjectionWorker, default_handlers,
};
pub use colossus_provider::{
    CredentialResolver, EnvironmentCredentialResolver, HostCredentialResolver,
};
use colossus_provider::{
    ModelProfile, ProviderEffectInput, ProviderError, ProviderExecutor, ProviderKind,
    ProviderProfile, ProviderRegistry,
};
use colossus_research::{
    EventSourcedResearchRepository, ResearchCollection, ResearchCollector, ResearchLimits,
    ResearchModel, ResearchService, ResearchSourceDraft,
};
use colossus_sandbox::{
    FilesystemExecutor, HttpExecutor, ProcessSpec, SandboxDoctorReport, SandboxExecutorConfig,
    SandboxProcessExecutor, sandbox_doctor,
};
use colossus_search::{
    SearchAdapterError, SearchEffectInput, SearchExecutor, SearchKind, SearchProfile,
    SearchRegistry, default_search_limit,
};
use colossus_session::EventSourcedSessionRepository;
use colossus_skills::{
    FilesystemSkillRepository, SkillAuthoringService, SkillComposer, SkillResourceService,
    SkillRoot,
};
use colossus_telemetry::TelemetryService;
use colossus_tools::{StaticToolRegistry, ToolCatalogError, builtin_specs};
use colossus_work::{EventSourcedWorkRepository, WorkService};
use colossus_workflow::{
    EventSourcedWorkflowRepository, ValidatedWorkflow, WorkflowEffect, WorkflowEffectRunner,
    WorkflowError, WorkflowService, validate_definition,
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::{Mutex as TokioMutex, Notify};
use tokio::task::JoinSet;
use url::Url;
use uuid::Uuid;

mod agent_runs;
mod agent_tools;
mod composition;
mod config;
mod context_tools;
mod development_sandbox;
mod diagnostics;
mod direct_effects;
mod error;
mod extensions;
mod gateway_tool_dispatch;
mod gateway_tool_helpers;
mod generic_effects;
mod memory;
mod memory_gateway;
mod operations;
mod pack_extensions;
mod pack_process;
mod presentation_work_effects;
mod provider_gateway;
mod repository_tools;
mod research_gateway;
mod research_skill_effects;
mod runtime_helpers;
mod services;
mod sessions_context;
mod subagents;
mod tool_arguments;
mod trace_tools;
mod work;
mod workflows_research;
mod workspace;
mod workspace_binding;
mod workspace_lease;

pub use colossus_contracts::ModelCapabilities;
pub use composition::Runtime;
pub use config::{
    AgentConfig, AuditConfig, AuditExporterConfig, KeyConfig, MemoryConfig, MemoryEmbeddingConfig,
    ModelProfileConfig, ModelsConfig, NetworkConfig, PacksConfig, PolicyConfig,
    ProviderProfileConfig, ProvidersConfig, ResearchConfig, ResearchSearchConfig, RuntimeConfig,
    SandboxConfig, SearchConfig, SearchProfileConfig, SemanticMemoryConfig, SkillsConfig,
    StorageAdapter, StorageConfig, SubagentConfig, WorkflowLibraryConfig,
};
pub use diagnostics::format_provider_response_diagnostic;
pub use error::RuntimeError;
pub use workspace::RuntimeOpenOptions;
pub use workspace_lease::WorkspaceIdentityToken;

use agent_tools::*;
use config::*;
use context_tools::*;
use development_sandbox::*;
use error::{explicit_secret, read_optional};
use gateway_tool_helpers::*;
use generic_effects::*;
use memory_gateway::*;
use operations::*;
use pack_extensions::*;
use pack_process::*;
use presentation_work_effects::*;
use provider_gateway::*;
use repository_tools::*;
use research_gateway::*;
use research_skill_effects::*;
use runtime_helpers::*;
use tool_arguments::*;
use trace_tools::*;
use workspace_binding::*;

#[cfg(test)]
mod tests;
