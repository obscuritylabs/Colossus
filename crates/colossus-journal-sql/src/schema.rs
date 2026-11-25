use colossus_journal_format::GENESIS_HASH;
use colossus_ports::{JournalPayloadProtection, StoreError};

use crate::driver::{Connection, Param, SqlJournalBackend};

pub(crate) const MAX_SQL_SEQUENCE: u64 = i64::MAX as u64;

const SCHEMA: &[&str] = &[
    "CREATE TABLE journal_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        format INTEGER NOT NULL CHECK (format = 1),
        backend TEXT NOT NULL,
        protection TEXT NOT NULL,
        sequence INTEGER NOT NULL CHECK (sequence >= 0),
        record_hash TEXT NOT NULL
    )",
    "CREATE TABLE events (
        sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
        stream_id BLOB NOT NULL,
        stream_version INTEGER NOT NULL CHECK (stream_version > 0),
        event_id TEXT NOT NULL,
        record_hash TEXT NOT NULL,
        envelope BLOB NOT NULL,
        UNIQUE (stream_id, stream_version)
    )",
    "CREATE TABLE streams (
        stream_id BLOB NOT NULL PRIMARY KEY,
        version INTEGER NOT NULL CHECK (version > 0),
        tail_sequence INTEGER NOT NULL CHECK (tail_sequence > 0)
    )",
    "CREATE TABLE projection_outbox (
        sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
        event_id TEXT NOT NULL
    )",
    "CREATE TABLE checkpoints (
        sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
        checkpoint BLOB NOT NULL
    )",
    "CREATE TABLE projection_positions (
        projection TEXT NOT NULL PRIMARY KEY,
        position INTEGER NOT NULL CHECK (position >= 0)
    )",
    "CREATE TABLE projection_records (
        projection TEXT NOT NULL,
        record_key BLOB NOT NULL,
        record BLOB NOT NULL,
        PRIMARY KEY (projection, record_key)
    )",
];

pub(crate) fn initialize(
    db: &Connection,
    backend: SqlJournalBackend,
    protection: JournalPayloadProtection,
) -> Result<(), StoreError> {
    let established = db
        .one(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'journal_state'",
            &[],
            |row| row.sequence(0),
        )?
        .is_some();
    if established {
        let stored = db.one(
            "SELECT format, backend, protection FROM journal_state WHERE id = 1",
            &[],
            |row| {
                Ok((
                    row.sequence(0)?,
                    row.text(1)?.into_owned(),
                    row.text(2)?.into_owned(),
                ))
            },
        )?;
        if stored.as_ref().is_none_or(|(version, engine, mode)| {
            *version != 1 || engine != backend.name() || mode != protection.as_str()
        }) {
            return Err(invalid(
                "journal format, backend, or protection mode does not match",
            ));
        }
    } else if db
        .one("SELECT 1 FROM sqlite_master LIMIT 1", &[], |row| {
            row.sequence(0)
        })?
        .is_some()
    {
        return Err(invalid("database already contains an unrelated schema"));
    }

    let wal_pragma = if established {
        "PRAGMA journal_mode"
    } else {
        "PRAGMA journal_mode = WAL"
    };
    let wal = db.one(wal_pragma, &[], |row| {
        Ok(row.text(0)?.eq_ignore_ascii_case("wal"))
    })?;
    if wal != Some(true) {
        return Err(invalid("embedded journal requires WAL mode"));
    }
    db.execute("PRAGMA synchronous = FULL", &[])?;
    if db.one("PRAGMA synchronous", &[], |row| row.sequence(0))? != Some(2) {
        return Err(invalid("embedded journal requires FULL synchronization"));
    }
    if backend == SqlJournalBackend::Sqlite {
        // Turso's WAL has this threshold built in and does not expose the pragma.
        db.visit("PRAGMA wal_autocheckpoint = 1000", &[], |_| Ok(()))?;
        db.execute("PRAGMA trusted_schema = OFF", &[])?;
    }

    if !established {
        db.write(|| {
            for sql in SCHEMA {
                db.execute(sql, &[])?;
            }
            db.execute(
                "INSERT INTO journal_state VALUES (1, 1, ?1, ?2, 0, ?3)",
                &[
                    Param::Text(backend.name()),
                    Param::Text(protection.as_str()),
                    Param::Text(GENESIS_HASH),
                ],
            )
        })?;
    }
    Ok(())
}

pub(crate) fn head(db: &Connection) -> Result<(u64, String), StoreError> {
    db.one(
        "SELECT sequence, record_hash FROM journal_state WHERE id = 1",
        &[],
        |row| Ok((row.sequence(0)?, row.text(1)?.into_owned())),
    )?
    .ok_or_else(|| invalid("journal head is absent"))
}

pub(crate) fn sql_sequence(sequence: u64) -> Result<Param<'static>, StoreError> {
    i64::try_from(sequence)
        .map(Param::Integer)
        .map_err(|_| StoreError::Adapter("sequence exceeds embedded journal range".into()))
}

pub(crate) fn sql_limit(limit: usize) -> Param<'static> {
    Param::Integer(i64::try_from(limit).unwrap_or(i64::MAX))
}

pub(crate) fn prefix_range(
    prefix: &str,
    after: Option<&str>,
) -> Result<(Vec<u8>, Vec<u8>), StoreError> {
    if prefix.contains('\0')
        || after.is_some_and(|key| key.contains('\0') || !key.starts_with(prefix))
    {
        return Err(StoreError::Adapter(
            "prefix and cursor must contain no NUL and share one prefix".into(),
        ));
    }
    let mut start = after.unwrap_or(prefix).as_bytes().to_vec();
    if after.is_some() {
        // The immediate successor of a byte string is that string followed by zero.
        start.push(0);
    }
    let mut end = prefix.as_bytes().to_vec();
    if let Some(last) = end.last_mut() {
        // A valid UTF-8 string cannot contain byte 255.
        *last += 1;
    } else {
        end.push(u8::MAX);
    }
    Ok((start, end))
}

pub(crate) fn invalid(reason: &str) -> StoreError {
    StoreError::Verification(reason.into())
}

pub(crate) fn decode<'a, T: serde::Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| invalid("invalid stored embedded journal JSON"))
}

pub(crate) fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value)
        .map_err(|_| StoreError::Adapter("cannot encode embedded journal JSON".into()))
}
