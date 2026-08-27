//! Private, bounded bootstrap frames shared by the native SDK launcher and sidecar host.
//!
//! This is not a network protocol. Frames travel only over anonymous handles inherited
//! by a freshly verified child process. Secret fields are zeroized and redact debug
//! output; callers must also zeroize encoded frames after writing them.

#![allow(clippy::missing_errors_doc)]

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{Read, Write},
    path::{Component, Path},
};
use thiserror::Error;
use url::{Host, Url};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Exact bootstrap protocol version.
pub const PROTOCOL_VERSION: u16 = 9;
/// Exact desktop-to-TUI inherited-channel protocol version.
pub const DESKTOP_TUI_PROTOCOL_VERSION: u16 = 2;
/// Fixed child descriptor from which the bundled TUI reads native authentication.
pub const DESKTOP_TUI_AUTH_INPUT_FD: i32 = 3;
/// Fixed child descriptor to which the bundled TUI writes authentication responses.
pub const DESKTOP_TUI_AUTH_OUTPUT_FD: i32 = 4;
/// Maximum serialized frame size, excluding the four-byte length prefix.
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
/// Maximum host-provided credentials accepted by one managed runtime.
pub const MAX_HOST_CREDENTIALS: usize = 64;
/// Maximum provider connections in one app-managed runtime.
pub const MAX_MANAGED_PROVIDERS: usize = 16;
/// Maximum explicit model profiles in one app-managed runtime.
pub const MAX_MANAGED_MODELS: usize = 64;
/// Maximum explicit search profiles in one app-managed runtime.
pub const MAX_MANAGED_SEARCH_PROFILES: usize = 16;
/// Maximum MCP server definitions in one app-managed runtime.
pub const MAX_MANAGED_MCP_SERVERS: usize = 64;
/// Maximum sparse configuration overrides accepted by one managed runtime.
pub const MAX_MANAGED_FIELD_OVERRIDES: usize = 512;
/// Default request timeout for remote model providers.
pub const REMOTE_PROVIDER_TIMEOUT_MS: u64 = 300_000;
/// Default request timeout for providers hosted on the local loopback interface.
pub const LOOPBACK_PROVIDER_TIMEOUT_MS: u64 = 900_000;
const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_PRIVATE_PATH_BYTES: usize = 4_096;
const MAX_AUTHORIZATION_ITEMS: usize = 512;
const MAX_MANAGED_FIELD_ID_BYTES: usize = 160;
const MAX_MANAGED_FIELD_VALUE_BYTES: usize = 64 * 1024;
const MAX_MANAGED_FIELD_VALUE_DEPTH: usize = 16;
/// Maximum repository configuration accepted by private sidecar inspection.
pub const MAX_CONFIGURATION_INSPECTION_BYTES: usize = 1024 * 1024;
const MAX_CERTIFICATE_PEM_BYTES: usize = 256 * 1024;
const APPROVALS_RESPOND_SCOPE: &str = "approvals:respond";
const LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION: u16 = 1;
const LEGACY_UNIX_WORKSPACE_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-unix-device-inode-v1\0";
const MACOS_WORKSPACE_IDENTITY_VERSION: u16 = 2;
const MACOS_WORKSPACE_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-macos-device-inode-birthtime-v2\0";
const WINDOWS_WORKSPACE_IDENTITY_VERSION: u16 = 3;
const WINDOWS_WORKSPACE_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-windows-volume-file-id-v3\0";

/// Opaque identity of the exact workspace directory opened by the native parent.
///
/// The digest deliberately hides platform inode fields from the bootstrap schema while
/// still binding the child to the same kernel object. Persisted Managed Desktop state
/// uses the macOS device, inode, and descriptor-derived directory birthtime contract;
/// version 1 remains only for descriptor-lifetime SDK compatibility on other Unix hosts.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceIdentity {
    /// Exact identity derivation version.
    pub version: u16,
    /// Lowercase SHA-256 of the versioned, domain-separated platform identity fields.
    pub sha256: String,
    /// Records an omitted preview-era version so Desktop can migrate it without
    /// allowing the same ambiguous encoding across a private bootstrap channel.
    #[serde(skip)]
    version_was_missing: bool,
}

impl fmt::Debug for WorkspaceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkspaceIdentity")
            .field("version", &self.version)
            .field("sha256", &"[OPAQUE SHA-256]")
            .finish()
    }
}

impl<'de> Deserialize<'de> for WorkspaceIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireIdentity {
            #[serde(default)]
            version: Option<u16>,
            sha256: String,
        }

        let wire = WireIdentity::deserialize(deserializer)?;
        Ok(Self {
            version: wire
                .version
                .unwrap_or(LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION),
            sha256: wire.sha256,
            version_was_missing: wire.version.is_none(),
        })
    }
}

impl WorkspaceIdentity {
    /// Derive the legacy ephemeral Unix identity from an already-opened directory.
    ///
    /// Version 1 remains available for non-persisted SDK sidecar compatibility, but it
    /// is not a valid persisted Managed Desktop authority because inode values may be
    /// reused after the retaining process exits.
    pub fn from_unix_parts(device: u64, inode: u64) -> Self {
        let mut digest = Sha256::new();
        digest.update(LEGACY_UNIX_WORKSPACE_IDENTITY_DOMAIN);
        digest.update(device.to_le_bytes());
        digest.update(inode.to_le_bytes());
        Self {
            version: LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION,
            sha256: format!("{:x}", digest.finalize()),
            version_was_missing: false,
        }
    }

    /// Derive the current macOS identity from metadata read from one securely opened
    /// directory descriptor.
    pub fn from_macos_parts(
        device: u64,
        inode: u64,
        birth_seconds: i64,
        birth_nanoseconds: i64,
    ) -> Result<Self, ProtocolError> {
        if birth_seconds <= 0 || !(0..1_000_000_000).contains(&birth_nanoseconds) {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut digest = Sha256::new();
        digest.update(MACOS_WORKSPACE_IDENTITY_DOMAIN);
        digest.update(device.to_le_bytes());
        digest.update(inode.to_le_bytes());
        digest.update(birth_seconds.to_le_bytes());
        digest.update(birth_nanoseconds.to_le_bytes());
        Ok(Self {
            version: MACOS_WORKSPACE_IDENTITY_VERSION,
            sha256: format!("{:x}", digest.finalize()),
            version_was_missing: false,
        })
    }

    /// Derive the current Windows identity from `FileIdInfo` on a securely retained
    /// directory handle.
    pub fn from_windows_parts(
        volume_serial_number: u64,
        file_id: [u8; 16],
    ) -> Result<Self, ProtocolError> {
        if volume_serial_number == 0 || file_id == [0; 16] {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut digest = Sha256::new();
        digest.update(WINDOWS_WORKSPACE_IDENTITY_DOMAIN);
        digest.update(volume_serial_number.to_le_bytes());
        digest.update(file_id);
        Ok(Self {
            version: WINDOWS_WORKSPACE_IDENTITY_VERSION,
            sha256: format!("{:x}", digest.finalize()),
            version_was_missing: false,
        })
    }

    /// Validate a bounded wire identity. Version 1 is accepted only so an ephemeral
    /// non-macOS SDK sidecar can retain its existing descriptor-lifetime contract.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !self.version_was_missing
            && matches!(
                self.version,
                LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION
                    | MACOS_WORKSPACE_IDENTITY_VERSION
                    | WINDOWS_WORKSPACE_IDENTITY_VERSION
            )
            && lowercase_sha256(&self.sha256)
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidFrame)
        }
    }

    /// Require a non-reusable identity used by persisted Managed Desktop state.
    pub fn validate_current(&self) -> Result<(), ProtocolError> {
        if !self.version_was_missing
            && matches!(
                self.version,
                MACOS_WORKSPACE_IDENTITY_VERSION | WINDOWS_WORKSPACE_IDENTITY_VERSION
            )
            && lowercase_sha256(&self.sha256)
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidFrame)
        }
    }

    /// Whether this is a current macOS device/inode/birthtime identity.
    pub fn is_current_macos(&self) -> bool {
        !self.version_was_missing
            && self.version == MACOS_WORKSPACE_IDENTITY_VERSION
            && lowercase_sha256(&self.sha256)
    }

    /// Whether this is a current Windows volume/file-ID identity.
    pub fn is_current_windows(&self) -> bool {
        !self.version_was_missing
            && self.version == WINDOWS_WORKSPACE_IDENTITY_VERSION
            && lowercase_sha256(&self.sha256)
    }

    /// Whether this is a syntactically valid path-only preview identity that must be
    /// replaced by an explicit user folder selection.
    pub fn is_legacy_v1(&self) -> bool {
        self.version == LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION && lowercase_sha256(&self.sha256)
    }
}

/// A string secret that is erased on drop and always redacted from diagnostics.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Validate one non-empty bounded secret.
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains('\0') {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    /// Borrow the value only while delivering or consuming it at a trusted boundary.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// One host-resolved provider credential delivered only in bootstrap memory.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCredential {
    /// Opaque identifier referenced as `host:<id>` by runtime configuration.
    pub id: String,
    /// Credential value, erased on drop.
    pub secret: SecretString,
}

