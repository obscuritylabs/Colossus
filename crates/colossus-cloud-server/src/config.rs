use colossus_cloud::CloudPermission;
use colossus_cloud_postgres::{CloudDatabaseConfig, CloudDatabaseTls};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::PathBuf,
};

/// Secret-reference-only control-plane host configuration.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// HTTP listener; production terminates HTTPS at its trusted ingress.
    pub http_bind: SocketAddr,
    /// End-to-end TLS gRPC listener.
    pub grpc_bind: SocketAddr,
    /// Exact browser origin, also used for CSRF and OIDC callbacks.
    pub public_origin: String,
    /// Public outbound connector endpoint, always HTTPS.
    pub grpc_endpoint: String,
    /// PEM connector certificate authority.
    pub ca_certificate: PathBuf,
    /// PEM authority private key mounted from a Kubernetes Secret.
    pub ca_key: PathBuf,
    /// PEM server leaf, including its public chain.
    pub server_certificate: PathBuf,
    /// PEM server private key.
    pub server_key: PathBuf,
    /// OIDC discovery and relying-party configuration.
    pub oidc: Option<OidcConfig>,
    /// Local password authentication, disabled unless explicitly configured.
    #[serde(default)]
    pub local_auth: Option<LocalAuthConfig>,
    /// One-time first-administrator provisioning; no default credential exists.
    #[serde(default)]
    pub bootstrap_admin: Option<BootstrapAdmin>,
    /// Optional initial deployment marking; retained administrator edits take precedence.
    #[serde(default)]
    pub classification: Option<colossus_cloud::settings::ClassificationBanner>,
    /// One-time legacy issuer-bound membership seeds; persisted roles are authoritative.
    #[serde(default)]
    pub memberships: Vec<Membership>,
    /// Built web frontend directory served at the authenticated application origin.
    pub web_root: PathBuf,
    /// Dedicated cloud relational database and bounded asynchronous pool.
    pub database: CloudDatabaseConfig,
    /// Independent 32-byte OIDC-flow envelope key reference, required in production.
    pub auth_key_variable: Option<String>,
    /// Bounded operational cleanup; conversation history and audit are retained.
    #[serde(default)]
    pub maintenance: colossus_cloud::storage::CloudMaintenancePolicy,
    /// Allow HTTP origins and plaintext state only on loopback for local acceptance.
    #[serde(default)]
    pub local_development: bool,
    /// Optional controlled chat service. Absence never opens hosted runtime state.
    #[serde(default)]
    pub chat: Option<ChatConfig>,
}

/// Operator-owned hosted chat deployment; browser requests cannot deserialize this authority.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatConfig {
    /// Absolute owner-private service directory, independent from Control Plane SQL state.
    pub home: PathBuf,
    /// Exact environment variable wrapping encrypted user/project credential vaults.
    pub wrapping_key_variable: String,
    /// Shared SQL custody and independently deployed journal-anchor authority.
    /// Absence retains only the explicit single-process local development setup.
    #[serde(default)]
    pub shared: Option<ChatSharedConfig>,
    /// Bound resident tenant runtimes per worker process, independently from browser sessions.
    #[serde(default = "default_chat_limit")]
    pub max_active_runtimes: usize,
    /// Warm idle lifetime in seconds. Active work, interactions and resources pin ownership.
    #[serde(default = "default_hosted_idle_seconds")]
    pub idle_seconds: u64,
    /// Initial reviewed project snapshots. Retained administrative revisions take precedence.
    pub projects: BTreeMap<String, ChatProjectConfig>,
}

/// Operator-owned distributed chat durability. SQL uses the Control Plane's verified database.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSharedConfig {
    /// Externally managed encryption/signing keys and independent rollback-anchor authority.
    pub keys: colossus_cloud_chat::SharedChatKeyConfig,
}

