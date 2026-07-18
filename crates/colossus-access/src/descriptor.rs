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
    /// Enabled and reverified signed pack capability.
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
    /// At least one read-capable filesystem root.
    FilesystemRead,
    /// At least one write-capable filesystem root.
    FilesystemWrite,
    /// Exactly one configured Git executable.
    GitExecutable,
    /// At least one exact executable.
    AnyExecutable,
    /// At least one exact network origin.
    NetworkDestination,
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
    /// At least one read-capable filesystem root is configured.
    pub filesystem_read: bool,
    /// At least one write filesystem root is configured.
    pub filesystem_write: bool,
    /// Exactly one configured executable is Git.
    pub git_executable: bool,
    /// At least one exact executable is configured.
    pub any_executable: bool,
    /// At least one exact network origin is configured.
    pub network_destination: bool,
    /// A valid agent search route is configured.
    pub agent_search_route: bool,
    /// A trusted interactive prompt interface is present.
    pub interactive: bool,
    /// At least one trusted MCP server is configured.
    pub mcp_configured: bool,
}