impl HostCredential {
    /// Validate a host credential and its opaque identifier.
    pub fn new(id: impl Into<String>, secret: SecretString) -> Result<Self, ProtocolError> {
        let value = Self {
            id: id.into(),
            secret,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ProtocolError> {
        if !valid_host_identifier(&self.id) {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

impl fmt::Debug for HostCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostCredential")
            .field("id", &self.id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

/// Exact application authority requested by the native host.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapGrant {
    /// Stable `app:<token>` application identity.
    pub application_id: String,
    /// Exact public API scopes.
    pub scopes: Vec<String>,
    /// Exact logical role ceiling.
    pub allowed_roles: Vec<String>,
    /// Exact tool ceiling.
    pub allowed_tools: Vec<String>,
}

/// Built-in access profile selected for an app-managed runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAccessProfile {
    /// Support and inspection tools only.
    Minimal,
    /// Workspace development tools with consequential actions approval-gated.
    Development,
    /// Broad trusted tools, still bounded by the safety kernel and sandbox.
    AllowAll,
    /// Deny-by-default profile; exact additions remain sidecar-owned configuration.
    Pinned,
}

/// Host execution boundary selected for an app-managed runtime.
///
/// This is independent from the access profile and approval interaction mode. Full
/// access is the compatibility default for app hosts, while the isolated variants
/// let callers explicitly opt into Colossus sandbox boundaries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedExecutionBoundary {
    /// Run with the ambient filesystem, environment, and network access of the host.
    #[default]
    FullAccess,
    /// Confine filesystem access to the workspace while retaining configured network.
    WorkspaceIsolated,
    /// Confine filesystem access to the workspace and disable general network access.
    OfflineIsolated,
}

/// Provider adapter selected by compact native onboarding state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedProviderKind {
    /// Deterministic credential-free offline self-test.
    Echo,
    /// OpenAI Responses API.
    OpenAiResponses,
    /// OpenAI-compatible chat completions API.
    OpenAiCompatible,
    /// Fixed ChatGPT Codex subscription backend using the official Codex credential store.
    OpenAiCodex,
}

/// Provider-neutral reasoning effort carried by app-managed model configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedReasoningEffort {
    /// Disable reasoning when supported.
    None,
    /// Use the smallest nonzero reasoning budget.
    Minimal,
    /// Prefer lower latency and lighter reasoning.
    Low,
    /// Balance latency and reasoning depth.
    Medium,
    /// Allocate more reasoning for complex work.
    High,
    /// Allocate extra-high reasoning.
    #[serde(rename = "xhigh")]
    XHigh,
    /// Allocate the model's maximum ordinary reasoning budget.
    Max,
    /// Use the Codex model's provider-defined ultra mode.
    Ultra,
}

/// Chat Completions field used by an app-managed OpenAI-compatible provider to carry
/// the canonical output-token limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedChatCompletionsOutputTokenParameter {
    /// Legacy and broadly compatible `max_tokens` field.
    MaxTokens,
    /// Modern `max_completion_tokens` field required by newer models.
    MaxCompletionTokens,
    /// Do not send an output-token limit field.
    Omit,
}

/// Compact provider connection settings that contain references but never credential values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedProviderConfig {
    /// Stable provider connection profile.
    pub profile: String,
    /// Selected first-party adapter.
    pub kind: ManagedProviderKind,
    /// API-version base URL for a network provider.
    pub base_url: Option<String>,
    /// Opaque host credential identifier without the `host:` prefix.
    pub credential_id: Option<String>,
    /// Per-request transport timeout.
    pub timeout_ms: u64,
    /// Optional Chat Completions output-token wire parameter for an OpenAI-compatible
    /// provider. Omission keeps the migration-safe `max_tokens` default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_completions_output_token_parameter: Option<ManagedChatCompletionsOutputTokenParameter>,
}

impl ManagedProviderConfig {
    /// Validate compact provider settings without resolving a secret or network name.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !valid_token(&self.profile)
            || self.timeout_ms == 0
            || self.base_url.as_ref().is_some_and(|url| url.len() > 2_048)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        // The output-token wire parameter shapes Chat Completions requests only, so any
        // other adapter must leave it unset rather than carry an inert setting.
        if self.chat_completions_output_token_parameter.is_some()
            && !matches!(self.kind, ManagedProviderKind::OpenAiCompatible)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        match self.kind {
            ManagedProviderKind::Echo => {
                if self.base_url.is_some() || self.credential_id.is_some() {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
            ManagedProviderKind::OpenAiResponses | ManagedProviderKind::OpenAiCompatible => {
                if self
                    .base_url
                    .as_deref()
                    .is_none_or(|url| validate_managed_provider_base_url(url).is_err())
                    || self
                        .credential_id
                        .as_deref()
                        .is_some_and(|credential_id| !valid_host_identifier(credential_id))
                {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
            ManagedProviderKind::OpenAiCodex => {
                if self.base_url.is_some() || self.credential_id.is_some() {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
        }
        Ok(())
    }
}

/// Explicit request-shaping capabilities for an app-managed model.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedModelCapabilities {
    /// Whether the model receives tools and structured tool history.
    pub tool_calls: bool,
    /// Whether the model uses provider streaming.
    pub streaming: bool,
    /// Whether the model receives verified encrypted run-input images.
    #[serde(default)]
    pub image_inputs: bool,
}

/// Compact explicit model metadata without provider credentials.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedModelConfig {
    /// Stable model profile.
    pub profile: String,
    /// Referenced provider connection profile.
    pub provider_profile: String,
    /// Exact model identifier.
    pub model: String,
    /// Total provider context window.
    pub context_window_tokens: u64,
    /// Maximum output allocation.
    pub max_output_tokens: u64,
    /// Explicit model capabilities.
    pub capabilities: ManagedModelCapabilities,
    /// Optional provider-neutral reasoning effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ManagedReasoningEffort>,
}

impl ManagedModelConfig {
    fn validate(&self) -> Result<(), ProtocolError> {
        let safety = self.context_window_tokens.div_ceil(10).max(512);
        if !valid_token(&self.profile)
            || !valid_token(&self.provider_profile)
            || validate_managed_model_identifier(&self.model).is_err()
            || self.context_window_tokens < 1_024
            || self.max_output_tokens == 0
            || self
                .context_window_tokens
                .checked_sub(self.max_output_tokens)
                .and_then(|remaining| remaining.checked_sub(safety))
                .is_none_or(|input| input == 0)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

/// Search adapter generated by an app-managed runtime.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSearchKind {
    /// Direct SearXNG JSON search endpoint.
    #[default]
    Searxng,
    /// Direct SerpAPI Google search endpoint.
    SerpApi,
}

/// Native-credential-backed search connection generated by an app-managed runtime.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedSearchConfig {
    /// Stable search profile identifier.
    pub profile: String,
    /// Search adapter kind.
    #[serde(default)]
    pub kind: ManagedSearchKind,
    /// Exact search endpoint.
    pub endpoint: String,
    /// Optional opaque native host credential identifier.
    #[serde(default)]
    pub credential_id: Option<String>,
    /// Optional exact API-key header for SearXNG.
    #[serde(default)]
    pub auth_header: Option<String>,
    /// Per-request transport timeout.
    pub timeout_ms: u64,
}

impl ManagedSearchConfig {
    /// Validate a bounded search profile without resolving its native credential.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let endpoint = Url::parse(&self.endpoint).map_err(|_| ProtocolError::InvalidFrame)?;
        if !valid_token(&self.profile)
            || self.timeout_ms == 0
            || self.timeout_ms > LOOPBACK_PROVIDER_TIMEOUT_MS
            || validate_managed_provider_base_url(&self.endpoint).is_err()
            || self
                .credential_id
                .as_deref()
                .is_some_and(|credential_id| !valid_host_identifier(credential_id))
        {
            return Err(ProtocolError::InvalidFrame);
        }
        match self.kind {
            ManagedSearchKind::Searxng => {
                if !endpoint.path().ends_with("/search")
                    || self.auth_header.as_deref().is_some_and(|header| {
                        header.is_empty()
                            || header.len() > 128
                            || !header
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    })
                {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
            ManagedSearchKind::SerpApi => {
                if self.credential_id.is_none() || self.auth_header.is_some() {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
        }
        Ok(())
    }
}

/// Transport selected for one managed MCP server.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedMcpTransport {
    /// Launch one exact executable through the runtime sandbox.
    Stdio,
    /// Connect to one exact MCP Streamable HTTP endpoint.
    StreamableHttp,
}

/// One secret HTTP header backed by an opaque native credential.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedMcpCredentialHeader {
    /// Optional authentication scheme such as `Bearer`.
    pub scheme: Option<String>,
    /// Opaque host credential identifier without the `host:` prefix.
    pub credential_id: String,
}

/// OAuth client metadata for one managed MCP server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedMcpOAuthConfig {
    /// Registered non-secret client identity.
    pub client_id: String,
    /// Optional opaque native client secret identifier.
    pub client_secret_credential_id: Option<String>,
    /// Exact registered loopback callback port.
    pub callback_port: u16,
    /// Explicit OAuth scopes.
    pub scopes: Vec<String>,
}

/// One allowlisted managed MCP research call template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedMcpResearchTool {
    /// Exact MCP tool name.
    pub tool: String,
    /// Optional source title.
    pub title: Option<String>,
    /// Structured arguments whose string leaves may contain `{query}`.
    pub arguments: Value,
}

/// Secret-free MCP definition generated by Desktop.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedMcpServerConfig {
    /// Stable server name.
    pub name: String,
    /// Local process or Streamable HTTP transport.
    pub transport: ManagedMcpTransport,
    /// Exact executable for stdio servers.
    pub command: Option<String>,
    /// Literal stdio arguments.
    pub args: Vec<String>,
    /// Optional absolute or workspace-relative working directory.
    pub working_directory: Option<String>,
    /// Child environment names mapped to opaque native credential identifiers.
    pub environment_credentials: BTreeMap<String, String>,
    /// Exact Streamable HTTP endpoint.
    pub url: Option<String>,
    /// Non-secret literal HTTP headers.
    pub headers: BTreeMap<String, String>,
    /// Secret HTTP headers resolved from the inherited credential channel.
    pub credential_headers: BTreeMap<String, ManagedMcpCredentialHeader>,
    /// Permit an explicitly configured remote server to omit session identifiers.
    pub allow_stateless: bool,
    /// Optional OAuth client metadata.
    pub oauth: Option<ManagedMcpOAuthConfig>,
    /// Exact allowed MCP tools or the sole wildcard `*`.
    pub allowed_tools: Vec<String>,
    /// Research collection templates.
    pub research_tools: Vec<ManagedMcpResearchTool>,
    /// Optional server-specific timeout.
    pub timeout_ms: Option<u64>,
    /// Optional server-specific output cap.
    pub max_output_bytes: Option<u64>,
}

impl ManagedMcpServerConfig {
    fn validate(&self) -> Result<(), ProtocolError> {
        if !valid_token(&self.name)
            || self.args.len() > 128
            || self.environment_credentials.len() > 128
            || self.headers.len() > 64
            || self.credential_headers.len() > 16
            || self.allowed_tools.len() > 256
            || self.research_tools.len() > 64
            || self.timeout_ms == Some(0)
            || self.max_output_bytes == Some(0)
            || self.allowed_tools.iter().any(|tool| !valid_token(tool))
            || (self.allowed_tools.iter().any(|tool| tool == "*") && self.allowed_tools.len() != 1)
            || self
                .environment_credentials
                .iter()
                .any(|(name, credential)| {
                    !valid_environment_name(name) || !valid_host_identifier(credential)
                })
            || self.credential_headers.iter().any(|(name, credential)| {
                !valid_http_header_name(name)
                    || !valid_host_identifier(&credential.credential_id)
                    || credential.scheme.as_deref().is_some_and(|scheme| {
                        scheme.is_empty()
                            || scheme.len() > 64
                            || !scheme.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
                            })
                    })
            })
            || self.headers.iter().any(|(name, value)| {
                !valid_http_header_name(name)
                    || value.is_empty()
                    || value.len() > 8 * 1024
                    || value.bytes().any(|byte| byte.is_ascii_control())
            })
            || self.research_tools.iter().any(|tool| {
                !valid_token(&tool.tool)
                    || tool.title.as_ref().is_some_and(|title| {
                        title.is_empty() || title.len() > 512 || title.chars().any(char::is_control)
                    })
                    || !tool.arguments.is_object()
                    || json_value_depth(&tool.arguments) > MAX_MANAGED_FIELD_VALUE_DEPTH
            })
        {
            return Err(ProtocolError::InvalidFrame);
        }
        if let Some(oauth) = &self.oauth
            && (oauth.client_id.is_empty()
                || oauth.client_id.len() > 1_024
                || oauth.client_id.chars().any(char::is_control)
                || oauth.callback_port == 0
                || oauth.scopes.len() > 32
                || oauth.scopes.iter().any(|scope| !valid_token(scope))
                || oauth
                    .client_secret_credential_id
                    .as_deref()
                    .is_some_and(|id| !valid_host_identifier(id)))
        {
            return Err(ProtocolError::InvalidFrame);
        }
        match self.transport {
            ManagedMcpTransport::Stdio => {
                if self.command.as_ref().is_none_or(|command| {
                    command.is_empty() || command.len() > MAX_PRIVATE_PATH_BYTES
                }) || self.url.is_some()
                    || !self.headers.is_empty()
                    || !self.credential_headers.is_empty()
                    || self.allow_stateless
                    || self.oauth.is_some()
                {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
            ManagedMcpTransport::StreamableHttp => {
                if self.command.is_some()
                    || !self.args.is_empty()
                    || self.working_directory.is_some()
                    || !self.environment_credentials.is_empty()
                    || self
                        .url
                        .as_deref()
                        .is_none_or(|url| validate_managed_provider_base_url(url).is_err())
                {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
        }
        Ok(())
    }
}

fn valid_environment_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn valid_http_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Validate an exact model identifier accepted by an app-managed runtime.
pub fn validate_managed_model_identifier(value: &str) -> Result<(), ProtocolError> {
    if valid_token(value) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidFrame)
    }
}

/// Validate a network provider URL accepted by a managed runtime.
///
/// Provider credentials must be carried only by the inherited secret channel, so
/// user information, query strings, and fragments are rejected even when a URL
/// parser would otherwise accept them. Plain HTTP is limited to an actual loopback
/// host for local provider development.
pub fn validate_managed_provider_base_url(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > 2_048
        || value.chars().any(char::is_control)
        || value.contains('\\')
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let url = Url::parse(value).map_err(|_| ProtocolError::InvalidFrame)?;
    if url.cannot_be_a_base()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let host = url.host().ok_or(ProtocolError::InvalidFrame)?;
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && match host {
            Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(address) => address.is_loopback(),
            Host::Ipv6(address) => address.is_loopback(),
        };
    if !secure && !loopback_http {
        return Err(ProtocolError::InvalidFrame);
    }
    Ok(())
}

/// Resolve the automatic provider timeout after validating the provider URL.
pub fn default_managed_provider_timeout_ms(value: &str) -> Result<u64, ProtocolError> {
    validate_managed_provider_base_url(value)?;
    let url = Url::parse(value).map_err(|_| ProtocolError::InvalidFrame)?;
    let loopback = match url.host().ok_or(ProtocolError::InvalidFrame)? {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    Ok(if loopback {
        LOOPBACK_PROVIDER_TIMEOUT_MS
    } else {
        REMOTE_PROVIDER_TIMEOUT_MS
    })
}

/// One sparse, secret-free override applied by the managed sidecar before canonical
/// runtime validation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFieldOverride {
    /// Stable dotted field identity using canonical camel-case configuration names.
    pub field_id: String,
    /// Structured replacement value for the selected field.
    pub value: Value,
}

/// OTLP transport selected by a Desktop-managed telemetry profile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedOtlpProtocol {
    /// OTLP over gRPC.
    #[default]
    Grpc,
    /// OTLP protobuf over HTTP.
    HttpProtobuf,
}

