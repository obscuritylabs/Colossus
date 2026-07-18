use super::*;

impl Runtime {
    pub(super) async fn execute_memory_operation(
        &self,
        operation: MemoryOperation,
    ) -> Result<Value, RuntimeError> {
        let action = operation.action();
        let resource = operation.resource();
        let session_id = match &operation {
            MemoryOperation::Create {
                scope: MemoryScope::Session(id),
                ..
            } => Some(id.clone()),
            MemoryOperation::Archive { id }
            | MemoryOperation::Update { id, .. }
            | MemoryOperation::Supersede { id, .. }
            | MemoryOperation::Read { id } => {
                self.memory_executor
                    .service
                    .get(id)?
                    .and_then(|record| match record.scope {
                        MemoryScope::Session(id) => Some(id),
                        _ => None,
                    })
            }
            MemoryOperation::Search { session_id, .. } => session_id.clone(),
            _ => None,
        };
        let mut request = effect_request(
            terminal_actor(),
            action,
            resource,
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![action.into()];
        request.context.session_id = session_id;
        let result = self
            .gateway
            .execute(request, self.memory_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Create one canonical memory through the universal permission boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_memory(
        &self,
        scope: MemoryScope,
        kind: &str,
        confidence: f32,
        text: &str,
        rationale: &str,
        expires_at: Option<String>,
    ) -> Result<MemoryRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::Create {
                scope,
                kind: kind.into(),
                confidence,
                text: text.into(),
                rationale: rationale.into(),
                expires_at,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Update mutable fields on one active canonical memory.
    pub async fn update_memory(
        &self,
        id: &str,
        text: Option<&str>,
        rationale: Option<&str>,
        confidence: Option<f32>,
    ) -> Result<MemoryRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::Update {
                id: id.into(),
                text: text.map(str::to_owned),
                rationale: rationale.map(str::to_owned),
                confidence,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Archive one canonical memory through the permission boundary.
    pub async fn archive_memory(&self, id: &str) -> Result<MemoryRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::Archive { id: id.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Atomically supersede a canonical memory through the permission boundary.
    pub async fn supersede_memory(
        &self,
        id: &str,
        text: &str,
        rationale: &str,
    ) -> Result<(MemoryRecord, MemoryRecord), RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::Supersede {
                id: id.into(),
                text: text.into(),
                rationale: rationale.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Read one canonical memory through two-phase policy release.
    pub async fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::Read { id: id.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List bounded canonical memories through two-phase policy release.
    pub async fn list_memories(
        &self,
        status: Option<MemoryStatus>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::List {
                status,
                limit,
                session_id: None,
                repository_id: None,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Search candidate ids and re-filter canonical scoped records before release.
    pub async fn search_memories(
        &self,
        query: &str,
        session_id: Option<&str>,
        repository_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, RuntimeError> {
        serde_json::from_value(
            self.execute_memory_operation(MemoryOperation::Search {
                query: query.into(),
                session_id: session_id.map(str::to_owned),
                repository_id: repository_id.map(str::to_owned),
                limit,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Return policy-authorized index readiness and lag.
    pub async fn memory_index_status(&self) -> Result<Value, RuntimeError> {
        self.execute_memory_operation(MemoryOperation::IndexStatus)
            .await
    }

    /// Retry queued index work through the permission boundary.
    pub async fn sync_memory_index(&self) -> Result<Value, RuntimeError> {
        self.execute_memory_operation(MemoryOperation::IndexSync)
            .await
    }

    /// Rebuild the disposable memory index from canonical active records.
    pub async fn rebuild_memory_index(&self) -> Result<Value, RuntimeError> {
        self.execute_memory_operation(MemoryOperation::IndexRebuild)
            .await
    }
}
