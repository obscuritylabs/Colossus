use super::*;
use std::ops::Bound;

struct EncodedProjectionBatch {
    projection: String,
    expected_position: u64,
    through_sequence: u64,
    mutations: Vec<(String, Option<Vec<u8>>)>,
}

fn encode_projection_batch(batch: &ProjectionBatch) -> Result<EncodedProjectionBatch, StoreError> {
    projection_prefix(&batch.projection)?;
    if batch.through_sequence <= batch.expected_position {
        return Err(StoreError::Adapter(
            "projection position must advance".into(),
        ));
    }
    let mutations = batch
        .mutations
        .iter()
        .map(|mutation| match mutation {
            ProjectionMutation::Upsert { key, value } => Ok((
                projection_record_key(&batch.projection, key)?,
                Some(serde_json::to_vec(value).map_err(adapter_error)?),
            )),
            ProjectionMutation::Delete { key } => {
                Ok((projection_record_key(&batch.projection, key)?, None))
            }
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(EncodedProjectionBatch {
        projection: batch.projection.clone(),
        expected_position: batch.expected_position,
        through_sequence: batch.through_sequence,
        mutations,
    })
}

impl ProjectionStore for RedbEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        projection_prefix(projection)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        Ok(table
            .get(projection)
            .map_err(adapter_error)?
            .map_or(0, |position| position.value()))
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        let namespaced = projection_record_key(projection, key)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        table
            .get(namespaced.as_str())
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        self.list_after(projection, key_prefix, None, limit)
    }

    fn list_after(
        &self,
        projection: &str,
        key_prefix: &str,
        after_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        let namespace = projection_prefix(projection)?;
        if key_prefix.contains('\0') || after_key.is_some_and(|key| key.contains('\0')) {
            return Err(StoreError::Adapter(
                "projection key cursors may not contain NUL".into(),
            ));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        // Keys sharing a prefix are contiguous. Seek straight to that prefix or
        // beyond the exclusive cursor, then stop as soon as the prefix ends.
        let (start, exclusive) = match after_key {
            Some(cursor) if cursor >= key_prefix => (format!("{namespace}{cursor}"), true),
            _ => (format!("{namespace}{key_prefix}"), false),
        };
        let lower = if exclusive {
            Bound::Excluded(start.as_str())
        } else {
            Bound::Included(start.as_str())
        };
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        let mut records = Vec::with_capacity(limit.min(1024));
        for entry in table
            .range::<&str>((lower, Bound::Unbounded))
            .map_err(adapter_error)?
            .take(limit)
        {
            let (stored_key, value) = entry.map_err(adapter_error)?;
            let Some(key) = stored_key.value().strip_prefix(&namespace) else {
                break;
            };
            if !key.starts_with(key_prefix) {
                break;
            }
            records.push((
                key.to_owned(),
                serde_json::from_slice(value.value()).map_err(adapter_error)?,
            ));
        }
        Ok(records)
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        self.apply_all(std::slice::from_ref(&batch))
    }

    fn apply_all(&self, batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        if batches.is_empty() {
            return Ok(());
        }
        let encoded = batches
            .iter()
            .map(encode_projection_batch)
            .collect::<Result<Vec<_>, StoreError>>()?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            for batch in &encoded {
                let actual = positions
                    .get(batch.projection.as_str())
                    .map_err(adapter_error)?
                    .map_or(0, |position| position.value());
                if actual != batch.expected_position {
                    return Err(StoreError::Conflict {
                        stream_id: format!("projection:{}", batch.projection),
                        expected: batch.expected_position,
                        actual,
                    });
                }
                for (key, value) in &batch.mutations {
                    if let Some(value) = value {
                        records
                            .insert(key.as_str(), value.as_slice())
                            .map_err(adapter_error)?;
                    } else {
                        records.remove(key.as_str()).map_err(adapter_error)?;
                    }
                }
                positions
                    .insert(batch.projection.as_str(), batch.through_sequence)
                    .map_err(adapter_error)?;
            }
        }
        write.commit().map_err(adapter_error)
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        let namespace = projection_prefix(projection)?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            let mut keys = Vec::new();
            for entry in records.range(namespace.as_str()..).map_err(adapter_error)? {
                let (key, _) = entry.map_err(adapter_error)?;
                if !key.value().starts_with(&namespace) {
                    break;
                }
                keys.push(key.value().to_owned());
            }
            for key in keys {
                records.remove(key.as_str()).map_err(adapter_error)?;
            }
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            positions.remove(projection).map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)
    }
}
