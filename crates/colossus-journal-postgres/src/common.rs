use super::*;

pub(super) const ZERO_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
pub(super) const CHECKPOINT_INTERVAL: u64 = 100;
pub(super) const CHECKPOINT_MAX_AGE: Duration = Duration::from_secs(60);
pub(super) const DEFAULT_STATEMENT_TIMEOUT_MS: u64 = 30_000;

pub(super) const TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS journal_metadata (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    last_sequence BIGINT NOT NULL CHECK (last_sequence >= 0),
    last_hash TEXT NOT NULL,
    latest_checkpoint BYTEA NULL
);
INSERT INTO journal_metadata (singleton, last_sequence, last_hash)
VALUES (TRUE, 0, '0000000000000000000000000000000000000000000000000000000000000000')
ON CONFLICT (singleton) DO NOTHING;

CREATE TABLE IF NOT EXISTS journal_events (
    global_sequence BIGINT PRIMARY KEY CHECK (global_sequence > 0),
    event_id TEXT NOT NULL UNIQUE,
    stream_id TEXT NOT NULL,
    stream_version BIGINT NOT NULL CHECK (stream_version > 0),
    envelope BYTEA NOT NULL,
    UNIQUE (stream_id, stream_version)
);
CREATE INDEX IF NOT EXISTS journal_events_stream_idx
ON journal_events (stream_id, stream_version);

CREATE TABLE IF NOT EXISTS journal_stream_versions (
    stream_id TEXT PRIMARY KEY,
    stream_version BIGINT NOT NULL CHECK (stream_version > 0)
);

CREATE TABLE IF NOT EXISTS projection_outbox (
    global_sequence BIGINT PRIMARY KEY REFERENCES journal_events(global_sequence),
    event_id TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS projection_positions (
    projection TEXT PRIMARY KEY,
    position BIGINT NOT NULL CHECK (position >= 0)
);

CREATE TABLE IF NOT EXISTS projection_records (
    projection TEXT NOT NULL,
    record_key TEXT NOT NULL,
    value BYTEA NOT NULL,
    PRIMARY KEY (projection, record_key)
);
"#;

pub(super) fn adapter_error(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(super) fn database_error(error: postgres::Error) -> StoreError {
    if let Some(db) = error.as_db_error() {
        StoreError::Adapter(format!(
            "PostgreSQL rejected operation ({})",
            db.code().code()
        ))
    } else {
        StoreError::Adapter("PostgreSQL is unavailable".into())
    }
}

pub(super) fn commit_error(_error: postgres::Error) -> StoreError {
    StoreError::OutcomeUnknown("PostgreSQL commit outcome is unknown".into())
}

pub(super) fn utc_now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(adapter_error)
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn to_i64(value: u64, label: &str) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Adapter(format!("{label} exceeds PostgreSQL BIGINT")))
}

pub(super) fn to_u64(value: i64, label: &str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Verification(format!("{label} is negative")))
}

pub(super) fn bounded_limit(limit: usize) -> Result<i64, StoreError> {
    Ok(i64::try_from(limit).unwrap_or(i64::MAX))
}

pub(super) fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && value.len() <= 63
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

pub(super) fn validate_projection(projection: &str) -> Result<(), StoreError> {
    if projection.is_empty() || projection.contains('\0') {
        return Err(StoreError::Adapter(
            "projection name must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_record_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty() || key.contains('\0') {
        return Err(StoreError::Adapter(
            "projection key must be nonempty and contain no NUL".into(),
        ));
    }
    Ok(())
}
