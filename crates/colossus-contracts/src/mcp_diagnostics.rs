//! Payload-free evidence for an explicit local MCP health check.

use serde::{Deserialize, Serialize};

/// Last locally observed step of an MCP health check.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiagnosticStage {
    /// Resolve the configured server and authorize discovery.
    #[default]
    Configuration,
    /// Construct the pinned client and resolve the destination.
    ClientSetup,
    /// Resolve configured host credentials or OAuth credentials.
    Credentials,
    /// Execute a stdio subprocess and collect its bounded protocol output.
    Process,
    /// Negotiate an MCP session.
    Initialize,
    /// Discover allowlisted tools.
    ListTools,
    /// Validate and release the bounded tool inventory.
    ValidateResponse,
    /// Discovery completed successfully.
    Complete,
}

/// Allowlisted failure category; never contains remote text or a source error.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiagnosticCode {
    /// Server configuration is absent or invalid.
    Configuration,
    /// Policy, approval, or safety validation rejected discovery.
    Policy,
    /// Required credentials could not be resolved.
    Credentials,
    /// The MCP subprocess could not run or exceeded a resource limit.
    Process,
    /// DNS failed or returned no permitted addresses.
    Dns,
    /// A direct connection could not be established.
    Connect,
    /// TLS negotiation or certificate verification failed.
    Tls,
    /// The bounded request or operation timed out.
    Timeout,
    /// An HTTP server returned a non-success status.
    HttpStatus,
    /// MCP negotiation or response framing failed.
    Protocol,
    /// A response exceeded the authorized byte bound.
    ResponseTooLarge,
    /// An HTTP request or response stream failed without a more specific cause.
    Transport,
    /// Audit or other runtime infrastructure failed.
    Runtime,
}

/// Sanitized transport failure metadata from one health check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpDiagnosticFailure {
    /// Fixed, locally classified category.
    pub code: McpDiagnosticCode,
    /// Numeric HTTP status only, without headers or body.
    pub http_status: Option<u16>,
}

/// Effective transport selected by the running worker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiagnosticTransport {
    /// Colossus owns the direct HTTP connection and additional CA roots.
    StreamableHttp,
    /// The child process owns its networking and TLS configuration.
    Stdio,
}

/// Comparable, bounded runtime configuration without paths, endpoints, or secrets.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpDiagnosticConfiguration {
    /// Actual runtime version, not the desktop shell version.
    pub runtime_version: String,
    /// Transport implementation used by this server.
    pub transport: McpDiagnosticTransport,
    /// SHA-256 of the configured endpoint; never the endpoint itself.
    pub endpoint_sha256: Option<String>,
    /// Number of additional roots loaded by this runtime.
    pub additional_ca_certificates: usize,
    /// SHA-256 of sorted DER certificate fingerprints (independent of PEM formatting).
    pub additional_ca_sha256: Option<String>,
    /// Whether the Colossus HTTP client explicitly bypasses proxies.
    pub direct_http: bool,
    /// Number of configured credential headers, without names or values.
    pub credential_headers: usize,
    /// Whether the server uses an OAuth overlay.
    pub oauth: bool,
    /// Whether a server without session IDs is accepted.
    pub allow_stateless: bool,
    /// Configured server deadline in milliseconds; policy may tighten it.
    pub configured_timeout_ms: Option<u64>,
}

/// Status-only evidence produced by real, authorized MCP discovery.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpHealthReport {
    /// True only after the complete tool inventory passes validation and release.
    pub healthy: bool,
    /// Total elapsed duration, including configuration and policy checks.
    pub elapsed_ms: u64,
    /// Last locally observed stage.
    pub stage: McpDiagnosticStage,
    /// Safe failure metadata, absent on success.
    pub failure: Option<McpDiagnosticFailure>,
    /// Configuration from the actual worker, absent if it could not be resolved.
    pub configuration: Option<McpDiagnosticConfiguration>,
}
