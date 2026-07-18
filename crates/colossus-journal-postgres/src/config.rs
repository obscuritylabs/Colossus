use super::*;

/// PostgreSQL TLS policy. Disabling TLS is intended only for isolated local acceptance.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PostgresTlsConfig {
    /// Require TLS with the pinned Mozilla WebPKI root set.
    #[default]
    WebpkiRoots,
    /// Require TLS and trust only the certificates in one PEM CA bundle.
    CustomCa {
        /// PEM CA-bundle path read only by the adapter.
        ca_pem_path: PathBuf,
    },
    /// Disable TLS explicitly for isolated loopback development and CI.
    Disabled,
}

/// Credential-reference-only PostgreSQL journal configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostgresJournalConfig {
    /// Environment variable containing a libpq-style URL or key/value connection string.
    pub connection_variable: String,
    /// Dedicated PostgreSQL schema owned by this Colossus instance.
    pub schema: String,
    /// TLS verification policy.
    #[serde(default)]
    pub tls: PostgresTlsConfig,
    /// Per-connection statement and lock timeout.
    #[serde(default = "default_statement_timeout_ms")]
    pub statement_timeout_ms: u64,
}

const fn default_statement_timeout_ms() -> u64 {
    DEFAULT_STATEMENT_TIMEOUT_MS
}

impl PostgresJournalConfig {
    /// Construct and validate a PostgreSQL adapter configuration.
    pub fn new(
        connection_variable: impl Into<String>,
        schema: impl Into<String>,
        tls: PostgresTlsConfig,
    ) -> Result<Self, StoreError> {
        let config = Self {
            connection_variable: connection_variable.into(),
            schema: schema.into(),
            tls,
            statement_timeout_ms: DEFAULT_STATEMENT_TIMEOUT_MS,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate identifiers and bounded timeout values without resolving credentials.
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.connection_variable.is_empty()
            || !self
                .connection_variable
                .bytes()
                .enumerate()
                .all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                })
        {
            return Err(StoreError::Adapter(
                "PostgreSQL connection variable must be a POSIX-style environment name".into(),
            ));
        }
        if !valid_identifier(&self.schema) {
            return Err(StoreError::Adapter(
                "PostgreSQL schema must be a 1-63 byte ASCII identifier".into(),
            ));
        }
        if !(100..=300_000).contains(&self.statement_timeout_ms) {
            return Err(StoreError::Adapter(
                "PostgreSQL statement timeout must be between 100 and 300000 ms".into(),
            ));
        }
        Ok(())
    }
}