/// Reviewed connections and model routes beneath one project's immutable egress ceiling.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatProjectConfig {
    /// Existing runtime configuration; controlled chat independently narrows its authority.
    pub runtime: colossus_runtime::RuntimeConfig,
    /// Display metadata is retained separately from runtime profiles and authority.
    #[serde(default)]
    pub labels: ChatSettingsLabels,
    /// Missing entries require each user's own credential; shared identity is explicit.
    #[serde(default)]
    pub mcp_authentication: BTreeMap<String, colossus_cloud_chat::ChatAuthentication>,
    /// Provider credentials follow the same personal-by-default rule.
    #[serde(default)]
    pub provider_authentication: BTreeMap<String, colossus_cloud_chat::ChatAuthentication>,
    /// Credential generations change only when that connection's identity changes.
    #[serde(default)]
    pub provider_generations: BTreeMap<String, u64>,
    /// Independent MCP credential generations preserve unrelated personal sign-ins.
    #[serde(default)]
    pub mcp_generations: BTreeMap<String, u64>,
    /// Exact configuration generation bound into credentials and browser OAuth flows.
    #[serde(default = "initial_chat_revision")]
    pub revision: u64,
    /// Optional host-pinned Codex account file, requiring explicit shared provider policy.
    #[serde(default)]
    pub codex_auth_path: Option<PathBuf>,
    /// Explicit operator-selected project Codex account in encrypted shared SQL custody.
    #[serde(default)]
    pub codex_auth_shared: bool,
}

/// Optional human-readable labels for existing hosted provider and model profiles.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSettingsLabels {
    /// Provider profile ID to display label; missing entries use the profile ID.
    #[serde(default)]
    pub providers: BTreeMap<String, String>,
    /// Model profile ID to display label; missing entries use the profile ID.
    #[serde(default)]
    pub models: BTreeMap<String, String>,
}

impl ChatSettingsLabels {
    pub(crate) fn validate(
        &self,
        runtime: &colossus_runtime::RuntimeConfig,
    ) -> Result<(), &'static str> {
        for (labels, profiles) in [
            (
                &self.providers,
                runtime.providers.profiles.keys().collect::<BTreeSet<_>>(),
            ),
            (
                &self.models,
                runtime.models.profiles.keys().collect::<BTreeSet<_>>(),
            ),
        ] {
            if labels.len() > profiles.len()
                || labels.iter().any(|(id, label)| {
                    !profiles.contains(id)
                        || label.trim().is_empty()
                        || label.len() > 96
                        || label.chars().any(char::is_control)
                })
            {
                return Err("hosted labels require existing profiles and 1–96 bytes of text");
            }
        }
        Ok(())
    }
}

fn default_chat_limit() -> usize {
    16
}
fn default_hosted_idle_seconds() -> u64 {
    300
}
fn initial_chat_revision() -> u64 {
    1
}

impl ChatConfig {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if let Some(shared) = &self.shared {
            shared
                .keys
                .validate()
                .map_err(|_| "invalid shared hosted key authority")?;
        }
        let variable = &self.wrapping_key_variable;
        if !self.home.is_absolute()
            || !(1..=64).contains(&self.max_active_runtimes)
            || !(1..=86400).contains(&self.idle_seconds)
            || variable.is_empty()
            || variable.len() > 256
            || !variable
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || !variable.as_bytes()[0].is_ascii_alphabetic()
            || self.projects.len() > 256
        {
            return Err("invalid hosted chat deployment");
        }
        for (project, configuration) in &self.projects {
            colossus_cloud::validate_identifier(project).map_err(|_| "invalid chat project")?;
            configuration.validate()?;
            if self.shared.is_some() && configuration.codex_auth_path.is_some() {
                return Err("native Codex account files are unavailable in shared hosted chat");
            }
        }
        Ok(())
    }
}

