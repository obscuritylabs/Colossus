use super::*;

pub(super) struct MemoryEffectExecutor {
    pub(super) service: Arc<MemoryService>,
    pub(super) repository_id: String,
}

impl MemoryEffectExecutor {
    fn model_controlled(request: &EffectRequest) -> bool {
        matches!(
            request.actor.actor_type,
            ActorType::Model | ActorType::Workflow | ActorType::Subagent
        )
    }

    fn scope_allowed(&self, scope: &MemoryScope, request: &EffectRequest) -> bool {
        match scope {
            MemoryScope::Global => true,
            MemoryScope::Repository(id) => id == &self.repository_id,
            MemoryScope::Session(id) => request.context.session_id.as_ref() == Some(id),
        }
    }

    fn validate_access(
        &self,
        request: &EffectRequest,
        operation: &MemoryOperation,
    ) -> Result<(), ExecutionError> {
        if !Self::model_controlled(request) {
            return Ok(());
        }
        match operation {
            MemoryOperation::Create { scope, .. } => {
                if !self.scope_allowed(scope, request) {
                    return Err(ExecutionError::Failed(
                        "memory tool cannot create outside its current scope".into(),
                    ));
                }
            }
            MemoryOperation::Update { id, .. }
            | MemoryOperation::Archive { id }
            | MemoryOperation::Supersede { id, .. }
            | MemoryOperation::Read { id } => {
                let record = self
                    .service
                    .get(id)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?
                    .ok_or_else(|| ExecutionError::Failed(format!("memory {id} was not found")))?;
                if !self.scope_allowed(&record.scope, request) {
                    return Err(ExecutionError::Failed(
                        "memory tool cannot access another scope".into(),
                    ));
                }
            }
            MemoryOperation::List {
                session_id,
                repository_id,
                ..
            }
            | MemoryOperation::Search {
                session_id,
                repository_id,
                ..
            } => {
                if session_id.as_ref() != request.context.session_id.as_ref()
                    || repository_id.as_deref() != Some(self.repository_id.as_str())
                {
                    return Err(ExecutionError::Failed(
                        "memory query scope does not match the current context".into(),
                    ));
                }
            }
            MemoryOperation::IndexStatus
            | MemoryOperation::IndexSync
            | MemoryOperation::IndexRebuild => {
                return Err(ExecutionError::Failed(
                    "model-controlled actors cannot administer the memory index".into(),
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EffectExecutor for MemoryEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: MemoryOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() {
            return Err(ExecutionError::Failed(
                "memory operation action does not match its validated content".into(),
            ));
        }
        self.validate_access(request, &operation)?;
        let actor = request.actor.clone();
        let value = match operation {
            MemoryOperation::Create {
                scope,
                kind,
                confidence,
                text,
                rationale,
                expires_at,
            } => work_result(
                self.service
                    .create(
                        scope, &kind, confidence, &text, &rationale, expires_at, actor,
                    )
                    .await,
            ),
            MemoryOperation::Update {
                id,
                text,
                rationale,
                confidence,
            } => work_result(
                self.service
                    .update(
                        &id,
                        text.as_deref(),
                        rationale.as_deref(),
                        confidence,
                        actor,
                    )
                    .await,
            ),
            MemoryOperation::Archive { id } => work_result(self.service.archive(&id, actor).await),
            MemoryOperation::Supersede {
                id,
                text,
                rationale,
            } => work_result(self.service.supersede(&id, &text, &rationale, actor).await),
            MemoryOperation::Read { id } => work_result(self.service.get(&id)),
            MemoryOperation::List {
                status,
                limit,
                session_id: _,
                repository_id: _,
            } => {
                let fetch_limit = if Self::model_controlled(request) {
                    1_000
                } else {
                    limit
                };
                let mut records = self
                    .service
                    .list(status, fetch_limit)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?;
                if Self::model_controlled(request) {
                    records.retain(|record| self.scope_allowed(&record.scope, request));
                    records.truncate(limit);
                }
                work_result(Ok::<_, StoreError>(records))
            }
            MemoryOperation::Search {
                query,
                session_id,
                repository_id,
                limit,
            } => work_result(
                self.service
                    .search(
                        &query,
                        session_id.as_deref(),
                        repository_id.as_deref(),
                        limit,
                    )
                    .await,
            ),
            MemoryOperation::IndexStatus => {
                let _ = self.service.sync_index().await;
                work_result(self.service.index_status().await)
            }
            MemoryOperation::IndexSync => {
                let result = match self.service.sync_index().await {
                    Ok(_) => self.service.index_status().await,
                    Err(error) => Err(error),
                };
                work_result(result)
            }
            MemoryOperation::IndexRebuild => work_result(self.service.rebuild_index().await),
        }?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

pub(super) struct GatewayMemoryRetriever {
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) executor: Arc<MemoryEffectExecutor>,
    pub(super) limit: usize,
    pub(super) repository_id: String,
}

#[async_trait]
impl MemoryRetriever for GatewayMemoryRetriever {
    async fn relevant(
        &self,
        query: &str,
        session_id: &str,
        context: ExecutionContext,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let operation = MemoryOperation::Search {
            query: query.into(),
            session_id: Some(session_id.into()),
            repository_id: Some(self.repository_id.clone()),
            limit: limit.min(self.limit),
        };
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::Model,
                id: "context-memory-retriever".into(),
            },
            operation.action(),
            format!("session:{session_id}"),
            serde_json::to_value(&operation).map_err(|error| {
                StoreError::Adapter(format!("memory request encoding failed: {error}"))
            })?,
        );
        request.capabilities = vec![operation.action().into()];
        request.context = context;
        match self.gateway.execute(request, self.executor.as_ref()).await {
            Ok(result) => serde_json::from_slice(&result.bytes).map_err(|error| {
                StoreError::Verification(format!("released memory result is invalid: {error}"))
            }),
            Err(GatewayError::Denied(_) | GatewayError::Approval(_)) => Ok(Vec::new()),
            Err(error) => Err(StoreError::Adapter(format!(
                "memory retrieval failed: {error}"
            ))),
        }
    }
}
