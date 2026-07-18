use super::*;

/// Integration protocol or adapter family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    /// Built-in typed connector.
    Native,
    /// Imported JSON OpenAPI operations.
    OpenApi,
    /// Configured Model Context Protocol server.
    Mcp,
}

/// Connection readiness reconstructed from canonical events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStatus {
    /// Valid and available to the dynamic tool registry.
    Connected,
    /// Structurally valid but its credential reference is currently unresolved.
    PendingAuth,
    /// Explicitly disconnected and hidden from the tool registry.
    Disconnected,
}

/// Credential placement performed only by the permit-bearing adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntegrationAuth {
    /// No credential is required.
    None,
    /// Send a bearer-like authorization value.
    Bearer {
        /// Header name, normally `Authorization`.
        header: String,
        /// Scheme prefix, normally `Bearer`.
        scheme: String,
    },
    /// Send the secret in a configured header with an optional scheme prefix.
    ApiKey {
        /// Header name.
        header: String,
        /// Optional value prefix.
        scheme: Option<String>,
    },
    /// Construct an HTTP Basic authorization value from named username/password refs.
    Basic {
        /// Header name, normally `Authorization`.
        header: String,
    },
    /// Send a service-account value in a configured header.
    ServiceAccount {
        /// Header name.
        header: String,
    },
}

/// One compiled integration operation and its strict model-visible schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationOperation {
    /// Dynamic namespaced tool specification.
    pub tool: ToolSpec,
    /// Stable source operation identifier.
    pub operation_id: String,
    /// Uppercase HTTP method.
    pub method: String,
    /// Relative path template.
    pub path: String,
    /// Arguments substituted into path placeholders.
    pub path_parameters: Vec<String>,
    /// Arguments encoded into the query string.
    pub query_parameters: Vec<String>,
    /// Whether an optional or required `body` argument is supported.
    pub accepts_body: bool,
}

/// Canonical integration connection state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationConnection {
    /// Stable lowercase connection name.
    pub name: String,
    /// Protocol or adapter family.
    pub kind: IntegrationKind,
    /// Current readiness.
    pub status: IntegrationStatus,
    /// Human-facing title.
    pub title: String,
    /// Bounded description.
    pub description: String,
    /// Canonical API base URL without credentials, query, or fragment.
    pub base_url: String,
    /// Adapter-only credential placement.
    pub auth: IntegrationAuth,
    /// Local credential handle, never its value.
    pub credential_reference: Option<String>,
    /// Named credential handles such as OpenSearch username/password, never values.
    #[serde(default)]
    pub credential_references: std::collections::BTreeMap<String, String>,
    /// Declared authorization scopes.
    pub scopes: Vec<String>,
    /// Compiled operations hidden unless status is connected.
    pub operations: Vec<IntegrationOperation>,
    /// SHA-256 of the imported source schema or native manifest.
    pub manifest_sha256: String,
    /// Original creation timestamp.
    pub connected_at: String,
    /// Last lifecycle event timestamp.
    pub updated_at: String,
}

/// Safe connection summary for CLI and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationSummary {
    /// Connection name.
    pub name: String,
    /// Protocol family.
    pub kind: IntegrationKind,
    /// Current readiness.
    pub status: IntegrationStatus,
    /// Human-facing title.
    pub title: String,
    /// Credential handle without a value.
    pub credential_reference: Option<String>,
    /// Named credential handles without values.
    pub credential_references: std::collections::BTreeMap<String, String>,
    /// Dynamic tool names.
    pub tools: Vec<String>,
    /// Last lifecycle timestamp.
    pub updated_at: String,
}
