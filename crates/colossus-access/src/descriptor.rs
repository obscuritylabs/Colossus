use serde::Serialize;

/// Stable risk/behavior class used by live profiles.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    /// Configured model-provider transport.
    Provider,
    /// Read-only local or quarantined retrieval.
    Read,
    /// Colossus-owned canonical or disposable state changes.
    LocalState,
    /// Workspace or operator-visible filesystem mutation.
    WorkspaceMutation,
    /// Process, workflow, or executable code execution.
    Execution,
    /// Non-provider network or connected external-system access.
    ExternalNetwork,
    /// Installation, trust, registry, or audit administration.
    Administration,
}

/// Provenance class controlling whether a capability may participate in live profiles.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    /// First-party runtime capability.
    Core,
    /// Canonically connected integration operation.
    Integration,
    /// Explicitly configured and allowlisted MCP capability.
    Mcp,
    /// Enabled and verified Agent Plugin capability.
    SignedPack,
}

/// One action/capability identity and its stable profile classification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActionDescriptor {
    /// Exact effect action or Safety Kernel capability identity.
    pub name: String,
    /// Stable action class.
    pub class: ActionClass,
    /// Trusted source.
    pub source: CapabilitySource,
}

impl ActionDescriptor {
    /// Construct one descriptor.
    pub fn new(name: impl Into<String>, class: ActionClass, source: CapabilitySource) -> Self {
        Self {
            name: name.into(),
            class,
            source,
        }
    }
}

/// Static prerequisite that can hide an inherited tool.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPrerequisite {
    /// Read-capable filesystem authority is declared or ambient.
    FilesystemRead,
    /// Write-capable filesystem authority is declared or ambient.
    FilesystemWrite,
    /// Git resolution is available from a declaration or ambient process authority.
    GitExecutable,
    /// Executable resolution is available from a declaration or ambient process authority.
    AnyExecutable,
    /// Network authority is available from a declaration or ambient authority.
    NetworkDestination,
    /// Trusted host composition permits general model-visible network tools.
    ModelNetworkTools,
    /// A valid agent search route.
    AgentSearchRoute,
    /// A trusted interactive interface for this run.
    Interactive,
    /// At least one configured and allowlisted MCP server.
    McpConfigured,
}

/// One model-visible tool's source, family, and static prerequisites.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolDescriptor {
    /// Exact model-visible tool name.
    pub name: String,
    /// Stable diagnostic family.
    pub family: String,
    /// Trusted capability source.
    pub source: CapabilitySource,
    /// Static availability requirements.
    pub prerequisites: Vec<ToolPrerequisite>,
}

impl ToolDescriptor {
    /// Construct one trusted tool descriptor.
    pub fn new(
        name: impl Into<String>,
        family: impl Into<String>,
        source: CapabilitySource,
        prerequisites: Vec<ToolPrerequisite>,
    ) -> Self {
        Self {
            name: name.into(),
            family: family.into(),
            source,
            prerequisites,
        }
    }
}

/// Runtime facts used only for availability; they never grant an effect.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccessContext {
    /// Declared or acknowledged ambient filesystem read authority is available.
    pub filesystem_read: bool,
    /// Declared or acknowledged ambient filesystem write authority is available.
    pub filesystem_write: bool,
    /// Git is declared or ambient executable resolution is available.
    pub git_executable: bool,
    /// An exact declaration or ambient executable resolution is available.
    pub any_executable: bool,
    /// A declared destination or acknowledged ambient network authority is available.
    pub network_destination: bool,
    /// General model-visible network tools are enabled by trusted host composition.
    pub model_network_tools: bool,
    /// A valid agent search route is configured.
    pub agent_search_route: bool,
    /// A trusted interactive prompt interface is present.
    pub interactive: bool,
    /// At least one trusted MCP server is configured.
    pub mcp_configured: bool,
}
