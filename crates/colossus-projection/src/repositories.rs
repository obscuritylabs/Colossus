use super::*;

/// Session repository served from a rebuildable projection.
pub struct ProjectedSessionRepository {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedSessionRepository {
    /// Bind the repository to a projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }
}

impl AggregateRepository for ProjectedSessionRepository {
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        self.store.get("sessions-v1", id)
    }

    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        Ok(self
            .store
            .list("sessions-v1", "", limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}

/// Work repository served from task/decision/plan/goal projections.
pub struct ProjectedWorkRepository {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedWorkRepository {
    /// Bind the repository to a projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }
}

impl AggregateRepository for ProjectedWorkRepository {
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        if id.contains(':') {
            return self.store.get("work-v1", id);
        }
        for prefix in ["task:", "decision:", "plan:", "goal:"] {
            if let Some(record) = self.store.get("work-v1", &format!("{prefix}{id}"))? {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        Ok(self
            .store
            .list("work-v1", "", limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}

/// Read-only projected view used after canonical memory authorization.
pub struct ProjectedMemoryReader {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedMemoryReader {
    /// Bind the reader to a projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }

    /// Load one canonical memory snapshot.
    pub fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        self.store.get("memory-v1", id)
    }

    /// List bounded canonical memory snapshots.
    pub fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        Ok(self
            .store
            .list("memory-v1", "", limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}