/// Durable journal detail released to live telemetry sinks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedJournalPayloadMode {
    /// Do not export journal records.
    #[default]
    Disabled,
    /// Export correlation and envelope metadata only.
    Metadata,
    /// Export complete plaintext durable payloads.
    Full,
}

/// One immutable, secret-free telemetry profile pinned by a Workspace.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTelemetryConfig {
    /// OpenTelemetry service name.
    pub name: String,
    /// Optional collector endpoint.
    pub endpoint: Option<String>,
    /// OTLP encoding.
    pub protocol: ManagedOtlpProtocol,
    /// Per-export timeout.
    pub timeout_ms: u64,
    /// Export traces over OTLP.
    pub traces_enabled: bool,
    /// Parent-based sampling ratio in millionths.
    pub trace_sample_ratio_millionths: u32,
    /// Export metrics over OTLP.
    pub metrics_enabled: bool,
    /// Periodic metric export interval.
    pub metric_export_interval_ms: u64,
    /// Export structured events over OTLP.
    pub logs_otlp: bool,
    /// Write structured events to stdout.
    pub logs_stdout_json: bool,
    /// Journal payload detail released to live sinks.
    pub journal_payloads: ManagedJournalPayloadMode,
    /// Explicit acknowledgement for full journal disclosure.
    pub acknowledge_sensitive_content: bool,
    /// Explicit acknowledgement for non-loopback plaintext OTLP.
    pub acknowledge_insecure_transport: bool,
    /// Process-wide, non-secret OpenTelemetry resource attributes.
    #[serde(default)]
    pub resource_attributes: BTreeMap<String, String>,
}

/// Private request for canonical repository-configuration inspection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationInspectionRequest {
    /// Exact private protocol version.
    pub protocol_version: u16,
    /// Bounded repository YAML read by the native Desktop host.
    pub yaml: String,
}

impl ConfigurationInspectionRequest {
    /// Validate request bounds before parsing repository content.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION
            || self.yaml.is_empty()
            || self.yaml.len() > MAX_CONFIGURATION_INSPECTION_BYTES
            || self.yaml.contains('\0')
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

/// Canonical, secret-free result returned by private sidecar inspection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationInspectionResponse {
    /// Exact private protocol version.
    pub protocol_version: u16,
    /// Canonical validated `RuntimeConfig`, or `None` when validation failed.
    pub canonical_config: Option<Value>,
    /// Stable dotted paths explicitly present in the repository document.
    pub explicit_field_ids: Vec<String>,
    /// Stable sanitized failure code.
    pub error_code: Option<String>,
}

impl ConfigurationInspectionResponse {
    /// Validate the mutually exclusive success and failure shape.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let success = self.canonical_config.is_some()
            && self.error_code.is_none()
            && !self.explicit_field_ids.is_empty();
        let failure = self.canonical_config.is_none()
            && self.explicit_field_ids.is_empty()
            && self.error_code.as_deref() == Some("invalid_configuration");
        if self.protocol_version != PROTOCOL_VERSION
            || (!success && !failure)
            || self.explicit_field_ids.len() > 4096
            || self.explicit_field_ids.iter().any(|field| {
                field.is_empty()
                    || field.len() > MAX_MANAGED_FIELD_ID_BYTES
                    || field.chars().any(char::is_control)
            })
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

impl ManagedTelemetryConfig {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.name.is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || !(100..=120_000).contains(&self.timeout_ms)
            || self.trace_sample_ratio_millionths > 1_000_000
            || !(1_000..=300_000).contains(&self.metric_export_interval_ms)
            || self.resource_attributes.len() > 32
            || self.resource_attributes.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 256
                    || value.len() > 256
                    || key.chars().any(char::is_control)
                    || value.chars().any(char::is_control)
            })
            || (self.journal_payloads == ManagedJournalPayloadMode::Full
                && !self.acknowledge_sensitive_content)
            || (self.acknowledge_sensitive_content
                && self.journal_payloads != ManagedJournalPayloadMode::Full)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let enabled = self.traces_enabled
            || self.metrics_enabled
            || self.logs_otlp
            || self.logs_stdout_json
            || self.journal_payloads != ManagedJournalPayloadMode::Disabled;
        if enabled
            && self.endpoint.is_none()
            && (self.traces_enabled || self.metrics_enabled || self.logs_otlp)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        if let Some(endpoint) = self.endpoint.as_deref() {
            let url = Url::parse(endpoint).map_err(|_| ProtocolError::InvalidFrame)?;
            if endpoint.len() > 2_048
                || url.cannot_be_a_base()
                || url.username() != ""
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(ProtocolError::InvalidFrame);
            }
            let host = url.host().ok_or(ProtocolError::InvalidFrame)?;
            let loopback = match host {
                Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
                Host::Ipv4(address) => address.is_loopback(),
                Host::Ipv6(address) => address.is_loopback(),
            };
            if url.scheme() != "https"
                && !(url.scheme() == "http" && (loopback || self.acknowledge_insecure_transport))
            {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        Ok(())
    }
}

