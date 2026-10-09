use super::*;

struct PendingProjection {
    expected_position: u64,
    through_sequence: u64,
    records: BTreeMap<String, Option<Value>>,
}

/// Read-through state bounded by the mutations produced in one worker round.
pub(super) struct ProjectionOverlay<'a> {
    store: &'a dyn ProjectionStore,
    pending: BTreeMap<String, PendingProjection>,
}

impl<'a> ProjectionOverlay<'a> {
    pub(super) fn new(store: &'a dyn ProjectionStore) -> Self {
        Self {
            store,
            pending: BTreeMap::new(),
        }
    }

    pub(super) fn stage(
        &mut self,
        projection: &str,
        expected_position: u64,
        through_sequence: u64,
        mutations: Vec<ProjectionMutation>,
    ) {
        let pending = self
            .pending
            .entry(projection.into())
            .or_insert_with(|| PendingProjection {
                expected_position,
                through_sequence,
                records: BTreeMap::new(),
            });
        pending.through_sequence = through_sequence;
        for mutation in mutations {
            match mutation {
                ProjectionMutation::Upsert { key, value } => {
                    pending.records.insert(key, Some(value));
                }
                ProjectionMutation::Delete { key } => {
                    pending.records.insert(key, None);
                }
            }
        }
    }

    pub(super) fn take_batch(&mut self, projection: &str) -> Option<ProjectionBatch> {
        self.pending
            .remove(projection)
            .map(|pending| ProjectionBatch {
                projection: projection.into(),
                expected_position: pending.expected_position,
                through_sequence: pending.through_sequence,
                mutations: pending
                    .records
                    .into_iter()
                    .map(|(key, value)| match value {
                        Some(value) => ProjectionMutation::Upsert { key, value },
                        None => ProjectionMutation::Delete { key },
                    })
                    .collect(),
            })
    }
}

impl ProjectionStore for ProjectionOverlay<'_> {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        self.pending.get(projection).map_or_else(
            || self.store.position(projection),
            |pending| Ok(pending.through_sequence),
        )
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        match self
            .pending
            .get(projection)
            .and_then(|pending| pending.records.get(key))
        {
            Some(value) => Ok(value.clone()),
            None => self.store.get(projection, key),
        }
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
        let Some(pending) = self.pending.get(projection) else {
            return self
                .store
                .list_after(projection, key_prefix, after_key, limit);
        };
        if limit == 0 {
            return self.store.list_after(projection, key_prefix, after_key, 0);
        }
        let in_page =
            |key: &str| key.starts_with(key_prefix) && after_key.is_none_or(|cursor| key > cursor);
        // Fetch enough durable candidates to refill the page after staged deletes.
        // Unrelated keys and projections never enlarge this read.
        let changed = pending.records.keys().filter(|key| in_page(key)).count();
        let mut records = self
            .store
            .list_after(
                projection,
                key_prefix,
                after_key,
                limit.saturating_add(changed),
            )?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        for (key, value) in &pending.records {
            if !in_page(key) {
                continue;
            }
            if let Some(value) = value {
                records.insert(key.clone(), value.clone());
            } else {
                records.remove(key);
            }
        }
        Ok(records.into_iter().take(limit).collect())
    }

    fn apply(&self, _batch: ProjectionBatch) -> Result<(), StoreError> {
        Err(StoreError::Adapter(
            "projection reducers must return mutations instead of writing to the store".into(),
        ))
    }

    fn apply_all(&self, _batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        Err(StoreError::Adapter(
            "projection reducers must return mutations instead of writing to the store".into(),
        ))
    }

    fn reset(&self, _projection: &str) -> Result<(), StoreError> {
        Err(StoreError::Adapter(
            "projection reducers cannot reset the store".into(),
        ))
    }
}
