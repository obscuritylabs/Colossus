//! Private, bounded bootstrap frames shared by the native SDK launcher and sidecar host.
//!
//! This is not a network protocol. Frames travel only over anonymous handles inherited
//! by a freshly verified child process. Secret fields are zeroized and redact debug
//! output; callers must also zeroize encoded frames after writing them.

#![allow(clippy::missing_errors_doc)]

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    fmt,
    io::{Read, Write},
    path::{Component, Path},
};
use thiserror::Error;
use url::{Host, Url};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Exact bootstrap protocol version.
pub const PROTOCOL_VERSION: u16 = 2;
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
const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_AUTHORIZATION_ITEMS: usize = 512;
const MAX_CERTIFICATE_PEM_BYTES: usize = 256 * 1024;
const APPROVALS_RESPOND_SCOPE: &str = "approvals:respond";
const LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION: u16 = 1;
const LEGACY_UNIX_WORKSPACE_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-unix-device-inode-v1\0";
const MACOS_WORKSPACE_IDENTITY_VERSION: u16 = 2;
const MACOS_WORKSPACE_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-macos-device-inode-birthtime-v2\0";

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

    /// Validate a bounded wire identity. Version 1 is accepted only so an ephemeral
    /// non-macOS SDK sidecar can retain its existing descriptor-lifetime contract.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !self.version_was_missing
            && matches!(
                self.version,
                LEGACY_UNIX_WORKSPACE_IDENTITY_VERSION | MACOS_WORKSPACE_IDENTITY_VERSION
            )
            && lowercase_sha256(&self.sha256)
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidFrame)
        }
    }

    /// Require the non-reusable macOS identity used by persisted Managed Desktop state.
    pub fn validate_current(&self) -> Result<(), ProtocolError> {
        if !self.version_was_missing
            && self.version == MACOS_WORKSPACE_IDENTITY_VERSION
            && lowercase_sha256(&self.sha256)
        {
            Ok(())
        } else {
            Err(ProtocolError::InvalidFrame)
        }
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
}

/// Compact provider settings that contain references but never credential values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedProviderConfig {
    /// Selected first-party adapter.
    pub kind: ManagedProviderKind,
    /// Exact model identifier.
    pub model: String,
    /// API-version base URL for a network provider.
    pub base_url: Option<String>,
    /// Opaque host credential identifier without the `host:` prefix.
    pub credential_id: Option<String>,
}

impl ManagedProviderConfig {
    /// Validate compact provider settings without resolving a secret or network name.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !valid_token(&self.model) || self.base_url.as_ref().is_some_and(|url| url.len() > 2_048)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        match self.kind {
            ManagedProviderKind::Echo => {
                if self.base_url.is_some() || self.credential_id.is_some() || self.model != "echo" {
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
                        .is_none_or(|credential_id| !valid_host_identifier(credential_id))
                {
                    return Err(ProtocolError::InvalidFrame);
                }
            }
        }
        Ok(())
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

/// Compact secret-free runtime settings generated into canonical sidecar YAML.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRuntimeConfig {
    /// Access and policy preset.
    pub access_profile: ManagedAccessProfile,
    /// Primary model provider.
    pub provider: ManagedProviderConfig,
}

impl ManagedRuntimeConfig {
    /// Validate the compact configuration.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.provider.validate()
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
            || self
                .allowed_tools
                .iter()
                .any(|tool| tool == "agent.delegate")
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
        let mut ids = BTreeSet::new();
        for credential in &self.host_credentials {
            credential.validate()?;
            if !ids.insert(credential.id.as_str()) {
                return Err(ProtocolError::InvalidFrame);
            }
        }
        if let Some(credential_id) = self.runtime.provider.credential_id.as_deref()
            && !ids.contains(credential_id)
        {
            return Err(ProtocolError::InvalidFrame);
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

    fn request() -> BootstrapRequest {
        BootstrapRequest {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: Uuid::now_v7().to_string(),
            instance_id: Uuid::now_v7().to_string(),
            api_major: 1,
            instance_dir: "/tmp/colossus-sidecar-instance".into(),
            workspace: "/tmp/colossus-sidecar-workspace".into(),
            workspace_identity: WorkspaceIdentity::from_unix_parts(42, 84),
            runtime: ManagedRuntimeConfig {
                access_profile: ManagedAccessProfile::Development,
                provider: ManagedProviderConfig {
                    kind: ManagedProviderKind::OpenAiCompatible,
                    model: "model-v1".into(),
                    base_url: Some("https://provider.example/v1".into()),
                    credential_id: Some("provider-main".into()),
                },
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
    fn secret_frames_round_trip_without_debug_disclosure() {
        let request = request();
        request.validate().expect("request");
        let frame = ParentFrame::Bootstrap(Box::new(request));
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).expect("write");
        let decoded: ParentFrame = read_frame(&mut Cursor::new(bytes)).expect("read");
        let ParentFrame::Bootstrap(decoded) = decoded else {
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

        let mut wrong_version = identity.clone();
        wrong_version.version += 1;
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
    fn frames_fail_closed_on_oversize_and_delegation_authority() {
        let mut request = request();
        request.grant.allowed_tools.push("agent.delegate".into());
        assert_eq!(request.validate(), Err(ProtocolError::InvalidFrame));

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