impl ManagedFieldOverride {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.field_id.is_empty()
            || self.field_id.len() > MAX_MANAGED_FIELD_ID_BYTES
            || self.field_id.split('.').any(|segment| {
                segment.is_empty()
                    || !segment.as_bytes()[0].is_ascii_lowercase()
                    || !segment
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
            || serde_json::to_vec(&self.value).map_or(true, |encoded| {
                encoded.len() > MAX_MANAGED_FIELD_VALUE_BYTES
            })
            || json_value_depth(&self.value) > MAX_MANAGED_FIELD_VALUE_DEPTH
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

fn json_value_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => values
            .iter()
            .map(json_value_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        Value::Object(values) => values
            .values()
            .map(json_value_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
    }
}

/// Stable RuntimeConfig fields that managed Desktop revisions may override directly.
/// Complex catalogs and host-owned invariants are intentionally absent.
pub const MANAGED_EDITABLE_FIELD_IDS: &[&str] = &[
    "access.tools.include",
    "access.tools.exclude",
    "access.actions.allow",
    "access.actions.requireApproval",
    "access.actions.deny",
    "audit.exporter",
    "policy",
    "agent.maxTurns",
    "subagents.maxConcurrent",
    "context.autoCompaction",
    "context.compactAtPercent",
    "context.targetPercent",
    "context.preserveRecentMessages",
    "context.modelAssisted",
    "memory.indexEnabled",
    "memory.retrievalLimit",
    "memory.semantic",
    "research.maxSources",
    "research.maxWorkers",
    "skills.enabled",
    "skills.allowUserOverrides",
    "skills.bundled",
    "skills.repository",
    "skills.disabled",
    "workflows.repository",
    "sandbox.profile",
    "sandbox.allowBrokerFallback",
    "sandbox.helperPath",
    "sandbox.ociRuntime",
    "sandbox.ociImage",
    "sandbox.ociProxyImage",
    "sandbox.filesystem",
    "sandbox.executables",
    "sandbox.environment",
    "sandbox.networkDestinations",
    "sandbox.timeoutMs",
    "sandbox.maxOutputBytes",
    "sandbox.maxProcesses",
    "sandbox.maxMemoryBytes",
    "sandbox.maxConcurrency",
];

/// Compact secret-free runtime settings generated into canonical sidecar YAML.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRuntimeConfig {
    /// Access and policy preset.
    pub access_profile: ManagedAccessProfile,
    /// Host execution boundary, independent from access and approval policy.
    pub execution_boundary: ManagedExecutionBoundary,
    /// Bounded provider connection profiles.
    pub providers: Vec<ManagedProviderConfig>,
    /// Bounded explicit model profiles.
    pub models: Vec<ManagedModelConfig>,
    /// Logical roles mapped to model profiles.
    pub roles: BTreeMap<String, String>,
    /// Bounded credential-free SearXNG profiles.
    #[serde(default)]
    pub search_profiles: Vec<ManagedSearchConfig>,
    /// Exact `agent` and `research` routes mapped to search profiles.
    #[serde(default)]
    pub search_roles: BTreeMap<String, String>,
    /// Version-pinned MCP definitions compiled by Desktop.
    #[serde(default)]
    pub mcp_servers: Vec<ManagedMcpServerConfig>,
    /// Optional immutable telemetry profile selected by this Workspace.
    #[serde(default)]
    pub telemetry: Option<ManagedTelemetryConfig>,
    /// Sparse ordinary configuration fields compiled after typed catalogs and before
    /// canonical runtime validation. Desktop-owned invariants are rejected by the
    /// managed sidecar.
    #[serde(default)]
    pub field_overrides: Vec<ManagedFieldOverride>,
}

impl ManagedRuntimeConfig {
    /// Construct the deterministic credential-free managed runtime.
    pub fn echo(access_profile: ManagedAccessProfile) -> Self {
        Self {
            access_profile,
            execution_boundary: ManagedExecutionBoundary::default(),
            providers: vec![ManagedProviderConfig {
                profile: "echo".into(),
                kind: ManagedProviderKind::Echo,
                base_url: None,
                credential_id: None,
                timeout_ms: 120_000,
                chat_completions_output_token_parameter: None,
            }],
            models: vec![ManagedModelConfig {
                profile: "echo".into(),
                provider_profile: "echo".into(),
                model: "echo".into(),
                context_window_tokens: 32_768,
                max_output_tokens: 4_096,
                capabilities: ManagedModelCapabilities {
                    tool_calls: true,
                    streaming: true,
                    image_inputs: false,
                },
                reasoning_effort: None,
            }],
            roles: BTreeMap::from([("primary".into(), "echo".into())]),
            search_profiles: Vec::new(),
            search_roles: BTreeMap::new(),
            mcp_servers: Vec::new(),
            telemetry: None,
            field_overrides: Vec::new(),
        }
    }

    /// Select the host execution boundary for this managed runtime.
    #[must_use]
    pub const fn with_execution_boundary(
        mut self,
        execution_boundary: ManagedExecutionBoundary,
    ) -> Self {
        self.execution_boundary = execution_boundary;
        self
    }

    /// Return the selected host execution boundary.
    pub const fn execution_boundary(&self) -> ManagedExecutionBoundary {
        self.execution_boundary
    }

    /// Validate the compact configuration.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        const ROLES: [&str; 7] = [
            "primary",
            "risk_evaluator",
            "context_summarizer",
            "subagent_default",
            "research_planner",
            "research_worker",
            "research_synthesizer",
        ];
        if self.providers.is_empty()
            || self.providers.len() > MAX_MANAGED_PROVIDERS
            || self.models.is_empty()
            || self.models.len() > MAX_MANAGED_MODELS
            || !self.roles.contains_key("primary")
            || self
                .roles
                .keys()
                .any(|role| !ROLES.contains(&role.as_str()))
        {
            return Err(ProtocolError::InvalidFrame);
        }
        if self.search_profiles.len() > MAX_MANAGED_SEARCH_PROFILES
            || self.mcp_servers.len() > MAX_MANAGED_MCP_SERVERS
            || self.field_overrides.len() > MAX_MANAGED_FIELD_OVERRIDES
            || self
                .search_roles
                .keys()
                .any(|role| !matches!(role.as_str(), "agent" | "research"))
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut providers = BTreeSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if !providers.insert(provider.profile.as_str()) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        let mut models = BTreeSet::new();
        for model in &self.models {
            model.validate()?;
            if !models.insert(model.profile.as_str())
                || !providers.contains(model.provider_profile.as_str())
            {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        if self
            .roles
            .iter()
            .any(|(role, model)| !valid_token(role) || !models.contains(model.as_str()))
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut search_profiles = BTreeSet::new();
        for profile in &self.search_profiles {
            profile.validate()?;
            if !search_profiles.insert(profile.profile.as_str()) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        if self.search_roles.iter().any(|(role, profile)| {
            !valid_token(role) || !search_profiles.contains(profile.as_str())
        }) {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut mcp_servers = BTreeSet::new();
        for server in &self.mcp_servers {
            server.validate()?;
            if !mcp_servers.insert(server.name.as_str()) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        if let Some(telemetry) = &self.telemetry {
            telemetry.validate()?;
        }
        let mut field_ids = BTreeSet::new();
        for field in &self.field_overrides {
            field.validate()?;
            if !field_ids.insert(field.field_id.as_str()) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        Ok(())
    }
}

impl BootstrapGrant {
    /// Validate bounded, unique authority tokens.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !self
            .application_id
            .strip_prefix("app:")
            .is_some_and(valid_token)
            || !valid_unique_tokens(&self.scopes)
            || !valid_unique_tokens(&self.allowed_roles)
            || !valid_unique_tokens(&self.allowed_tools)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }

    fn validate_approval_broker(&self, primary: &Self) -> Result<(), ProtocolError> {
        self.validate()?;
        let primary_roles = primary.allowed_roles.iter().collect::<BTreeSet<_>>();
        if self.application_id != primary.application_id
            || self.scopes.as_slice() != [APPROVALS_RESPOND_SCOPE]
            || !self.allowed_tools.is_empty()
            || primary
                .scopes
                .iter()
                .any(|scope| scope == APPROVALS_RESPOND_SCOPE)
            || self
                .allowed_roles
                .iter()
                .any(|role| !primary_roles.contains(role))
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

/// Initial native-host request. It is accepted exactly once per child process.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRequest {
    /// Exact protocol version.
    pub protocol_version: u16,
    /// Fresh exchange identifier echoed by every subsequent frame.
    pub exchange_id: String,
    /// Expected runtime instance identifier.
    pub instance_id: String,
    /// Required public API major.
    pub api_major: u16,
    /// Canonical app-private runtime directory.
    pub instance_dir: String,
    /// Canonical selected workspace.
    pub workspace: String,
    /// Opaque identity of the exact workspace directory opened by the native parent.
    pub workspace_identity: WorkspaceIdentity,
    /// Optional owner-private Colossus home used only for bounded user instructions.
    ///
    /// This path travels only through inherited bootstrap IPC and is never emitted in
    /// generated runtime configuration or renderer DTOs.
    #[serde(default)]
    pub colossus_home: Option<String>,
    /// Suppress home/workspace AGENTS.md only for a trusted native diagnostic probe.
    ///
    /// Explicit probe instructions and immutable runtime-mode instructions remain active.
    /// This bootstrap-only bit is never accepted by public run APIs.
    #[serde(default)]
    pub suppress_automatic_agent_instructions: bool,
    /// Use a keyless plaintext journal for an explicitly isolated development runtime.
    ///
    /// This removes payload confidentiality, signed checkpoints, and the external
    /// rollback anchor. Native hosts must keep plaintext state separate from protected
    /// runtime state; the sidecar never migrates an existing journal in place.
    #[serde(default)]
    pub plaintext_journal_for_development: bool,
    /// Optional app-private PEM bundle copied and validated by the native host.
    ///
    /// This path travels only on the authenticated local bootstrap channel and is
    /// never part of the public API or renderer DTOs.
    #[serde(default)]
    pub ca_bundle_path: Option<String>,
    /// Optional native-selected official Codex credential file.
    ///
    /// This path travels only through inherited bootstrap IPC. It is required exactly
    /// when a managed provider uses `open_ai_codex` and never enters generated YAML.
    #[serde(default)]
    pub codex_auth_path: Option<String>,
    /// Compact secret-free app-managed runtime configuration.
    pub runtime: ManagedRuntimeConfig,
    /// Exact application authority.
    pub grant: BootstrapGrant,
    /// Optional native-only credential allowed to answer effect approvals and nothing else.
    pub approval_broker_grant: Option<BootstrapGrant>,
    /// Provider credentials referenced by opaque `host:` identifiers.
    pub host_credentials: Vec<HostCredential>,
    /// Optional worker IPC key supplied by a native host for its bundled TUI.
    ///
    /// The encoded key is accepted only through the inherited sidecar bootstrap
    /// channel and is never written into the generated managed configuration.
    pub worker_ipc_authentication: Option<SecretString>,
}

impl BootstrapRequest {
    /// Validate framing-independent bootstrap invariants before runtime construction.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION
            || !canonical_uuid(&self.exchange_id)
            || !canonical_uuid(&self.instance_id)
            || self.api_major != 1
            || !absolute_non_root(Path::new(&self.instance_dir))
            || !absolute_non_root(Path::new(&self.workspace))
            || self.colossus_home.as_deref().is_some_and(|path| {
                path.len() > MAX_PRIVATE_PATH_BYTES || !absolute_non_root(Path::new(path))
            })
            || self.ca_bundle_path.as_deref().is_some_and(|path| {
                path.len() > MAX_PRIVATE_PATH_BYTES || !absolute_non_root(Path::new(path))
            })
            || self.codex_auth_path.as_deref().is_some_and(|path| {
                path.len() > MAX_PRIVATE_PATH_BYTES || !absolute_non_root(Path::new(path))
            })
            || self.host_credentials.len() > MAX_HOST_CREDENTIALS
        {
            return Err(ProtocolError::InvalidFrame);
        }
        self.grant.validate()?;
        self.workspace_identity.validate()?;
        if let Some(grant) = &self.approval_broker_grant {
            grant.validate_approval_broker(&self.grant)?;
        }
        self.runtime.validate()?;
        let uses_codex = self
            .runtime
            .providers
            .iter()
            .any(|provider| provider.kind == ManagedProviderKind::OpenAiCodex);
        if uses_codex != self.codex_auth_path.is_some() {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut ids = BTreeSet::new();
        for credential in &self.host_credentials {
            credential.validate()?;
            if !ids.insert(credential.id.as_str()) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        for provider in &self.runtime.providers {
            if let Some(credential_id) = provider.credential_id.as_deref()
                && !ids.contains(credential_id)
            {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        for search in &self.runtime.search_profiles {
            if let Some(credential_id) = search.credential_id.as_deref()
                && !ids.contains(credential_id)
            {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        for server in &self.runtime.mcp_servers {
            let references = server
                .environment_credentials
                .values()
                .map(String::as_str)
                .chain(
                    server
                        .credential_headers
                        .values()
                        .map(|header| header.credential_id.as_str()),
                )
                .chain(
                    server
                        .oauth
                        .iter()
                        .filter_map(|oauth| oauth.client_secret_credential_id.as_deref()),
                );
            if references.into_iter().any(|id| !ids.contains(id)) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        if let Some(authentication) = &self.worker_ipc_authentication {
            decode_worker_authentication(authentication)?;
        }
        Ok(())
    }
}

impl fmt::Debug for BootstrapRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BootstrapRequest")
            .field("protocol_version", &self.protocol_version)
            .field("exchange_id", &self.exchange_id)
            .field("instance_id", &self.instance_id)
            .field("api_major", &self.api_major)
            .field("instance_dir", &"[PRIVATE PATH]")
            .field("workspace", &"[PRIVATE PATH]")
            .field("workspace_identity", &self.workspace_identity)
            .field("colossus_home_configured", &self.colossus_home.is_some())
            .field(
                "automatic_agent_instructions",
                &!self.suppress_automatic_agent_instructions,
            )
            .field(
                "plaintext_journal_for_development",
                &self.plaintext_journal_for_development,
            )
            .field("ca_bundle_configured", &self.ca_bundle_path.is_some())
            .field("codex_auth_configured", &self.codex_auth_path.is_some())
            .field("runtime", &self.runtime)
            .field("grant", &self.grant)
            .field("host_credentials", &"[REDACTED]")
            .field("worker_ipc_authentication", &"[REDACTED]")
            .finish()
    }
}

/// Encode one exact worker IPC authentication key for inherited-channel transfer.
pub fn encode_worker_authentication(
    authentication: &[u8; 32],
) -> Result<SecretString, ProtocolError> {
    let mut encoded = zeroize::Zeroizing::new(String::with_capacity(64));
    for byte in authentication {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").map_err(|_| ProtocolError::InvalidFrame)?;
    }
    SecretString::new(encoded.as_str())
}

/// Decode one inherited worker IPC authentication key into zeroizing memory.
pub fn decode_worker_authentication(
    authentication: &SecretString,
) -> Result<Zeroizing<[u8; 32]>, ProtocolError> {
    let encoded = authentication.expose().as_bytes();
    if encoded.len() != 64 {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut decoded = Zeroizing::new([0_u8; 32]);
    for (index, output) in decoded.iter_mut().enumerate() {
        let offset = index * 2;
        *output =
            (decode_lower_hex(encoded[offset])? << 4) | decode_lower_hex(encoded[offset + 1])?;
    }
    Ok(decoded)
}

/// Native desktop request delivered exactly once through a private inherited channel.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopTuiParentFrame {
    /// Bind the signed CLI to the already-running managed worker.
    Authenticate(DesktopTuiAuthenticationRequest),
}

/// Signed TUI responses delivered before the renderer receives the PTY session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopTuiChildFrame {
    /// The CLI owns the fixed inherited descriptors and is ready for one secret frame.
    Ready(DesktopTuiReady),
    /// The CLI consumed and validated the secret frame into zeroizing memory.
    Authenticated(DesktopTuiAuthenticated),
}

/// One-use TUI authentication request.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopTuiAuthenticationRequest {
    /// Exact private protocol version.
    pub protocol_version: u16,
    /// Fresh CLI-generated exchange identifier.
    pub exchange_id: String,
    /// Existing managed worker IPC key.
    pub worker_ipc_authentication: SecretString,
}

impl DesktopTuiAuthenticationRequest {
    /// Validate exact framing identity and worker key shape.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != DESKTOP_TUI_PROTOCOL_VERSION
            || !canonical_uuid(&self.exchange_id)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        decode_worker_authentication(&self.worker_ipc_authentication)?;
        Ok(())
    }
}

impl fmt::Debug for DesktopTuiAuthenticationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopTuiAuthenticationRequest")
            .field("protocol_version", &self.protocol_version)
            .field("exchange_id", &self.exchange_id)
            .field("worker_ipc_authentication", &"[REDACTED]")
            .finish()
    }
}

/// CLI readiness marker emitted over the fixed inherited response descriptor.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopTuiReady {
    /// Exact private protocol version.
    pub protocol_version: u16,
    /// Fresh CLI-generated exchange identifier.
    pub exchange_id: String,
    /// Opaque identity of the exact workspace directory securely opened by the CLI.
    pub workspace_identity: WorkspaceIdentity,
}

impl DesktopTuiReady {
    /// Validate a readiness marker before releasing authentication material.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != DESKTOP_TUI_PROTOCOL_VERSION
            || !canonical_uuid(&self.exchange_id)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        self.workspace_identity.validate_current()
    }
}

impl fmt::Debug for DesktopTuiReady {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopTuiReady")
            .field("protocol_version", &self.protocol_version)
            .field("exchange_id", &self.exchange_id)
            .field("workspace_identity", &"[OPAQUE IDENTITY]")
            .finish()
    }
}

/// Confirmation that the CLI consumed the one-use authentication frame.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopTuiAuthenticated {
    /// Exact private protocol version.
    pub protocol_version: u16,
    /// Exchange identifier from the readiness and authentication frames.
    pub exchange_id: String,
}

impl DesktopTuiAuthenticated {
    /// Validate an acknowledgement against the active one-use exchange.
    pub fn validate(&self, exchange_id: &str) -> Result<(), ProtocolError> {
        if self.protocol_version == DESKTOP_TUI_PROTOCOL_VERSION
            && canonical_uuid(&self.exchange_id)
            && self.exchange_id == exchange_id
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidFrame)
        }
    }
}

