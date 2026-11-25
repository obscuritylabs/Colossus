use std::{borrow::Cow, path::Path, time::Duration};

use colossus_ports::StoreError;
use futures_lite::future::block_on;
use rusqlite::types::{ToSql, ToSqlOutput, ValueRef};

/// Embedded database implementation used by a local journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlJournalBackend {
    /// Upstream SQLite, statically linked by rusqlite.
    Sqlite,
    /// Turso's local Rust engine using write-ahead logging.
    Turso,
}

impl SqlJournalBackend {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Turso => "turso",
        }
    }
}

pub(crate) enum Param<'a> {
    Integer(i64),
    Text(&'a str),
    Bytes(&'a [u8]),
}

impl ToSql for Param<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let value = match self {
            Self::Integer(value) => ValueRef::Integer(*value),
            Self::Text(value) => ValueRef::Text(value.as_bytes()),
            Self::Bytes(value) => ValueRef::Blob(value),
        };
        Ok(ToSqlOutput::Borrowed(value))
    }
}

fn turso_params(params: &[Param<'_>]) -> Vec<turso::Value> {
    params
        .iter()
        .map(|param| match param {
            Param::Integer(value) => turso::Value::Integer(*value),
            Param::Text(value) => turso::Value::Text((*value).into()),
            Param::Bytes(value) => turso::Value::Blob(value.to_vec()),
        })
        .collect()
}

pub(crate) enum Row<'a, 'stmt> {
    Sqlite(&'a rusqlite::Row<'stmt>),
    Turso(&'a turso::Row),
}

impl<'a> Row<'a, '_> {
    pub(crate) fn sequence(&self, column: usize) -> Result<u64, StoreError> {
        let integer: i64 = match self {
            Self::Sqlite(row) => row.get(column).map_err(invalid_column)?,
            Self::Turso(row) => row.get(column).map_err(invalid_column)?,
        };
        u64::try_from(integer).map_err(invalid_column)
    }

    pub(crate) fn bytes(&self, column: usize) -> Result<Cow<'a, [u8]>, StoreError> {
        match self {
            Self::Sqlite(row) => row
                .get_ref(column)
                .and_then(|value| value.as_blob().map_err(Into::into))
                .map(Cow::Borrowed)
                .map_err(invalid_column),
            Self::Turso(row) => row.get(column).map(Cow::Owned).map_err(invalid_column),
        }
    }

    pub(crate) fn text(&self, column: usize) -> Result<Cow<'a, str>, StoreError> {
        match self {
            Self::Sqlite(row) => row
                .get_ref(column)
                .and_then(|value| value.as_str().map_err(Into::into))
                .map(Cow::Borrowed)
                .map_err(invalid_column),
            Self::Turso(row) => row.get(column).map(Cow::Owned).map_err(invalid_column),
        }
    }
}

fn invalid_column(_: impl std::fmt::Debug) -> StoreError {
    StoreError::Verification("invalid embedded journal column encoding".into())
}

fn sqlite_error(error: rusqlite::Error) -> StoreError {
    use rusqlite::ErrorCode;
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            StoreError::Verification("invalid SQLite journal database".into())
        }
        code => StoreError::Adapter(format!("SQLite journal operation failed ({code:?})")),
    }
}

fn turso_error(error: turso::Error) -> StoreError {
    use turso::Error;
    let reason = match error {
        Error::Corrupt(_) | Error::NotAdb(_) => {
            return StoreError::Verification("invalid Turso journal database".into());
        }
        Error::Busy(_) | Error::BusySnapshot(_) => "busy",
        Error::Constraint(_) => "constraint",
        Error::DatabaseFull(_) => "disk full",
        Error::IoError(_, _) => "I/O",
        Error::Readonly(_) => "read-only",
        Error::Interrupt(_) => "interrupted",
        _ => "statement or connection",
    };
    StoreError::Adapter(format!("Turso journal operation failed ({reason})"))
}

pub(crate) enum Connection {
    Sqlite(rusqlite::Connection),
    Turso(turso::Connection),
}

