use colossus_cloud::CloudPermission;
use colossus_journal_postgres::PostgresJournalConfig;
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
    pub oidc: OidcConfig,
    /// Explicit issuer-bound subject/project memberships.
    pub memberships: Vec<Membership>,
    /// Built web frontend directory served at the authenticated application origin.
    pub web_root: PathBuf,
    /// Canonical cloud journal adapter.
    pub storage: Storage,
    /// Independent 32-byte Ed25519 checkpoint seed reference, required in production.
    pub signing_key_variable: Option<String>,
    /// Allow HTTP origins and plaintext state only on loopback for local acceptance.
    #[serde(default)]
    pub local_development: bool,
}
/// OIDC authorization-code/PKCE sign-in configuration.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    /// Exact trusted issuer; discovered metadata must match it.
    pub issuer: String,
    /// Registered relying-party client identity.
    pub client_id: String,
    /// Optional mounted confidential-client secret.
    pub client_secret_file: Option<PathBuf>,
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
/// Canonical storage selection, with credentials resolved only by its adapter.
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Storage {
    /// Owner-private local redb; plaintext requires local development opt-in.
    Redb {
        /// Owner-private canonical state path.
        path: PathBuf,
        /// Explicit injected journal encryption key reference.
        key_variable: Option<String>,
    },
    /// Existing encrypted PostgreSQL journal for Kubernetes deployment.
    Postgres {
        /// Existing PostgreSQL adapter connection and TLS policy.
        config: PostgresJournalConfig,
        /// Injected journal encryption key reference.
        key_variable: String,
        /// Owner-private retained secure anchor path.
        anchor_path: PathBuf,
    },
}
impl Config {
    /// Validate origins and namespaces before network or credential access.
    pub fn validate(&self) -> Result<(), &'static str> {
        let origin = url::Url::parse(&self.public_origin).map_err(|_| "invalid public origin")?;
        let endpoint = url::Url::parse(&self.grpc_endpoint).map_err(|_| "invalid gRPC endpoint")?;
        let issuer = url::Url::parse(&self.oidc.issuer).map_err(|_| "invalid OIDC issuer")?;
        let loopback =
            |url: &url::Url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if endpoint.scheme() != "https"
            || endpoint.path() != "/"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !issuer.username().is_empty()
            || issuer.password().is_some()
            || issuer.query().is_some()
            || issuer.fragment().is_some()
            || !matches!(origin.scheme(), "http" | "https")
            || !matches!(issuer.scheme(), "http" | "https")
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
                || !loopback(&issuer)
                || !loopback(&endpoint)
            {
                return Err("local development requires loopback endpoints");
            }
        } else if origin.scheme() != "https" || issuer.scheme() != "https" {
            return Err("production requires HTTPS and OIDC");
        }
        if !self.local_development
            && matches!(&self.storage, Storage::Postgres{config,..} if matches!(config.tls,colossus_journal_postgres::PostgresTlsConfig::Disabled))
        {
            return Err("production PostgreSQL requires verified TLS");
        }
        if self.memberships.is_empty() || self.memberships.len() > 1024 {
            return Err("invalid project memberships");
        }
        for member in &self.memberships {
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
        if matches!(
            &self.storage,
            Storage::Redb {
                key_variable: None,
                ..
            }
        ) && !self.local_development
        {
            return Err("production state requires encryption");
        }
        if !self.local_development && self.signing_key_variable.is_none() {
            return Err("production state requires checkpoint signing");
        }
        if let Some(signing) = &self.signing_key_variable {
            let journal = match &self.storage {
                Storage::Redb { key_variable, .. } => key_variable.as_ref(),
                Storage::Postgres { key_variable, .. } => Some(key_variable),
            };
            if Some(signing) == journal {
                return Err("journal and signing keys must be independent");
            }
        }
        Ok(())
    }
}