/// Native-host frames sent to the child.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParentFrame {
    /// One-use bootstrap request.
    Bootstrap(Box<BootstrapRequest>),
    /// Acknowledgement that the bearer was delivered into native memory.
    Ack(AckRequest),
}

/// Acknowledgement required before the pending public credential is activated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckRequest {
    /// Exact protocol version.
    pub protocol_version: u16,
    /// Exchange identifier from the ready response.
    pub exchange_id: String,
    /// Non-secret pending credential identifier.
    pub credential_id: String,
    /// Pending approval-broker credential identifier, when one was requested.
    pub approval_broker_credential_id: Option<String>,
}

/// Child frames sent to the native host.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChildFrame {
    /// Endpoint identity plus one-use bearer delivery.
    Ready(ReadyResponse),
    /// Durable credential activation completed.
    Activated(ActivatedResponse),
    /// Sanitized fail-closed startup outcome.
    Failed(FailureResponse),
}

/// Public endpoint metadata and memory-only bearer delivery.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyResponse {
    /// Exact protocol version.
    pub protocol_version: u16,
    /// Fresh exchange identifier.
    pub exchange_id: String,
    /// Runtime instance identity.
    pub instance_id: String,
    /// Public API major.
    pub api_major: u16,
    /// Must be `sidecar`.
    pub deployment_mode: String,
    /// Exact bound loopback HTTPS endpoint.
    pub endpoint: String,
    /// Single public leaf certificate PEM.
    pub certificate_pem: String,
    /// Canonical lowercase SHA-256 certificate fingerprint.
    pub certificate_sha256: String,
    /// Non-secret pending credential identifier.
    pub credential_id: String,
    /// Bearer delivered once through this inherited channel.
    pub bearer: SecretString,
    /// Non-secret pending approval-broker credential identifier.
    pub approval_broker_credential_id: Option<String>,
    /// Approval-broker bearer delivered once through this inherited channel.
    pub approval_broker_bearer: Option<SecretString>,
}

impl ReadyResponse {
    /// Validate bounded structural metadata before native TLS validation.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION
            || !canonical_uuid(&self.exchange_id)
            || !canonical_uuid(&self.instance_id)
            || self.api_major != 1
            || self.deployment_mode != "sidecar"
            || self.endpoint.is_empty()
            || self.endpoint.len() > 2_048
            || self.certificate_pem.is_empty()
            || self.certificate_pem.len() > MAX_CERTIFICATE_PEM_BYTES
            || !lowercase_sha256(&self.certificate_sha256)
            || !canonical_uuid(&self.credential_id)
            || !matching_optional_credential(
                &self.approval_broker_credential_id,
                &self.approval_broker_bearer,
            )
            || self.approval_broker_credential_id.as_deref() == Some(self.credential_id.as_str())
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

impl fmt::Debug for ReadyResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadyResponse")
            .field("protocol_version", &self.protocol_version)
            .field("exchange_id", &self.exchange_id)
            .field("instance_id", &self.instance_id)
            .field("api_major", &self.api_major)
            .field("deployment_mode", &self.deployment_mode)
            .field("endpoint", &self.endpoint)
            .field("certificate_pem_bytes", &self.certificate_pem.len())
            .field("certificate_sha256", &self.certificate_sha256)
            .field("credential_id", &self.credential_id)
            .field("bearer", &"[REDACTED]")
            .field(
                "approval_broker_credential_id",
                &self.approval_broker_credential_id,
            )
            .field("approval_broker_bearer", &"[REDACTED]")
            .finish()
    }
}

/// Confirmation that the pending credential is durably active.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivatedResponse {
    /// Exact protocol version.
    pub protocol_version: u16,
    /// Exchange identifier from the bootstrap request.
    pub exchange_id: String,
    /// Activated non-secret credential identifier.
    pub credential_id: String,
    /// Activated approval-broker credential identifier, when one was requested.
    pub approval_broker_credential_id: Option<String>,
}

