use super::*;

/// Canonical Agent Plugins v1 manifest schema identifier.
pub const AGENT_PLUGIN_SCHEMA_V1: &str =
    "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";

/// Canonical Agent Plugins v1 MCP schema identifier.
pub const AGENT_PLUGIN_MCP_SCHEMA_V1: &str =
    "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";

/// OCI artifact type used for one complete Agent Plugin directory.
pub const AGENT_PLUGIN_ARTIFACT_TYPE: &str = "application/vnd.colossus.agent-plugin.v1";

/// OCI config media type used by the Colossus Agent Plugin distribution profile.
pub const AGENT_PLUGIN_CONFIG_MEDIA_TYPE: &str =
    "application/vnd.colossus.agent-plugin.config.v1+json";

/// OCI layer media type used by the Colossus Agent Plugin distribution profile.
pub const AGENT_PLUGIN_LAYER_MEDIA_TYPE: &str =
    "application/vnd.colossus.agent-plugin.content.v1.tar+gzip";

/// Optional Agent Plugin author metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPluginAuthor {
    /// Human-readable author name.
    #[serde(default)]
    pub name: Option<String>,
    /// Author contact address.
    #[serde(default)]
    pub email: Option<String>,
    /// Author-supplied URL. Agent Plugins treats this as an opaque string.
    #[serde(default)]
    pub url: Option<String>,
}

/// Parsed Agent Plugins v1 root `plugin.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentPluginManifest {
    /// Canonical Agent Plugins schema identifier.
    #[serde(rename = "$schema")]
    pub schema: String,
    /// Stable plugin name.
    pub name: String,
    /// Optional author-declared version.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional discovery description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional author information.
    #[serde(default)]
    pub author: Option<AgentPluginAuthor>,
    /// Optional homepage string.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Optional source repository string.
    #[serde(default)]
    pub repository: Option<String>,
    /// Optional license identifier or description.
    #[serde(default)]
    pub license: Option<String>,
    /// Optional discovery keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Client-specific data. Colossus does not assign semantics to these values.
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

/// Standard Agent Skills frontmatter contained in an Agent Plugin.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AgentSkillManifest {
    /// Skill name, matching its parent directory.
    pub name: String,
    /// Discovery description explaining what the skill does and when to use it.
    pub description: String,
    /// Optional license name or bundled-file reference.
    #[serde(default)]
    pub license: Option<String>,
    /// Optional compatibility description.
    #[serde(default)]
    pub compatibility: Option<String>,
    /// Arbitrary string metadata defined by Agent Skills.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Experimental advisory tool list. It never expands Colossus authority.
    #[serde(default)]
    pub allowed_tools: Option<String>,
}

/// One skill discovered from an installed Agent Plugin.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSkillRecord {
    /// Canonical `<plugin>/<skill>` identifier.
    pub id: String,
    /// Owning plugin name.
    pub plugin: String,
    /// Validated Agent Skills frontmatter.
    pub manifest: AgentSkillManifest,
    /// Markdown instructions after frontmatter.
    pub instructions: String,
    /// Absolute, filesystem-resolved skill root.
    pub root: String,
}

/// Metadata disclosed for progressive skill discovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSkillMetadata {
    /// Canonical `<plugin>/<skill>` identifier.
    pub id: String,
    /// Owning plugin.
    pub plugin: String,
    /// Agent Skill name.
    pub name: String,
    /// Agent Skill description.
    pub description: String,
    /// Optional compatibility note.
    pub compatibility: Option<String>,
    /// Experimental advisory tool expression.
    pub allowed_tools: Option<String>,
}

/// Supported portable Agent Plugin MCP transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginMcpTransport {
    /// Stdio subprocess transport.
    Stdio,
    /// Streamable HTTP transport.
    StreamableHttp,
    /// Legacy HTTP+SSE transport, retained only for an unsupported diagnostic.
    Sse,
}

/// Validated portable MCP server declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMcpServer {
    /// Canonical `<plugin>/<server>` identity.
    pub id: String,
    /// Server name from `mcp.json`.
    pub name: String,
    /// Declared transport.
    pub transport: PluginMcpTransport,
    /// Optional stdio command token.
    pub command: Option<String>,
    /// Literal stdio arguments.
    pub args: Vec<String>,
    /// Literal stdio environment before reserved values are installed.
    pub environment: BTreeMap<String, String>,
    /// Optional expanded working directory.
    pub working_directory: Option<String>,
    /// Optional remote endpoint.
    pub url: Option<String>,
    /// Non-secret literal remote headers.
    pub headers: BTreeMap<String, String>,
}

