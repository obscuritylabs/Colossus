use colossus_cloud::{CloudError, CloudResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Verified database transport policy, independent of runtime journal configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CloudDatabaseTls {
    /// Require rustls hostname verification against WebPKI and configured runtime roots.
    #[default]
    WebpkiRoots,
    /// Require rustls hostname verification against this explicit CA bundle.
    CustomCa {
        /// Native adapter-owned PEM trust path.
        ca_pem_path: PathBuf,
    },
    /// Explicit isolated loopback/Unix fixture only; remote plaintext is rejected.
    Disabled,
}

/// Credential-reference-only, bounded cloud PostgreSQL pool configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudDatabaseConfig {
    /// Environment variable containing the connection URL; never copied into logs.
    pub connection_variable: String,
    /// Dedicated cloud relational schema.
    pub schema: String,
    /// Verified TLS policy.
    #[serde(default)]
    pub tls: CloudDatabaseTls,
    /// At most this many open connections per cloud replica.
    #[serde(default = "max_connections")]
    pub max_connections: u32,
    /// Connection/pool acquisition deadline.
    #[serde(default = "connection_timeout_ms")]
    pub connection_timeout_ms: u64,
    /// Per-connection SQL statement and lock deadline.
    #[serde(default = "statement_timeout_ms")]
    pub statement_timeout_ms: u64,
}
const fn max_connections() -> u32 {
    16
}
const fn connection_timeout_ms() -> u64 {
    5000
}
const fn statement_timeout_ms() -> u64 {
    15000
}

impl CloudDatabaseConfig {
    /// Validate the nonsecret configuration before resolving credential references.
    pub fn validate(&self) -> CloudResult<()> {
        if !identifier(&self.connection_variable, 128)
            || !identifier(&self.schema, 63)
            || !(1..=128).contains(&self.max_connections)
            || !(100..=60000).contains(&self.connection_timeout_ms)
            || !(100..=300000).contains(&self.statement_timeout_ms)
        {
            return Err(CloudError::InvalidArgument);
        }
        Ok(())
    }
}
fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().enumerate().all(|(index, b)| {
            b == b'_' || b.is_ascii_alphabetic() || (index > 0 && b.is_ascii_digit())
        })
}