/// Sanitized startup failure codes suitable for a native status surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// Bootstrap framing or identity was invalid.
    InvalidBootstrap,
    /// App-private storage validation failed.
    InvalidInstanceDirectory,
    /// Workspace identity was invalid or unavailable.
    InvalidWorkspace,
    /// Runtime configuration could not be validated.
    InvalidConfiguration,
    /// The runtime writer lease is already owned.
    WorkspaceBusy,
    /// Independent API identity or credential setup failed.
    PublicApiSetup,
    /// Credential delivery was not acknowledged exactly.
    CredentialActivation,
    /// Runtime service failed after activation.
    RuntimeFailed,
}

/// Fail-closed child startup result with no raw internal diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureResponse {
    /// Exact protocol version.
    pub protocol_version: u16,
    /// Exchange identifier when a valid request supplied one.
    pub exchange_id: Option<String>,
    /// Stable sanitized class.
    pub code: FailureCode,
}

/// Bootstrap framing and validation failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ProtocolError {
    /// A frame was absent, truncated, oversized, or could not be decoded.
    #[error("sidecar bootstrap frame is invalid")]
    InvalidFrame,
    /// Reading or writing the inherited channel failed.
    #[error("sidecar bootstrap channel failed")]
    Channel,
}

/// Encode one complete length-prefixed frame into zeroizing memory.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>, ProtocolError> {
    let payload =
        Zeroizing::new(serde_json::to_vec(value).map_err(|_| ProtocolError::InvalidFrame)?);
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::InvalidFrame)?;
    let mut frame = Zeroizing::new(Vec::with_capacity(4 + payload.len()));
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload.as_slice());
    Ok(frame)
}

/// Decode one already bounded JSON payload.
pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ProtocolError> {
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    serde_json::from_slice(payload).map_err(|_| ProtocolError::InvalidFrame)
}

/// Write and flush one complete frame to a synchronous inherited channel.
pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), ProtocolError> {
    let frame = encode_frame(value)?;
    writer
        .write_all(frame.as_slice())
        .and_then(|()| writer.flush())
        .map_err(|_| ProtocolError::Channel)
}

/// Read one complete bounded frame from a synchronous inherited channel.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, ProtocolError> {
    let mut length = [0_u8; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| ProtocolError::Channel)?;
    let length =
        usize::try_from(u32::from_be_bytes(length)).map_err(|_| ProtocolError::InvalidFrame)?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut payload = Zeroizing::new(vec![0_u8; length]);
    reader
        .read_exact(payload.as_mut_slice())
        .map_err(|_| ProtocolError::Channel)?;
    decode_payload(payload.as_slice())
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn valid_host_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value.len() <= MAX_IDENTIFIER_BYTES
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_unique_tokens(values: &[String]) -> bool {
    values.len() <= MAX_AUTHORIZATION_ITEMS
        && values.iter().all(|value| valid_token(value))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_lower_hex(byte: u8) -> Result<u8, ProtocolError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ProtocolError::InvalidFrame),
    }
}