/// Scope at which one component diagnostic applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginComponentKind {
    /// Root plugin manifest or package.
    Plugin,
    /// Agent Skill component.
    Skill,
    /// MCP configuration or server.
    McpServer,
}

/// Bounded non-sensitive component diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginComponentDiagnostic {
    /// Component family.
    pub kind: PluginComponentKind,
    /// Optional component name.
    pub name: Option<String>,
    /// Stable diagnostic code.
    pub code: String,
    /// Bounded remediation detail without component file contents.
    pub detail: String,
}

/// Current machine-scoped plugin lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginStatus {
    /// Installed content is retained but unavailable to workspaces.
    Disabled,
    /// This digest is the globally active version for its plugin name.
    Enabled,
    /// Installation history remains but content is no longer registered.
    Uninstalled,
}

/// Supply-chain verification outcome retained with an installation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTrustEvidence {
    /// Whether a configured signature policy authenticated the digest.
    pub trusted: bool,
    /// Trust profile used for verification, if any.
    pub profile: Option<String>,
    /// Public signer identity or key fingerprint, if verified.
    pub signer: Option<String>,
    /// Stable verification method such as `sigstore-key` or `digest-only`.
    pub method: String,
}

/// One installed Agent Plugin digest and its current lifecycle state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginOrigin {
    /// Installed by an operator from a directory or OCI source.
    #[default]
    Installed,
    /// Compiled into the Colossus executable and managed by its bootstrap.
    Bundled,
}

/// One installed Agent Plugin digest and its current lifecycle state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallation {
    /// Trusted installation provenance; never read from the portable manifest.
    #[serde(default)]
    pub origin: PluginOrigin,
    /// Validated portable manifest.
    pub manifest: AgentPluginManifest,
    /// Canonical `sha256:<hex>` OCI manifest digest.
    pub digest: String,
    /// Original local or registry source without credentials.
    pub source: String,
    /// Absolute immutable extracted plugin root.
    pub root: String,
    /// Current machine-scoped status.
    pub status: PluginStatus,
    /// Retained trust result.
    pub trust: PluginTrustEvidence,
    /// Installation time in RFC3339 UTC.
    pub installed_at: String,
    /// Latest lifecycle time in RFC3339 UTC.
    pub updated_at: String,
}

/// A fully discovered plugin snapshot used by one runtime.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPluginRecord {
    /// Installed lifecycle metadata.
    pub installation: PluginInstallation,
    /// Valid Agent Skills in deterministic order.
    pub skills: Vec<PluginSkillRecord>,
    /// Valid and supported MCP declarations in deterministic order.
    pub mcp_servers: Vec<PluginMcpServer>,
    /// Narrow component failures that did not invalidate the plugin.
    pub diagnostics: Vec<PluginComponentDiagnostic>,
}

/// Bounded plugin discovery view that excludes skill instructions and filesystem roots.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMcpInventoryEntry {
    /// Qualified server identity.
    pub id: String,
    /// Portable server name.
    pub name: String,
    /// Portable transport.
    pub transport: PluginMcpTransport,
    /// Explicitly enabled by this workspace's runtime configuration.
    pub enabled: bool,
    /// Availability or configuration status without credentials or command contents.
    pub status: String,
}

/// Bounded plugin discovery view that excludes skill instructions and filesystem roots.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInventoryEntry {
    /// Trusted installation provenance.
    #[serde(default)]
    pub origin: PluginOrigin,
    /// Whether this digest is available to a new run in the selected workspace.
    #[serde(default)]
    pub available: bool,
    /// Why an installed plugin is unavailable, when applicable.
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    /// Operator actions supported for this installation.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Validated portable plugin manifest.
    pub manifest: AgentPluginManifest,
    /// Canonical OCI manifest digest.
    pub digest: String,
    /// Credential-free installation source.
    pub source: String,
    /// Current global activation state.
    pub status: PluginStatus,
    /// Retained supply-chain trust result.
    pub trust: PluginTrustEvidence,
    /// Discovery-only Agent Skill metadata.
    pub skills: Vec<PluginSkillMetadata>,
    /// Portable MCP metadata; tools remain hidden until explicitly enabled.
    pub mcp_servers: Vec<PluginMcpInventoryEntry>,
    /// Independent component diagnostics.
    pub diagnostics: Vec<PluginComponentDiagnostic>,
}

