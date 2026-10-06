use colossus_cloud::CloudPermission;
use colossus_cloud_postgres::{CloudDatabaseConfig, CloudDatabaseTls};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf};

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