fn matching_optional_credential(
    credential_id: &Option<String>,
    bearer: &Option<SecretString>,
) -> bool {
    match (credential_id, bearer) {
        (None, None) => true,
        (Some(credential_id), Some(_)) => canonical_uuid(credential_id),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn absolute_non_root(path: &Path) -> bool {
    path.is_absolute()
        && path.parent().is_some()
        && path.file_name().is_some()
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn absolute_test_path(name: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\colossus-test\{name}")
        } else {
            format!("/tmp/colossus-test/{name}")
        }
    }

    fn request() -> BootstrapRequest {
        BootstrapRequest {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: Uuid::now_v7().to_string(),
            instance_id: Uuid::now_v7().to_string(),
            api_major: 1,
            instance_dir: absolute_test_path("sidecar-instance"),
            workspace: absolute_test_path("sidecar-workspace"),
            workspace_identity: WorkspaceIdentity::from_unix_parts(42, 84),
            colossus_home: None,
            suppress_automatic_agent_instructions: false,
            plaintext_journal_for_development: false,
            ca_bundle_path: None,
            codex_auth_path: None,
            runtime: ManagedRuntimeConfig {
                access_profile: ManagedAccessProfile::Development,
                execution_boundary: ManagedExecutionBoundary::WorkspaceIsolated,
                providers: vec![ManagedProviderConfig {
                    profile: "provider-main".into(),
                    kind: ManagedProviderKind::OpenAiCompatible,
                    base_url: Some("https://provider.example/v1".into()),
                    credential_id: Some("provider-main".into()),
                    timeout_ms: 120_000,
                    chat_completions_output_token_parameter: None,
                }],
                models: vec![ManagedModelConfig {
                    profile: "main".into(),
                    provider_profile: "provider-main".into(),
                    model: "model-v1".into(),
                    context_window_tokens: 32_768,
                    max_output_tokens: 4_096,
                    capabilities: ManagedModelCapabilities {
                        tool_calls: true,
                        streaming: true,
                        image_inputs: false,
                    },
                    reasoning_effort: None,
                }],
                roles: BTreeMap::from([("primary".into(), "main".into())]),
                search_profiles: Vec::new(),
                search_roles: BTreeMap::new(),
                mcp_servers: Vec::new(),
                telemetry: None,
                field_overrides: Vec::new(),
            },
            grant: BootstrapGrant {
                application_id: "app:desktop".into(),
                scopes: vec!["runs:execute".into()],
                allowed_roles: vec!["primary".into()],
                allowed_tools: vec!["session.list".into()],
            },
            approval_broker_grant: Some(BootstrapGrant {
                application_id: "app:desktop".into(),
                scopes: vec![APPROVALS_RESPOND_SCOPE.into()],
                allowed_roles: vec!["primary".into()],
                allowed_tools: Vec::new(),
            }),
            host_credentials: vec![
                HostCredential::new(
                    "provider-main",
                    SecretString::new("secret-value").expect("secret"),
                )
                .expect("credential"),
            ],
            worker_ipc_authentication: Some(
                encode_worker_authentication(&[0x5a; 32]).expect("worker authentication"),
            ),
        }
    }

    #[test]
    fn managed_execution_boundary_defaults_to_full_access_and_has_stable_wire_values() {
        let runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal);
        assert_eq!(
            runtime.execution_boundary(),
            ManagedExecutionBoundary::FullAccess
        );
        let mut value = serde_json::to_value(&runtime).expect("serialize managed runtime");
        assert_eq!(value["execution_boundary"], "full_access");

        value
            .as_object_mut()
            .expect("runtime object")
            .remove("execution_boundary");
        assert!(serde_json::from_value::<ManagedRuntimeConfig>(value).is_err());

        for (boundary, wire) in [
            (ManagedExecutionBoundary::FullAccess, "full_access"),
            (
                ManagedExecutionBoundary::WorkspaceIsolated,
                "workspace_isolated",
            ),
            (
                ManagedExecutionBoundary::OfflineIsolated,
                "offline_isolated",
            ),
        ] {
            assert_eq!(serde_json::to_value(boundary).expect("wire value"), wire);
        }
    }

    #[test]
    fn managed_search_accepts_only_explicit_bounded_searxng_routes() {
        let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal);
        runtime.search_profiles = vec![ManagedSearchConfig {
            profile: "local-search".into(),
            kind: ManagedSearchKind::Searxng,
            endpoint: "http://127.0.0.1:8888/search".into(),
            credential_id: None,
            auth_header: None,
            timeout_ms: 30_000,
        }];
        runtime.search_roles = BTreeMap::from([("research".into(), "local-search".into())]);
        runtime.validate().expect("valid local search");

        runtime.search_profiles[0].endpoint = "http://example.com/search".into();
        assert!(runtime.validate().is_err());
        runtime.search_profiles[0].endpoint = "http://127.0.0.1:8888/search".into();
        runtime.search_roles = BTreeMap::from([("research".into(), "missing".into())]);
        assert!(runtime.validate().is_err());

        runtime.search_profiles[0] = ManagedSearchConfig {
            profile: "serp".into(),
            kind: ManagedSearchKind::SerpApi,
            endpoint: "https://serpapi.com/search.json".into(),
            credential_id: None,
            auth_header: None,
            timeout_ms: 30_000,
        };
        runtime.search_roles = BTreeMap::from([("research".into(), "serp".into())]);
        assert!(runtime.validate().is_err());
        runtime.search_profiles[0].credential_id = Some("serp-key".into());
        runtime.validate().expect("credential-bound SerpAPI");
    }

    #[test]
    fn managed_telemetry_requires_bounded_explicit_disclosure() {
        let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal);
        runtime.telemetry = Some(ManagedTelemetryConfig {
            name: "colossus-desktop".into(),
            endpoint: Some("http://127.0.0.1:4317".into()),
            protocol: ManagedOtlpProtocol::Grpc,
            timeout_ms: 10_000,
            traces_enabled: true,
            trace_sample_ratio_millionths: 100_000,
            metrics_enabled: true,
            metric_export_interval_ms: 60_000,
            logs_otlp: true,
            logs_stdout_json: false,
            journal_payloads: ManagedJournalPayloadMode::Metadata,
            acknowledge_sensitive_content: false,
            acknowledge_insecure_transport: false,
            resource_attributes: BTreeMap::from([("service.namespace".into(), "colossus".into())]),
        });
        runtime.validate().expect("valid loopback telemetry");

        let telemetry = runtime.telemetry.as_mut().expect("telemetry");
        telemetry.journal_payloads = ManagedJournalPayloadMode::Full;
        assert!(runtime.validate().is_err());
        runtime
            .telemetry
            .as_mut()
            .expect("telemetry")
            .acknowledge_sensitive_content = true;
        runtime.validate().expect("acknowledged disclosure");

        let telemetry = runtime.telemetry.as_mut().expect("telemetry");
        telemetry.endpoint = Some("http://collector.example.test:4317".into());
        assert!(runtime.validate().is_err());
        runtime
            .telemetry
            .as_mut()
            .expect("telemetry")
            .acknowledge_insecure_transport = true;
        runtime
            .validate()
            .expect("acknowledged plaintext transport");
    }

    #[test]
    fn configuration_inspection_frames_are_bounded_and_mutually_exclusive() {
        let request = ConfigurationInspectionRequest {
            protocol_version: PROTOCOL_VERSION,
            yaml: "schemaVersion: 2\nstorage:\n  path: state.redb\n".into(),
        };
        request.validate().expect("inspection request");

        let success = ConfigurationInspectionResponse {
            protocol_version: PROTOCOL_VERSION,
            canonical_config: Some(serde_json::json!({ "schemaVersion": 2 })),
            explicit_field_ids: vec!["schemaVersion".into(), "storage.path".into()],
            error_code: None,
        };
        success.validate().expect("inspection response");

        let mut invalid = success;
        invalid.error_code = Some("invalid_configuration".into());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn bootstrap_binds_every_search_and_mcp_credential_reference() {
        let mut bootstrap = request();
        bootstrap.runtime.search_profiles = vec![ManagedSearchConfig {
            profile: "serp".into(),
            kind: ManagedSearchKind::SerpApi,
            endpoint: "https://serpapi.com/search.json".into(),
            credential_id: Some("serp-key".into()),
            auth_header: None,
            timeout_ms: 30_000,
        }];
        bootstrap.runtime.search_roles = BTreeMap::from([("research".into(), "serp".into())]);
        assert_eq!(bootstrap.validate(), Err(ProtocolError::InvalidFrame));

        bootstrap.host_credentials.push(
            HostCredential::new(
                "serp-key",
                SecretString::new("search-secret").expect("secret"),
            )
            .expect("search credential"),
        );
        bootstrap.validate().expect("bound search credential");

        bootstrap.runtime.mcp_servers = vec![ManagedMcpServerConfig {
            name: "docs".into(),
            transport: ManagedMcpTransport::Stdio,
            command: Some("docs-mcp".into()),
            args: Vec::new(),
            working_directory: None,
            environment_credentials: BTreeMap::from([("MCP_TOKEN".into(), "mcp-key".into())]),
            url: None,
            headers: BTreeMap::new(),
            credential_headers: BTreeMap::new(),
            allow_stateless: false,
            oauth: None,
            allowed_tools: vec!["search".into()],
            research_tools: Vec::new(),
            timeout_ms: None,
            max_output_bytes: None,
        }];
        assert_eq!(bootstrap.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn managed_field_overrides_are_sparse_bounded_and_unique() {
        let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal);
        runtime.field_overrides = vec![ManagedFieldOverride {
            field_id: "research.maxSources".into(),
            value: Value::from(12),
        }];
        runtime.validate().expect("valid sparse override");

        runtime
            .field_overrides
            .push(runtime.field_overrides[0].clone());
        assert!(runtime.validate().is_err());
        runtime.field_overrides.truncate(1);
        runtime.field_overrides[0].field_id = "Research.maxSources".into();
        assert!(runtime.validate().is_err());
        runtime.field_overrides[0].field_id = "research..maxSources".into();
        assert!(runtime.validate().is_err());
        runtime.field_overrides[0].field_id = "research.maxSources".into();
        runtime.field_overrides[0].value = Value::String("x".repeat(65 * 1024));
        assert!(runtime.validate().is_err());
    }

    #[test]
    fn managed_mcp_servers_accept_only_bounded_opaque_credential_references() {
        let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Development);
        runtime.mcp_servers = vec![ManagedMcpServerConfig {
            name: "research".into(),
            transport: ManagedMcpTransport::StreamableHttp,
            command: None,
            args: Vec::new(),
            working_directory: None,
            environment_credentials: BTreeMap::new(),
            url: Some("https://mcp.example.test/service".into()),
            headers: BTreeMap::new(),
            credential_headers: BTreeMap::from([(
                "Authorization".into(),
                ManagedMcpCredentialHeader {
                    scheme: Some("Bearer".into()),
                    credential_id: "mcp-token".into(),
                },
            )]),
            allow_stateless: false,
            oauth: None,
            allowed_tools: vec!["search".into()],
            research_tools: vec![ManagedMcpResearchTool {
                tool: "search".into(),
                title: Some("MCP search".into()),
                arguments: serde_json::json!({"query": "{query}"}),
            }],
            timeout_ms: Some(30_000),
            max_output_bytes: Some(1024 * 1024),
        }];
        runtime.validate().expect("managed MCP server");

        runtime.mcp_servers[0]
            .credential_headers
            .get_mut("Authorization")
            .expect("credential header")
            .credential_id = "host:already-prefixed".into();
        assert!(runtime.validate().is_err());
        runtime.mcp_servers[0]
            .credential_headers
            .get_mut("Authorization")
            .expect("credential header")
            .credential_id = "mcp-token".into();
        runtime.mcp_servers[0].allowed_tools = vec!["*".into(), "search".into()];
        assert!(runtime.validate().is_err());
    }

    #[test]
    fn secret_frames_round_trip_without_debug_disclosure() {
        let mut request = request();
        let ca_bundle_path = absolute_test_path("company-ca.pem");
        request.ca_bundle_path = Some(ca_bundle_path.clone());
        request.validate().expect("request");
        let frame = ParentFrame::Bootstrap(Box::new(request));
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).expect("write");
        let decoded: ParentFrame = read_frame(&mut Cursor::new(bytes)).expect("read");
        let ParentFrame::Bootstrap(mut decoded) = decoded else {
            panic!("wrong frame");
        };
        assert_eq!(decoded.host_credentials[0].secret.expose(), "secret-value");
        assert_eq!(
            decode_worker_authentication(
                decoded
                    .worker_ipc_authentication
                    .as_ref()
                    .expect("worker authentication")
            )
            .expect("decode")
            .as_ref(),
            &[0x5a; 32]
        );
        assert!(!format!("{decoded:?}").contains("secret-value"));
        assert!(!format!("{decoded:?}").contains(&"5a".repeat(32)));
        assert!(!format!("{decoded:?}").contains(&decoded.workspace_identity.sha256));
        assert!(!format!("{decoded:?}").contains("company-ca.pem"));
        assert_eq!(
            decoded.ca_bundle_path.as_deref(),
            Some(ca_bundle_path.as_str())
        );

        decoded.ca_bundle_path = Some("../company-ca.pem".into());
        assert_eq!(decoded.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn codex_bootstrap_requires_only_a_private_native_auth_path() {
        let mut request = request();
        request.runtime.providers[0].kind = ManagedProviderKind::OpenAiCodex;
        request.runtime.providers[0].base_url = None;
        request.runtime.providers[0].credential_id = None;
        request.runtime.models[0].reasoning_effort = Some(ManagedReasoningEffort::XHigh);
        request.host_credentials.clear();

        assert_eq!(request.validate(), Err(ProtocolError::InvalidFrame));

        let auth_path = absolute_test_path("codex-auth.json");
        request.codex_auth_path = Some(auth_path.clone());
        request.validate().expect("Codex bootstrap");
        let debug = format!("{request:?}");
        assert!(debug.contains("codex_auth_configured: true"));
        assert!(!debug.contains(&auth_path));

        request.runtime.providers[0].base_url = Some("https://chatgpt.com".into());
        assert_eq!(request.validate(), Err(ProtocolError::InvalidFrame));
        request.runtime.providers[0].base_url = None;
        request.runtime.providers[0].credential_id = Some("host-key".into());
        assert_eq!(request.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn colossus_home_is_optional_absolute_and_redacted() {
        let mut request = request();
        let home = absolute_test_path("home");
        request.colossus_home = Some(home.clone());
        request.validate().expect("Colossus home bootstrap");
        let debug = format!("{request:?}");
        assert!(debug.contains("colossus_home_configured: true"));
        assert!(!debug.contains(&home));

        request.colossus_home = Some("../.colossus".into());
        assert_eq!(request.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn diagnostic_instruction_suppression_is_private_and_defaults_to_loading() {
        let mut request = request();
        assert!(!request.suppress_automatic_agent_instructions);
        request.suppress_automatic_agent_instructions = true;
        request.validate().expect("diagnostic bootstrap");
        assert!(
            format!("{request:?}").contains("automatic_agent_instructions: false"),
            "safe debug output may expose only the non-secret mode"
        );

        let mut legacy = serde_json::to_value(&request).expect("request JSON");
        legacy
            .as_object_mut()
            .expect("request object")
            .remove("suppress_automatic_agent_instructions");
        let decoded: BootstrapRequest = serde_json::from_value(legacy).expect("defaulted request");
        assert!(
            !decoded.suppress_automatic_agent_instructions,
            "an omitted private flag must preserve normal AGENTS.md loading"
        );
    }

    #[test]
    fn plaintext_development_journal_is_explicit_and_defaults_to_protected() {
        let mut request = request();
        assert!(!request.plaintext_journal_for_development);
        request.plaintext_journal_for_development = true;
        request.validate().expect("plaintext development bootstrap");

        let mut legacy = serde_json::to_value(&request).expect("request JSON");
        legacy
            .as_object_mut()
            .expect("request object")
            .remove("plaintext_journal_for_development");
        let decoded: BootstrapRequest = serde_json::from_value(legacy).expect("defaulted request");
        assert!(
            !decoded.plaintext_journal_for_development,
            "an omitted journal mode must preserve platform protection"
        );
    }

    #[test]
    fn workspace_identity_is_versioned_bounded_and_object_specific() {
        let identity = WorkspaceIdentity::from_macos_parts(42, 84, 1_700_000_000, 123_456_789)
            .expect("current identity");
        identity.validate().expect("identity");
        identity.validate_current().expect("current identity");
        assert_eq!(identity.sha256.len(), 64);
        assert_ne!(
            identity,
            WorkspaceIdentity::from_macos_parts(42, 84, 1_700_000_000, 123_456_790)
                .expect("different birthtime")
        );
        assert_ne!(
            identity,
            WorkspaceIdentity::from_macos_parts(42, 85, 1_700_000_000, 123_456_789)
                .expect("different inode")
        );

        let legacy = WorkspaceIdentity::from_unix_parts(42, 84);
        legacy.validate().expect("legacy wire identity");
        assert!(legacy.is_legacy_v1());
        assert_eq!(legacy.validate_current(), Err(ProtocolError::InvalidFrame));
        assert_eq!(
            WorkspaceIdentity::from_macos_parts(42, 84, 0, 0),
            Err(ProtocolError::InvalidFrame)
        );
        let windows = WorkspaceIdentity::from_windows_parts(42, [7; 16]).expect("Windows identity");
        windows.validate().expect("Windows wire identity");
        windows
            .validate_current()
            .expect("current Windows identity");
        assert_ne!(
            windows,
            WorkspaceIdentity::from_windows_parts(42, [8; 16]).expect("different file ID")
        );
        assert_eq!(
            WorkspaceIdentity::from_windows_parts(0, [7; 16]),
            Err(ProtocolError::InvalidFrame)
        );
        assert_eq!(
            WorkspaceIdentity::from_windows_parts(42, [0; 16]),
            Err(ProtocolError::InvalidFrame)
        );

        let mut wrong_version = identity.clone();
        wrong_version.version = u16::MAX;
        assert_eq!(wrong_version.validate(), Err(ProtocolError::InvalidFrame));

        let mut malformed = identity;
        malformed.sha256 = "A".repeat(64);
        assert_eq!(malformed.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn missing_workspace_identity_version_is_migratable_but_not_wire_valid() {
        let legacy = WorkspaceIdentity::from_unix_parts(42, 84);
        let decoded: WorkspaceIdentity = serde_json::from_value(serde_json::json!({
            "sha256": legacy.sha256,
        }))
        .expect("preview identity");

        assert!(decoded.is_legacy_v1());
        assert_eq!(decoded.validate(), Err(ProtocolError::InvalidFrame));
        assert_eq!(decoded.validate_current(), Err(ProtocolError::InvalidFrame));
        let encoded = serde_json::to_value(&decoded).expect("serialized identity");
        assert_eq!(encoded["version"], LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION);

        let mut bootstrap = serde_json::to_value(request()).expect("bootstrap JSON");
        bootstrap["workspace_identity"]
            .as_object_mut()
            .expect("workspace identity")
            .remove("version");
        let bootstrap: BootstrapRequest =
            serde_json::from_value(bootstrap).expect("preview bootstrap shape");
        assert_eq!(bootstrap.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn desktop_tui_exchange_is_exact_bounded_and_redacted() {
        let exchange_id = Uuid::now_v7().to_string();
        let workspace_identity =
            WorkspaceIdentity::from_macos_parts(42, 84, 1_700_000_000, 123_456_789)
                .expect("workspace identity");
        let ready = DesktopTuiChildFrame::Ready(DesktopTuiReady {
            protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
            exchange_id: exchange_id.clone(),
            workspace_identity: workspace_identity.clone(),
        });
        let mut channel = Vec::new();
        write_frame(&mut channel, &ready).expect("ready frame");
        let decoded: DesktopTuiChildFrame =
            read_frame(&mut Cursor::new(channel)).expect("read ready");
        let DesktopTuiChildFrame::Ready(decoded) = decoded else {
            panic!("wrong frame");
        };
        decoded.validate().expect("valid ready");
        assert_eq!(decoded.workspace_identity, workspace_identity);
        assert!(!format!("{decoded:?}").contains(&decoded.workspace_identity.sha256));

        let request = DesktopTuiAuthenticationRequest {
            protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
            exchange_id: exchange_id.clone(),
            worker_ipc_authentication: encode_worker_authentication(&[0xa5; 32])
                .expect("authentication"),
        };
        request.validate().expect("valid request");
        assert!(!format!("{request:?}").contains(&"a5".repeat(32)));

        DesktopTuiAuthenticated {
            protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
            exchange_id: exchange_id.clone(),
        }
        .validate(&exchange_id)
        .expect("matching acknowledgement");
        assert_eq!(
            DesktopTuiAuthenticated {
                protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
                exchange_id,
            }
            .validate(&Uuid::now_v7().to_string()),
            Err(ProtocolError::InvalidFrame)
        );
    }

    #[test]
    fn delegation_authority_is_accepted_as_a_bounded_tool() {
        let mut request = request();
        request.grant.allowed_tools.push("agent.delegate".into());
        request.validate().expect("bounded delegation grant");
    }

    #[test]
    fn frames_fail_closed_on_oversize() {
        let mut input = Cursor::new((u32::try_from(MAX_FRAME_BYTES).unwrap() + 1).to_be_bytes());
        assert_eq!(
            read_frame::<_, ParentFrame>(&mut input).err(),
            Some(ProtocolError::InvalidFrame)
        );
    }

    #[test]
    fn duplicate_host_credential_ids_are_rejected() {
        let mut request = request();
        request.host_credentials.push(
            HostCredential::new(
                "provider-main",
                SecretString::new("another-secret").expect("secret"),
            )
            .expect("credential"),
        );
        assert_eq!(request.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn provider_urls_reject_credentials_and_non_loopback_cleartext() {
        for invalid in [
            "https://token@provider.example/v1",
            "https://provider.example/v1?api_key=secret",
            "https://provider.example/v1#secret",
            "http://provider.example/v1",
            "http://localhost.example/v1",
            "https://provider.example\\@attacker.invalid/v1",
        ] {
            assert_eq!(
                validate_managed_provider_base_url(invalid),
                Err(ProtocolError::InvalidFrame),
                "accepted unsafe URL: {invalid}"
            );
        }

        for valid in [
            "https://provider.example/v1",
            "http://localhost:8080/v1",
            "http://127.1:8080/v1",
            "http://[::1]:8080/v1",
        ] {
            validate_managed_provider_base_url(valid)
                .unwrap_or_else(|error| panic!("rejected {valid}: {error}"));
        }
    }

    #[test]
    fn automatic_provider_timeouts_distinguish_loopback_from_remote_hosts() {
        for loopback in [
            "http://localhost:8080/v1",
            "http://127.1:8080/v1",
            "http://[::1]:8080/v1",
            "https://127.0.0.1/v1",
        ] {
            assert_eq!(
                default_managed_provider_timeout_ms(loopback),
                Ok(LOOPBACK_PROVIDER_TIMEOUT_MS),
                "wrong loopback default for {loopback}"
            );
        }
        assert_eq!(
            default_managed_provider_timeout_ms("https://provider.example/v1"),
            Ok(REMOTE_PROVIDER_TIMEOUT_MS)
        );
        assert_eq!(
            default_managed_provider_timeout_ms("https://192.168.1.10/v1"),
            Ok(REMOTE_PROVIDER_TIMEOUT_MS),
            "private network hosts must retain the remote default"
        );
    }

    #[test]
    fn chat_completions_output_token_parameter_is_optional_and_chat_scoped() {
        let mut request = request();
        let encoded = serde_json::to_string(&request.runtime.providers[0]).expect("encode");
        assert!(
            !encoded.contains("chat_completions_output_token_parameter"),
            "omitted parameter must stay off the wire: {encoded}"
        );
        assert_eq!(
            serde_json::from_str::<ManagedProviderConfig>(&encoded).expect("decode"),
            request.runtime.providers[0],
            "hosts that predate the field must round-trip unchanged"
        );

        for parameter in [
            ManagedChatCompletionsOutputTokenParameter::MaxTokens,
            ManagedChatCompletionsOutputTokenParameter::MaxCompletionTokens,
            ManagedChatCompletionsOutputTokenParameter::Omit,
        ] {
            request.runtime.providers[0].chat_completions_output_token_parameter = Some(parameter);
            request.validate().expect("chat completions parameter");
        }
        assert!(
            serde_json::to_string(&request.runtime.providers[0])
                .expect("encode")
                .contains("\"chat_completions_output_token_parameter\":\"omit\"")
        );

        request.runtime.providers[0].kind = ManagedProviderKind::OpenAiResponses;
        assert_eq!(
            request.validate(),
            Err(ProtocolError::InvalidFrame),
            "the parameter shapes Chat Completions requests only"
        );
        request.runtime.providers[0].chat_completions_output_token_parameter = None;
        request
            .validate()
            .expect("responses provider without the parameter");
    }

    #[test]
    fn managed_model_identifiers_are_bounded_renderer_safe_tokens() {
        for valid in ["gpt-5.2", "openai/gpt-oss-120b", "vendor:model_v1"] {
            assert!(validate_managed_model_identifier(valid).is_ok());
        }
        for invalid in ["", "model with spaces", "model\nforged", "模型"] {
            assert_eq!(
                validate_managed_model_identifier(invalid),
                Err(ProtocolError::InvalidFrame),
            );
        }
    }

    #[test]
    fn approval_broker_grant_is_same_application_scope_only_and_toolless() {
        let mut wrong_application = request();
        wrong_application.validate().expect("valid broker grant");

        wrong_application
            .approval_broker_grant
            .as_mut()
            .expect("broker")
            .application_id = "app:other".into();
        assert_eq!(
            wrong_application.validate(),
            Err(ProtocolError::InvalidFrame)
        );

        let mut extra_scope = request();
        extra_scope
            .approval_broker_grant
            .as_mut()
            .expect("broker")
            .scopes
            .push("runs:control".into());
        assert_eq!(extra_scope.validate(), Err(ProtocolError::InvalidFrame));

        let mut tool_authority = request();
        tool_authority
            .approval_broker_grant
            .as_mut()
            .expect("broker")
            .allowed_tools
            .push("shell.run".into());
        assert_eq!(tool_authority.validate(), Err(ProtocolError::InvalidFrame));

        let mut unbounded_role = request();
        unbounded_role
            .approval_broker_grant
            .as_mut()
            .expect("broker")
            .allowed_roles
            .push("unbounded-role".into());
        assert_eq!(unbounded_role.validate(), Err(ProtocolError::InvalidFrame));
    }

    #[test]
    fn approval_broker_delivery_requires_a_distinct_id_and_bearer_pair() {
        let broker_id = Uuid::now_v7().to_string();
        assert!(matching_optional_credential(&None, &None));
        assert!(matching_optional_credential(
            &Some(broker_id),
            &Some(SecretString::new("broker-secret").expect("secret"))
        ));
        assert!(!matching_optional_credential(
            &Some(Uuid::now_v7().to_string()),
            &None
        ));
        assert!(!matching_optional_credential(
            &None,
            &Some(SecretString::new("broker-secret").expect("secret"))
        ));

        let primary_id = Uuid::now_v7().to_string();
        let broker_id = Uuid::now_v7().to_string();
        let ready = ReadyResponse {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: Uuid::now_v7().to_string(),
            instance_id: Uuid::now_v7().to_string(),
            api_major: 1,
            deployment_mode: "sidecar".into(),
            endpoint: "https://127.0.0.1:443".into(),
            certificate_pem: "certificate".into(),
            certificate_sha256: "0".repeat(64),
            credential_id: primary_id.clone(),
            bearer: SecretString::new("primary-secret").expect("secret"),
            approval_broker_credential_id: Some(broker_id),
            approval_broker_bearer: Some(SecretString::new("approval-secret").expect("secret")),
        };
        ready.validate().expect("paired delivery");
        let debug = format!("{ready:?}");
        assert!(!debug.contains("primary-secret"));
        assert!(!debug.contains("approval-secret"));

        let duplicated = ReadyResponse {
            approval_broker_credential_id: Some(primary_id),
            ..ready
        };
        assert_eq!(duplicated.validate(), Err(ProtocolError::InvalidFrame));
    }
}