impl AgentPluginRecord {
    /// Create the bounded progressive-disclosure inventory view for this plugin.
    #[must_use]
    pub fn inventory(&self) -> PluginInventoryEntry {
        PluginInventoryEntry {
            origin: self.installation.origin,
            available: self.installation.status == PluginStatus::Enabled,
            unavailable_reason: (self.installation.status != PluginStatus::Enabled)
                .then(|| "Plugin is not enabled globally".into()),
            actions: match self.installation.origin {
                PluginOrigin::Bundled => vec!["inspect", "verify", "export", "enable", "disable"],
                PluginOrigin::Installed => vec![
                    "inspect",
                    "verify",
                    "export",
                    "enable",
                    "disable",
                    "update",
                    "uninstall",
                ],
            }
            .into_iter()
            .map(str::to_owned)
            .collect(),
            manifest: self.installation.manifest.clone(),
            digest: self.installation.digest.clone(),
            source: self.installation.source.clone(),
            status: self.installation.status,
            trust: self.installation.trust.clone(),
            skills: self
                .skills
                .iter()
                .map(|skill| PluginSkillMetadata {
                    id: skill.id.clone(),
                    plugin: skill.plugin.clone(),
                    name: skill.manifest.name.clone(),
                    description: skill.manifest.description.clone(),
                    compatibility: skill.manifest.compatibility.clone(),
                    allowed_tools: skill.manifest.allowed_tools.clone(),
                })
                .collect(),
            mcp_servers: self
                .mcp_servers
                .iter()
                .map(|server| PluginMcpInventoryEntry {
                    id: server.id.clone(),
                    name: server.name.clone(),
                    transport: server.transport,
                    enabled: false,
                    status: "Requires explicit runtime enablement".into(),
                })
                .collect(),
            diagnostics: self.diagnostics.clone(),
        }
    }
}

/// Result of deterministic plugin-context composition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginComposition {
    /// Original instructions plus selected Agent Skill instructions.
    pub instructions: String,
    /// Metadata for every available skill.
    pub available_skills: Vec<PluginSkillMetadata>,
    /// Metadata for skills activated for the run.
    pub active_skills: Vec<PluginSkillMetadata>,
    /// Filesystem-resolved roots whose content may be read by selected skills.
    pub active_plugin_roots: Vec<String>,
}

/// One contained Agent Skill resource.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginResourceEntry {
    /// Canonical skill identifier.
    pub skill_id: String,
    /// POSIX path relative to the skill root.
    pub path: String,
    /// Exact file size.
    pub size: u64,
    /// Whether the content is eligible for bounded UTF-8 preview.
    pub text: bool,
}

/// One bounded UTF-8 Agent Skill resource read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginResourceRead {
    /// Canonical skill identifier.
    pub skill_id: String,
    /// POSIX path relative to the skill root.
    pub path: String,
    /// Exact byte length.
    pub size: u64,
    /// Released UTF-8 content.
    pub content: String,
}

/// Result of validating one Agent Plugin directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginValidation {
    /// Validated manifest.
    pub manifest: AgentPluginManifest,
    /// Deterministic regular-file count.
    pub file_count: usize,
    /// Sum of regular-file sizes.
    pub total_bytes: u64,
    /// SHA-256 of deterministic plugin content metadata.
    pub content_sha256: String,
    /// Discovered component diagnostics.
    pub diagnostics: Vec<PluginComponentDiagnostic>,
}

/// OCI descriptor used by the Agent Plugin distribution profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OciDescriptor {
    /// Descriptor media type.
    pub media_type: String,
    /// Canonical digest.
    pub digest: String,
    /// Exact blob size.
    pub size: u64,
    /// Optional OCI annotations.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// OCI image manifest carrying one complete Agent Plugin.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPluginOciManifest {
    /// OCI schema version, always `2`.
    pub schema_version: u8,
    /// OCI image-manifest media type.
    pub media_type: String,
    /// Colossus Agent Plugin artifact type.
    pub artifact_type: String,
    /// Plugin config descriptor.
    pub config: OciDescriptor,
    /// Exactly one plugin content layer.
    pub layers: Vec<OciDescriptor>,
    /// Optional OCI annotations.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// Small non-executable OCI config duplicating portable plugin identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPluginOciConfig {
    /// Colossus OCI profile version, currently `1`.
    pub schema_version: u8,
    /// Exact plugin name expected in the extracted `plugin.json`.
    pub name: String,
    /// Optional plugin version expected in the extracted `plugin.json`.
    pub version: Option<String>,
    /// Agent Plugins schema targeted by the portable payload.
    pub plugin_schema: String,
}