impl ChatProjectConfig {
    pub(crate) fn generation(
        &self,
        kind: colossus_cloud_chat::ChatConnectionKind,
        id: &str,
    ) -> u64 {
        match kind {
            colossus_cloud_chat::ChatConnectionKind::Provider => &self.provider_generations,
            colossus_cloud_chat::ChatConnectionKind::Mcp => &self.mcp_generations,
        }
        .get(id)
        .copied()
        .unwrap_or(1)
    }
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        use colossus_cloud_chat::ChatAuthentication;
        self.labels.validate(&self.runtime)?;
        let serialized = self
            .runtime
            .to_yaml()
            .map_err(|_| "invalid hosted runtime configuration")?;
        colossus_runtime::RuntimeConfig::from_yaml(&serialized)
            .map_err(|_| "invalid hosted runtime configuration")?;
        if self.revision == 0
            || self.runtime.models.roles.len() > 32
            || !matches!(
                self.runtime.storage.keys,
                colossus_runtime::KeyConfig::Environment { .. }
            )
        {
            return Err("hosted chat requires a versioned configuration and headless journal keys");
        }
        for (name, server) in &self.runtime.mcp.servers {
            let authentication = self
                .mcp_authentication
                .get(name)
                .copied()
                .unwrap_or_default();
            if server.oauth.is_some() && authentication != ChatAuthentication::Personal {
                return Err("hosted MCP OAuth requires personal authentication");
            }
            if server
                .oauth
                .as_ref()
                .is_some_and(|oauth| oauth.client_secret_reference.is_some())
            {
                return Err("hosted MCP OAuth requires public PKCE client configuration");
            }
            if server.credential_headers.len() > 1 {
                return Err("hosted MCP requires one credential header");
            }
            if authentication == ChatAuthentication::None
                && (!server.credential_headers.is_empty() || server.oauth.is_some())
            {
                return Err("unauthenticated MCP cannot declare credentials");
            }
        }
        if self
            .codex_auth_path
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err("hosted Codex account custody must be absolute");
        }
        for (name, provider) in &self.runtime.providers.profiles {
            if provider.kind.generation_action() == "provider.openai.codex"
                && ((!self.codex_auth_shared && self.codex_auth_path.is_none())
                    || self.provider_authentication.get(name) != Some(&ChatAuthentication::Shared))
            {
                return Err("hosted Codex requires explicit shared account custody");
            }
        }
        Ok(())
    }
}
/// OIDC authorization-code/PKCE sign-in configuration.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    /// Human-readable sign-in provider name, never HTML.
    #[serde(default = "default_oidc_label")]
    pub label: String,
    /// Exact trusted issuer; discovered metadata must match it.
    pub issuer: String,
    /// Registered relying-party client identity.
    pub client_id: String,
    /// Optional mounted confidential-client secret.
    pub client_secret_file: Option<PathBuf>,
}
fn default_oidc_label() -> String {
    "OpenID Connect".into()
}
/// Explicit local-authentication deployment switch.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAuthConfig {
    /// Default session duration, constrained to five minutes through eight hours.
    #[serde(default = "default_session_seconds")]
    pub session_seconds: u64,
}
fn default_session_seconds() -> u64 {
    8 * 60 * 60
}
/// Administrator bootstrap, used only while no persisted global administrator exists.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapAdmin {
    /// Initial administrator display name.
    pub display_name: String,
    /// Optional display-only contact address.
    pub email: Option<String>,
    /// Explicit issuer-bound subject under the configured provider.
    pub oidc_subject: Option<String>,
    /// Optional canonical local-login username.
    pub username: Option<String>,
    /// Reference to a one-time password secret; password values never enter config.
    pub password_variable: Option<String>,
}
/// One explicitly configured authenticated project membership.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    /// OIDC subject under the single configured issuer.
    pub subject: String,
    /// Project namespace.
    pub project_id: String,
    /// Independent cloud permissions.
    pub permissions: BTreeSet<CloudPermission>,
}
impl Config {
    /// Validate origins and namespaces before network or credential access.
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(chat) = &self.chat {
            chat.validate()?;
            if chat.shared.is_some()
                && matches!(&self.database.tls, CloudDatabaseTls::InCluster { .. })
            {
                return Err("shared hosted chat requires verified database TLS");
            }
            if !self.local_development
                && (chat.shared.is_none()
                    || chat
                        .shared
                        .as_ref()
                        .is_some_and(|shared| shared.keys.allow_loopback_http))
            {
                return Err(
                    "production hosted chat requires shared SQL and an HTTPS anchor authority",
                );
            }
        }
        let origin = url::Url::parse(&self.public_origin).map_err(|_| "invalid public origin")?;
        let endpoint = url::Url::parse(&self.grpc_endpoint).map_err(|_| "invalid gRPC endpoint")?;
        let issuer = self
            .oidc
            .as_ref()
            .map(|oidc| url::Url::parse(&oidc.issuer).map_err(|_| "invalid OIDC issuer"))
            .transpose()?;
        let loopback =
            |url: &url::Url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if endpoint.scheme() != "https"
            || endpoint.path() != "/"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || issuer.as_ref().is_some_and(|issuer| {
                !issuer.username().is_empty()
                    || issuer.password().is_some()
                    || issuer.query().is_some()
                    || issuer.fragment().is_some()
            })
            || !matches!(origin.scheme(), "http" | "https")
            || issuer
                .as_ref()
                .is_some_and(|issuer| !matches!(issuer.scheme(), "http" | "https"))
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err("invalid cloud origin");
        }
        if self.local_development {
            if !self.http_bind.ip().is_loopback()
                || !self.grpc_bind.ip().is_loopback()
                || !loopback(&origin)
                || issuer.as_ref().is_some_and(|issuer| !loopback(issuer))
                || !loopback(&endpoint)
            {
                return Err("local development requires loopback endpoints");
            }
        } else if origin.scheme() != "https"
            || issuer
                .as_ref()
                .is_some_and(|issuer| issuer.scheme() != "https")
        {
            return Err("production requires HTTPS authentication endpoints");
        }
        if !self.local_development && matches!(&self.database.tls, CloudDatabaseTls::Disabled) {
            return Err("production PostgreSQL requires verified TLS");
        }
        if self.oidc.is_none() && self.local_auth.is_none() {
            return Err("configure an OIDC provider or explicit local authentication");
        }
        if let Some(marking) = &self.classification {
            marking
                .validate()
                .map_err(|_| "invalid deployment marking")?;
        }
        if self.oidc.as_ref().is_some_and(|oidc| {
            oidc.client_id.is_empty()
                || oidc.client_id.len() > 256
                || oidc.label.trim().is_empty()
                || oidc.label.len() > 128
                || oidc.label.chars().any(char::is_control)
        }) {
            return Err("invalid OIDC provider");
        }
        if self
            .local_auth
            .as_ref()
            .is_some_and(|local| !(300..=8 * 60 * 60).contains(&local.session_seconds))
        {
            return Err("invalid local session duration");
        }
        if let Some(bootstrap) = &self.bootstrap_admin {
            if bootstrap.display_name.trim().is_empty()
                || bootstrap.display_name.len() > 256
                || bootstrap.display_name.chars().any(char::is_control)
                || bootstrap
                    .email
                    .as_ref()
                    .is_some_and(|s| s.len() > 320 || s.chars().any(char::is_control))
            {
                return Err("invalid administrator metadata");
            }
            if bootstrap
                .oidc_subject
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
                || bootstrap.oidc_subject.is_some() && self.oidc.is_none()
            {
                return Err("invalid administrator provider identity");
            }
            if let Some(username) = &bootstrap.username {
                colossus_cloud::normalize_username(username)
                    .map_err(|_| "invalid administrator username")?;
                if self.local_auth.is_none()
                    || bootstrap.password_variable.as_ref().is_none_or(|s| {
                        s.is_empty()
                            || s.len() > 128
                            || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    })
                {
                    return Err("administrator local login requires a secret reference");
                }
            } else if bootstrap.password_variable.is_some() {
                return Err("administrator password requires a username");
            }
            if bootstrap.username.is_none() && bootstrap.oidc_subject.is_none() {
                return Err("administrator requires a login identity");
            }
        }
        if self.memberships.len() > 1024 || (!self.memberships.is_empty() && self.oidc.is_none()) {
            return Err("invalid project memberships");
        }
        for member in &self.memberships {
            if member.project_id.starts_with("__") {
                return Err("reserved project namespace");
            }
            colossus_cloud::CloudCaller::new(
                member.subject.clone(),
                member.project_id.clone(),
                member.permissions.clone(),
            )
            .map_err(|_| "invalid membership")?;
        }
        let unique: BTreeSet<_> = self
            .memberships
            .iter()
            .map(|membership| (&membership.subject, &membership.project_id))
            .collect();
        if unique.len() != self.memberships.len() {
            return Err("duplicate project membership");
        }
        if !(1..=1024).contains(&self.maintenance.batch_limit)
            || !(60..=30 * 86400).contains(&self.maintenance.delivered_outbox_retention_seconds)
        {
            return Err("invalid cloud maintenance policy");
        }
        self.database
            .validate()
            .map_err(|_| "invalid cloud database configuration")?;
        if !self.local_development && self.oidc.is_some() && self.auth_key_variable.is_none() {
            return Err("production requires an OIDC-flow encryption key reference");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ChatProjectConfig;

    #[test]
    fn hosted_helm_example_uses_valid_project_configuration() {
        let values: serde_json::Value = serde_saphyr::from_str(include_str!(
            "../../../deploy/control-plane/chart/examples/hosted-chat.yaml"
        ))
        .expect("hosted deployment example must deserialize");
        assert_eq!(values["chat"]["enabled"], true);
        let projects: std::collections::BTreeMap<String, ChatProjectConfig> =
            serde_json::from_value(values["chat"]["projects"].clone())
                .expect("native hosted project snapshots");
        assert!(!projects.is_empty());
        for project in projects.values() {
            project.validate().expect("reviewed project authority");
            assert!(project.codex_auth_path.is_none());
        }
    }
}
