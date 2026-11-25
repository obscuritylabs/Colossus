use colossus_contracts::{ProjectionBatch, ProjectionMutation};
use colossus_ports::{ProjectionStore, StoreError};
use serde_json::Value;

use crate::{
    SqlEventJournal,
    driver::{Connection, Param},
    schema::{encode, invalid, prefix_range, sql_limit, sql_sequence},
};

impl ProjectionStore for SqlEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        validate_name(projection)?;
        self.with_state(false, |state| position(&state.db, projection))
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        validate_name(projection)?;
        validate_name(key)?;
        self.with_state(false, |state| {
            state.db.one(
                "SELECT record FROM projection_records WHERE projection = ?1 AND record_key = ?2",
                &[Param::Text(projection), Param::Bytes(key.as_bytes())],
                |row| decode_record(&row.bytes(0)?),
            )
        })
    }

    fn list(
        &self,
        projection: &str,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        self.list_after(projection, prefix, None, limit)
    }

    fn list_after(
        &self,
        projection: &str,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        validate_name(projection)?;
        if prefix.contains('\0') || after.is_some_and(|key| key.contains('\0')) {
            return Err(StoreError::Adapter(
                "projection cursor must contain no NUL".into(),
            ));
        }
        let after = after.filter(|key| *key >= prefix);
        if limit == 0 || after.is_some_and(|key| !key.starts_with(prefix)) {
            return Ok(Vec::new());
        }
        let (start, end) = prefix_range(prefix, after)?;
        self.with_state(false, |state| {
            let mut records = Vec::new();
            state.db.visit(
                "SELECT record_key, record FROM projection_records
                 WHERE projection = ?1 AND record_key >= ?2 AND record_key < ?3
                 ORDER BY record_key LIMIT ?4",
                &[
                    Param::Text(projection),
                    Param::Bytes(&start),
                    Param::Bytes(&end),
                    sql_limit(limit),
                ],
                |row| {
                    let key = String::from_utf8(row.bytes(0)?.into_owned())
                        .map_err(|_| StoreError::Adapter("invalid stored projection key".into()))?;
                    validate_name(&key)?;
                    records.push((key, decode_record(&row.bytes(1)?)?));
                    Ok(())
                },
            )?;
            Ok(records)
        })
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        self.apply_all(std::slice::from_ref(&batch))
    }

    fn apply_all(&self, batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        if batches.is_empty() {
            return Ok(());
        }
        self.with_state(true, |state| {
            let db = &state.db;
            db.write(|| {
                for batch in batches {
                    apply_batch(db, batch)?;
                }
                Ok(())
            })
        })
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        validate_name(projection)?;
        self.with_state(true, |state| {
            let db = &state.db;
            db.write(|| {
                let params = [Param::Text(projection)];
                db.execute(
                    "DELETE FROM projection_records WHERE projection = ?1",
                    &params,
                )?;
                db.execute(
                    "DELETE FROM projection_positions WHERE projection = ?1",
                    &params,
                )
            })
        })
    }
}

fn position(db: &Connection, projection: &str) -> Result<u64, StoreError> {
    Ok(db
        .one(
            "SELECT position FROM projection_positions WHERE projection = ?1",
            &[Param::Text(projection)],
            |row| row.sequence(0),
        )?
        .unwrap_or(0))
}

fn apply_batch(db: &Connection, batch: &ProjectionBatch) -> Result<(), StoreError> {
    validate_name(&batch.projection)?;
    if batch.through_sequence <= batch.expected_position {
        return Err(StoreError::Adapter(
            "projection position must advance".into(),
        ));
    }
    let actual = position(db, &batch.projection)?;
    if actual != batch.expected_position {
        return Err(StoreError::Conflict {
            stream_id: format!("projection:{}", batch.projection),
            expected: batch.expected_position,
            actual,
        });
    }
    let projection = &batch.projection;
    for mutation in &batch.mutations {
        match mutation {
            ProjectionMutation::Upsert { key, value } => {
                validate_name(key)?;
                db.execute(
                    "INSERT INTO projection_records (projection, record_key, record) VALUES (?1, ?2, ?3)
                     ON CONFLICT (projection, record_key) DO UPDATE SET record = excluded.record",
                    &[Param::Text(projection), Param::Bytes(key.as_bytes()), Param::Bytes(&encode(value)?)],
                )?;
            }
            ProjectionMutation::Delete { key } => {
                validate_name(key)?;
                db.execute(
                    "DELETE FROM projection_records WHERE projection = ?1 AND record_key = ?2",
                    &[Param::Text(projection), Param::Bytes(key.as_bytes())],
                )?;
            }
        }
    }
    db.execute(
        "INSERT INTO projection_positions (projection, position) VALUES (?1, ?2)
         ON CONFLICT (projection) DO UPDATE SET position = excluded.position",
        &[
            Param::Text(projection),
            sql_sequence(batch.through_sequence)?,
        ],
    )
}

pub(crate) fn verify_positions(db: &Connection, head: u64) -> Result<(), StoreError> {
    db.visit(
        "SELECT projection, position FROM projection_positions",
        &[],
        |row| {
            if !valid_name(&row.text(0)?) || row.sequence(1)? > head {
                return Err(invalid("embedded journal projection position is invalid"));
            }
            Ok(())
        },
    )
}

pub(crate) fn verify_records(db: &Connection) -> Result<(), StoreError> {
    db.visit(
        "SELECT projection, record_key, record FROM projection_records",
        &[],
        |row| {
            let key = row.bytes(1)?;
            let key = std::str::from_utf8(&key)
                .map_err(|_| invalid("invalid projection key encoding"))?;
            if !valid_name(&row.text(0)?) || !valid_name(key) {
                return Err(invalid(
                    "invalid embedded journal projection namespace or key",
                ));
            }
            decode_record(&row.bytes(2)?).map(|_| ())
        },
    )
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\0')
}

fn validate_name(name: &str) -> Result<(), StoreError> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "projection name and key must be nonempty and contain no NUL".into(),
        ))
    }
}

fn decode_record(bytes: &[u8]) -> Result<Value, StoreError> {
    serde_json::from_slice(bytes)
        .map_err(|_| StoreError::Adapter("invalid stored projection JSON".into()))
}
