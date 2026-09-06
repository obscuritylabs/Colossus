//! Runtime composition root. Interfaces call this layer and own no product logic.

pub use colossus_media::ValidatedImage;

const SESSION_MESSAGE_PAGE_LIMIT: usize = 100;
const SESSION_MESSAGE_PAGE_MAX_BYTES: usize = 2 * 1024 * 1024;
const RUN_INPUT_FILE_READ_ACTION: &str = "filesystem.read_run_input";
pub use colossus_provider::{
    ChatCompletionsOutputTokenParameter, CodexAuthStore, CredentialResolver,
    EnvironmentCredentialResolver, HostCredentialResolver,
};
mod access_policy;
mod adapter_composition;
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
mod goal_runs;
mod instruction_snapshots;
mod memory;
mod memory_gateway;
mod operations;
mod plan_runs;
#[cfg(test)]
mod plugin_authorization_tests;
mod plugin_catalog;
#[cfg(test)]
mod plugin_catalog_tests;
mod plugin_extensions;
mod plugin_inventory;
mod plugin_management;
mod plugin_registry_effects;
#[cfg(test)]
mod plugin_registry_tests;
mod prelude;
mod presentation_work_effects;
mod provider_gateway;
mod repository_tools;
mod research_gateway;
mod research_skill_effects;
mod runtime_helpers;
mod sandbox_boundary;
mod security_posture;
mod services;
mod session_activity;
mod sessions_context;
mod storage_composition;
mod subagents;
mod tool_arguments;
mod trace_tools;
mod work;
mod workflows_research;
mod workspace;
mod workspace_binding;
mod workspace_lease;

use prelude::*;

use access_policy::*;
use adapter_composition::*;

#[cfg(test)]
mod test_support;

pub use colossus_contracts::{ModelCapabilities, ReasoningEffort};
pub use colossus_observability::{
    JournalPayloadMode, LogSignalConfig, MetricSignalConfig, ObservabilityConfig, OtlpConfig,
    OtlpProtocol, TraceSignalConfig,
};
pub use composition::Runtime;
pub use config::{
    AgentConfig, AuditConfig, AuditExporterConfig, BundlesConfig, KeyConfig, MemoryConfig,
    MemoryEmbeddingConfig, ModelProfileConfig, ModelsConfig, NetworkConfig, PluginMcpServerConfig,
    PluginsConfig, PolicyConfig, ProviderProfileConfig, ProvidersConfig, ResearchConfig,
    RuntimeConfig, SandboxConfig, SearchConfig, SearchProfileConfig, SemanticMemoryConfig,
    StorageAdapter, StorageConfig, StorageLocation, SubagentConfig, WorkflowLibraryConfig,
};
pub use diagnostics::format_provider_response_diagnostic;
pub use error::RuntimeError;
pub use workflows_research::ResearchRunContext;
pub use workspace::RuntimeOpenOptions;
pub use workspace_lease::WorkspaceIdentityToken;

use agent_runs::*;
use agent_tools::*;
use config::*;
use context_tools::*;
use development_sandbox::*;
use error::{explicit_secret, read_optional};
use gateway_tool_helpers::*;
use generic_effects::*;
use instruction_snapshots::*;
use memory_gateway::*;
use operations::*;
use plugin_catalog::*;
use plugin_extensions::*;
use plugin_registry_effects::*;
use presentation_work_effects::*;
use provider_gateway::*;
use repository_tools::*;
use research_gateway::*;
use research_skill_effects::*;
use runtime_helpers::*;
use storage_composition::*;
use tool_arguments::*;
use trace_tools::*;
use workspace_binding::*;

#[cfg(test)]
mod tests;