impl Connection {
    pub(crate) fn open(path: &Path, backend: SqlJournalBackend) -> Result<Self, StoreError> {
        let timeout = Duration::from_secs(5);
        match backend {
            SqlJournalBackend::Sqlite => {
                let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                    | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW;
                let connection =
                    rusqlite::Connection::open_with_flags(path, flags).map_err(sqlite_error)?;
                connection.busy_timeout(timeout).map_err(sqlite_error)?;
                connection.set_prepared_statement_cache_capacity(64);
                Ok(Self::Sqlite(connection))
            }
            SqlJournalBackend::Turso => {
                let path = path.to_str().ok_or_else(|| {
                    StoreError::Adapter("Turso journal path must be UTF-8".into())
                })?;
                let database =
                    block_on(turso::Builder::new_local(path).build()).map_err(turso_error)?;
                let connection = database.connect().map_err(turso_error)?;
                connection.busy_timeout(timeout).map_err(turso_error)?;
                Ok(Self::Turso(connection))
            }
        }
    }

    pub(crate) fn execute(&self, sql: &str, params: &[Param<'_>]) -> Result<(), StoreError> {
        match self {
            Self::Sqlite(connection) => {
                let mut statement = connection.prepare_cached(sql).map_err(sqlite_error)?;
                statement
                    .execute(rusqlite::params_from_iter(params))
                    .map_err(sqlite_error)?;
            }
            Self::Turso(connection) => {
                block_on(async {
                    let mut statement = connection.prepare_cached(sql).await?;
                    statement.execute(turso_params(params)).await
                })
                .map_err(turso_error)?;
            }
        }
        Ok(())
    }

    pub(crate) fn visit(
        &self,
        sql: &str,
        params: &[Param<'_>],
        mut visit: impl FnMut(Row<'_, '_>) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        match self {
            Self::Sqlite(connection) => {
                let mut statement = connection.prepare_cached(sql).map_err(sqlite_error)?;
                let mut rows = statement
                    .query(rusqlite::params_from_iter(params))
                    .map_err(sqlite_error)?;
                while let Some(row) = rows.next().map_err(sqlite_error)? {
                    visit(Row::Sqlite(row))?;
                }
            }
            Self::Turso(connection) => {
                block_on(async {
                    let mut statement =
                        connection.prepare_cached(sql).await.map_err(turso_error)?;
                    let mut rows = statement
                        .query(turso_params(params))
                        .await
                        .map_err(turso_error)?;
                    while let Some(row) = rows.next().await.map_err(turso_error)? {
                        visit(Row::Turso(&row))?;
                    }
                    Ok::<(), StoreError>(())
                })?;
            }
        }
        Ok(())
    }

    pub(crate) fn one<T>(
        &self,
        sql: &str,
        params: &[Param<'_>],
        mut decode: impl FnMut(Row<'_, '_>) -> Result<T, StoreError>,
    ) -> Result<Option<T>, StoreError> {
        let mut result = None;
        self.visit(sql, params, |row| {
            if result.is_some() {
                return Err(StoreError::Verification(
                    "embedded journal query returned duplicate rows".into(),
                ));
            }
            result = Some(decode(row)?);
            Ok(())
        })?;
        Ok(result)
    }

    /// Finish the transaction before releasing the serialized connection.
    pub(crate) fn write<T>(
        &self,
        operation: impl FnOnce() -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.execute("BEGIN IMMEDIATE", &[])?;
        match operation() {
            Ok(value) => {
                if self.execute("COMMIT", &[]).is_err() {
                    let _ = self.rollback();
                    return Err(StoreError::OutcomeUnknown(
                        "embedded journal commit could not be confirmed".into(),
                    ));
                }
                Ok(value)
            }
            Err(error) => {
                self.rollback().map_err(|_| {
                    StoreError::OutcomeUnknown(
                        "embedded journal rollback could not be confirmed".into(),
                    )
                })?;
                Err(error)
            }
        }
    }

    fn rollback(&self) -> Result<(), StoreError> {
        let autocommit = match self {
            Self::Sqlite(connection) => connection.is_autocommit(),
            Self::Turso(connection) => connection.is_autocommit().map_err(turso_error)?,
        };
        if !autocommit {
            self.execute("ROLLBACK", &[])?;
        }
        Ok(())
    }
}
