//! Independent protected anchor authority. Deploy with a dedicated database, role,
//! encryption key and backups outside the runtime journal's administration boundary.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use colossus_cloud::{CloudError, CloudResult};
use colossus_cloud_chat::{
    SharedAnchorSnapshot, SharedAnchorUpdate, validate_shared_anchor_advance,
};
use colossus_cloud_postgres::{
    ChatRecord, ChatRecordWrite, ChatSqlStore, CloudDatabaseConfig, CloudPostgresStore,
};
use colossus_network::AdditionalRootCertificates;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc};
use zeroize::Zeroizing;

mod custody;
mod records;
#[cfg(test)]
mod tests;

const RECORD: &str = "protected-anchor-v1";

/// Secret-reference-only configuration of the independently deployed authority.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnchorAuthorityConfig {
    /// Private listener, or a listener behind an explicitly trusted TLS ingress.
    pub bind: SocketAddr,
    /// Dedicated independent database; never point this at the runtime journal DB.
    pub database: CloudDatabaseConfig,
    /// Environment reference to an independently rotated bearer credential.
    pub bearer_variable: String,
    /// Environment reference to a separate 32-byte hex-encoded storage encryption key.
    pub storage_key_variable: String,
    /// Explicit operator attestation that non-loopback traffic reaches a TLS ingress.
    #[serde(default)]
    pub trusted_tls_ingress: bool,
}

impl AnchorAuthorityConfig {
    /// Validate without resolving or returning any secret.
    pub fn validate(&self) -> CloudResult<()> {
        self.database.validate()?;
        for variable in [&self.bearer_variable, &self.storage_key_variable] {
            if variable.is_empty()
                || variable.len() > 128
                || !variable.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                })
            {
                return Err(CloudError::InvalidArgument);
            }
        }
        if !self.bind.ip().is_loopback() && !self.trusted_tls_ingress {
            return Err(CloudError::InvalidArgument);
        }
        Ok(())
    }
}

struct AuthorityState {
    records: Arc<dyn records::AnchorRecords>,
    key: Zeroizing<[u8; 32]>,
    bearer_hash: [u8; 32],
    admission: Arc<tokio::sync::Semaphore>,
}

/// Independent HTTP authority, with atomic encrypted CAS storage and no delete API.
pub struct AnchorAuthority {
    bind: SocketAddr,
    state: Arc<AuthorityState>,
}

impl AnchorAuthority {
    /// Resolve explicit secrets, migrate the dedicated database and open custody.
    pub async fn open(config: AnchorAuthorityConfig) -> CloudResult<Self> {
        config.validate()?;
        let bearer_hash = custody::bearer(&config.bearer_variable)?;
        let key = custody::key(&config.storage_key_variable)?;
        let migrations = CloudPostgresStore::open(
            config.database.clone(),
            &AdditionalRootCertificates::default(),
        )
        .await?;
        drop(migrations);
        let records = tokio::task::spawn_blocking(move || ChatSqlStore::open(config.database))
            .await
            .map_err(|_| CloudError::Storage)??;
        Ok(Self {
            bind: config.bind,
            state: Arc::new(AuthorityState {
                records: Arc::new(records),
                key,
                bearer_hash,
                admission: Arc::new(tokio::sync::Semaphore::new(16)),
            }),
        })
    }

    /// Authenticated bounded authority routes; the runtime client uses this fixed protocol.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/v1/anchors/{namespace}", get(read).post(update))
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(self.state.clone())
    }

    /// Serve until the supplied shutdown future completes. External TLS belongs to ingress.
    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(self.bind).await?;
        axum::serve(listener, self.router())
            .with_graceful_shutdown(shutdown)
            .await
    }
}

async fn read(
    State(state): State<Arc<AuthorityState>>,
    Path(namespace): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SharedAnchorSnapshot>, StatusCode> {
    custody::authorize(&state, &headers)?;
    valid_namespace(&namespace)?;
    let permit = state
        .admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.snapshot(&namespace)
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map(Json)
    .map_err(status)
}

async fn update(
    State(state): State<Arc<AuthorityState>>,
    Path(namespace): Path<String>,
    headers: HeaderMap,
    Json(input): Json<SharedAnchorUpdate>,
) -> Result<Json<SharedAnchorSnapshot>, StatusCode> {
    custody::authorize(&state, &headers)?;
    valid_namespace(&namespace)?;
    if input.version != 1 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let permit = state
        .admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.advance(&namespace, input)
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map(Json)
    .map_err(status)
}

fn valid_namespace(value: &str) -> Result<(), StatusCode> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

fn status(error: CloudError) -> StatusCode {
    match error {
        CloudError::Conflict => StatusCode::CONFLICT,
        CloudError::InvalidArgument => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

impl AuthorityState {
    fn snapshot(&self, namespace: &str) -> CloudResult<SharedAnchorSnapshot> {
        let record = self.records.read(namespace)?;
        match record.ciphertext {
            None if record.revision == 0 => Ok(SharedAnchorSnapshot {
                version: 1,
                revision: 0,
                anchor: None,
            }),
            None => Err(CloudError::Storage),
            Some(bytes) => custody::decrypt(&self.key, namespace, record.revision, &bytes),
        }
    }

    fn advance(
        &self,
        namespace: &str,
        input: SharedAnchorUpdate,
    ) -> CloudResult<SharedAnchorSnapshot> {
        let previous = self.snapshot(namespace)?;
        if previous.revision != input.expected_revision {
            return Err(CloudError::Conflict);
        }
        validate_shared_anchor_advance(previous.anchor.as_ref(), &input.anchor)
            .map_err(|_| CloudError::Conflict)?;
        let next = SharedAnchorSnapshot {
            version: 1,
            revision: previous
                .revision
                .checked_add(1)
                .ok_or(CloudError::Storage)?,
            anchor: Some(input.anchor),
        };
        let encrypted = custody::encrypt(&self.key, namespace, &next)?;
        self.records
            .write(namespace, previous.revision, &encrypted)?;
        Ok(next)
    }
}
