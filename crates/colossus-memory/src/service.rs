use super::*;

/// Canonical memory lifecycle, disposable-index synchronization, and re-filtered retrieval.
pub struct MemoryService {
    journal: Arc<dyn EventJournal>,
    repository: Arc<dyn MemoryRepository>,
    queue: Arc<dyn ExternalWorkQueue>,
    indexes: Vec<MemoryIndexRegistration>,
    sessions: Arc<dyn SessionRepository>,
}

/// One independently checkpointed disposable memory-index consumer.
pub struct MemoryIndexRegistration {
    consumer: String,
    index: Arc<dyn MemoryIndex>,
    last_error: Mutex<Option<String>>,
}

impl MemoryIndexRegistration {
    /// Register an adapter under a stable versioned external-work consumer name.
    pub fn new(
        consumer: impl Into<String>,
        index: Arc<dyn MemoryIndex>,
    ) -> Result<Self, StoreError> {
        let consumer = consumer.into();
        if consumer.is_empty()
            || consumer.len() > 128
            || !consumer
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(StoreError::Adapter(
                "memory index consumer must be 1-128 ASCII letters, digits, '.', '_', or '-'"
                    .into(),
            ));
        }
        Ok(Self {
            consumer,
            index,
            last_error: Mutex::new(None),
        })
    }
}

impl MemoryService {
    /// Compose memory behavior from replaceable repository and index ports.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        repository: Arc<dyn MemoryRepository>,
        queue: Arc<dyn ExternalWorkQueue>,
        index: Arc<dyn MemoryIndex>,
        sessions: Arc<dyn SessionRepository>,
    ) -> Result<Self, StoreError> {
        Self::with_indexes(
            journal,
            repository,
            queue,
            vec![MemoryIndexRegistration::new("memory.primary-v1", index)?],
            sessions,
        )
    }

    /// Compose memory behavior with multiple independently durable indexes.
    pub fn with_indexes(
        journal: Arc<dyn EventJournal>,
        repository: Arc<dyn MemoryRepository>,
        queue: Arc<dyn ExternalWorkQueue>,
        indexes: Vec<MemoryIndexRegistration>,
        sessions: Arc<dyn SessionRepository>,
    ) -> Result<Self, StoreError> {
        let mut consumers = indexes
            .iter()
            .map(|registration| registration.consumer.as_str())
            .collect::<Vec<_>>();
        consumers.sort_unstable();
        if consumers.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(StoreError::Adapter(
                "memory index consumer names must be unique".into(),
            ));
        }
        Ok(Self {
            journal,
            repository,
            queue,
            indexes,
            sessions,
        })
    }

    /// Create a canonical active memory and enqueue its event for indexing.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        scope: MemoryScope,
        kind: &str,
        confidence: f32,
        text: &str,
        rationale: &str,
        expires_at: Option<String>,
        actor: Actor,
    ) -> Result<MemoryRecord, StoreError> {
        self.validate_scope(&scope)?;
        let timestamp = now()?;
        let record = self.repository.create(
            MemoryRecord {
                id: format!("mem_{}", Uuid::now_v7()),
                scope,
                kind: kind.trim().into(),
                confidence,
                source: source_for_actor(&actor).into(),
                status: MemoryStatus::Active,
                text: text.trim().into(),
                rationale: rationale.into(),
                created_at: timestamp.clone(),
                updated_at: timestamp,
                expires_at,
                superseded_by: None,
            },
            actor,
        )?;
        self.sync_best_effort().await;
        Ok(record)
    }

    /// Append an updated active memory state while preserving identity and scope.
    pub async fn update(
        &self,
        id: &str,
        text: Option<&str>,
        rationale: Option<&str>,
        confidence: Option<f32>,
        actor: Actor,
    ) -> Result<MemoryRecord, StoreError> {
        let mut record = self
            .repository
            .get_memory(id)?
            .ok_or_else(|| StoreError::NotFound(format!("memory {id}")))?;
        if record.status != MemoryStatus::Active {
            return Err(StoreError::Adapter(
                "only an active memory can be updated".into(),
            ));
        }
        if text.is_none() && rationale.is_none() && confidence.is_none() {
            return Err(StoreError::Adapter(
                "memory update requires text, rationale, or confidence".into(),
            ));
        }
        if let Some(text) = text {
            record.text = text.trim().into();
        }
        if let Some(rationale) = rationale {
            record.rationale = rationale.into();
        }
        if let Some(confidence) = confidence {
            record.confidence = confidence;
        }
        record.updated_at = now()?;
        let record = self.repository.update(record, actor)?;
        self.sync_best_effort().await;
        Ok(record)
    }

    /// Archive one active canonical memory without deleting history.
    pub async fn archive(&self, id: &str, actor: Actor) -> Result<MemoryRecord, StoreError> {
        let record = self.repository.archive(id, actor)?;
        self.sync_best_effort().await;
        Ok(record)
    }

    /// Atomically supersede a canonical memory and index the linked replacement.
    pub async fn supersede(
        &self,
        id: &str,
        text: &str,
        rationale: &str,
        actor: Actor,
    ) -> Result<(MemoryRecord, MemoryRecord), StoreError> {
        let current = self
            .repository
            .get_memory(id)?
            .ok_or_else(|| StoreError::NotFound(format!("memory {id}")))?;
        let timestamp = now()?;
        let replacement = MemoryRecord {
            id: format!("mem_{}", Uuid::now_v7()),
            scope: current.scope.clone(),
            kind: current.kind.clone(),
            confidence: current.confidence,
            source: source_for_actor(&actor).into(),
            status: MemoryStatus::Active,
            text: text.trim().into(),
            rationale: rationale.into(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            expires_at: current.expires_at.clone(),
            superseded_by: None,
        };
        let records = self.repository.supersede(id, replacement, actor)?;
        self.sync_best_effort().await;
        Ok(records)
    }

    /// Reconstruct one canonical record.
    pub fn get(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        self.repository.get_memory(id)
    }

    /// List bounded canonical records before caller-specific policy release.
    pub fn list(
        &self,
        status: Option<MemoryStatus>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let mut records = self.repository.list_memories(status, MAX_LIST)?;
        if status == Some(MemoryStatus::Active) {
            let now = OffsetDateTime::now_utc();
            records.retain(|record| !expired(record, now));
        }
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    /// Search disposable candidates, then reload and re-filter canonical records.
    pub async fn search(
        &self,
        query: &str,
        session_id: Option<&str>,
        repository_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let limit = limit.clamp(1, 100);
        if query.trim().is_empty() {
            return self.fallback(query, session_id, repository_id, limit);
        }
        let mut candidates = BTreeMap::<String, f32>::new();
        for registration in &self.indexes {
            if let Err(error) = self.sync_with_retry(registration).await {
                self.record_index_error(registration, &error);
                continue;
            }
            match registration
                .index
                .search(query, limit.saturating_mul(4))
                .await
            {
                Ok(found) => {
                    for (id, score) in found {
                        candidates
                            .entry(id)
                            .and_modify(|current| *current = current.max(score))
                            .or_insert(score);
                    }
                }
                Err(error) => self.record_index_error(registration, &error),
            }
        }
        if candidates.is_empty() {
            return self.fallback(query, session_id, repository_id, limit);
        }
        let mut candidates = candidates.into_iter().collect::<Vec<_>>();
        candidates.sort_by(|(left_id, left_score), (right_id, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left_id.cmp(right_id))
        });
        let now = OffsetDateTime::now_utc();
        let mut records = Vec::new();
        for (id, _score) in candidates {
            let Some(record) = self.repository.get_memory(&id)? else {
                continue;
            };
            if memory_visible(&record, session_id, repository_id, now) {
                records.push(record);
                if records.len() >= limit {
                    break;
                }
            }
        }
        Ok(records)
    }

    /// Apply queued canonical memory events to the disposable index in global order.
    pub async fn sync_index(&self) -> Result<u64, StoreError> {
        let (head, _) = self.journal.head()?;
        let mut first_error = None;
        for registration in &self.indexes {
            if let Err(error) = self.sync_with_retry(registration).await {
                self.record_index_error(registration, &error);
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(head), Err)
    }

    async fn sync_with_retry(
        &self,
        registration: &MemoryIndexRegistration,
    ) -> Result<u64, StoreError> {
        self.enforce_retry_gate(registration)?;
        match self.sync_one(registration).await {
            Ok(position) => {
                self.queue.clear_failure(&registration.consumer)?;
                Ok(position)
            }
            Err(error) => {
                let pending = self
                    .queue
                    .pending(&registration.consumer, 1)
                    .ok()
                    .and_then(|items| items.into_iter().next());
                let (retryable, error_code) = retry_classification(&error);
                let diagnostic = bounded_retry_error(&error);
                let failed_at = now()?;
                if let Err(telemetry_error) = self.queue.record_failure(
                    &registration.consumer,
                    pending.as_ref(),
                    &failed_at,
                    retryable,
                    error_code,
                    &diagnostic,
                ) {
                    return Err(StoreError::Adapter(format!(
                        "{diagnostic}; durable retry telemetry failed: {telemetry_error}"
                    )));
                }
                Err(error)
            }
        }
    }

    fn enforce_retry_gate(&self, registration: &MemoryIndexRegistration) -> Result<(), StoreError> {
        let Some(state) = self.queue.retry_state(&registration.consumer)? else {
            return Ok(());
        };
        if !state.retryable {
            return Err(StoreError::OutcomeUnknown(format!(
                "memory index {} is blocked at sequence {} after {} attempt(s); operator-authorized rebuild required",
                registration.consumer, state.global_sequence, state.attempts
            )));
        }
        if let Some(next_retry_at) = state.next_retry_at.as_deref() {
            let next_retry = OffsetDateTime::parse(next_retry_at, &Rfc3339).map_err(|_| {
                StoreError::Verification(format!(
                    "memory index {} has an invalid retry timestamp",
                    registration.consumer
                ))
            })?;
            if OffsetDateTime::now_utc() < next_retry {
                return Err(StoreError::Adapter(format!(
                    "memory index {} retry is deferred until {next_retry_at}",
                    registration.consumer
                )));
            }
        }
        Ok(())
    }

    async fn sync_one(&self, registration: &MemoryIndexRegistration) -> Result<u64, StoreError> {
        let mut position = self.queue.position(&registration.consumer)?;
        let adapter_position = registration.index.position()?;
        if adapter_position < position {
            return Err(StoreError::Verification(format!(
                "memory index {} position {adapter_position} is behind acknowledged queue position {position}; rebuild required",
                registration.consumer
            )));
        }
        loop {
            let work = self.queue.pending(&registration.consumer, 256)?;
            if work.is_empty() {
                break;
            }
            for item in &work {
                let event = self
                    .journal
                    .read_global(item.global_sequence, 1)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "memory index queue sequence {} has no journal event",
                            item.global_sequence
                        ))
                    })?;
                if event.global_sequence != item.global_sequence || event.event_id != item.event_id
                {
                    return Err(StoreError::Verification(format!(
                        "memory index queue sequence {} does not match its journal event",
                        item.global_sequence
                    )));
                }
                if let Some(id) = event.stream_id.strip_prefix("memory:") {
                    match event.event_type.as_str() {
                        "memory.created.v1" | "memory.updated.v1" => {
                            let payload = self.journal.decrypt_payload(&event)?;
                            let record: MemoryRecord = serde_json::from_value(
                                payload.get("record").cloned().ok_or_else(|| {
                                    StoreError::Verification(
                                        "memory index event has no canonical record".into(),
                                    )
                                })?,
                            )
                            .map_err(adapter)?;
                            registration
                                .index
                                .upsert(
                                    &event.event_id,
                                    id,
                                    &record.text,
                                    &memory_metadata(&record),
                                    None,
                                )
                                .await?;
                        }
                        "memory.archived.v1" | "memory.superseded.v1" => {
                            registration.index.remove(&event.event_id, id).await?;
                        }
                        _ => {}
                    }
                }
            }
            let through_sequence = work
                .last()
                .map(|item| item.global_sequence)
                .ok_or_else(|| StoreError::Adapter("memory index work batch is empty".into()))?;
            // The adapter checkpoint is written before the queue acknowledgment.
            // A crash between them causes a safe idempotent batch replay; the inverse
            // ordering could lose external work permanently.
            registration.index.set_position(through_sequence).await?;
            position = self
                .queue
                .acknowledge_batch(&registration.consumer, position, &work)?;
            if work.len() < 256 {
                break;
            }
        }
        *registration.last_error.lock().map_err(adapter)? = None;
        Ok(position)
    }

    /// Delete and reconstruct the disposable index from canonical active records.
    pub async fn rebuild_index(&self) -> Result<Value, StoreError> {
        let records = self.repository.list_active(MAX_LIST)?;
        let values = records
            .iter()
            .map(|record| {
                (
                    record.id.clone(),
                    record.text.clone(),
                    memory_metadata(record),
                )
            })
            .collect::<Vec<_>>();
        let mut first_error = None;
        for registration in &self.indexes {
            // Reset acknowledgment first. If a destructive external rebuild fails,
            // the entire journal remains pending for explicit recovery.
            if let Err(error) = self.queue.reset(&registration.consumer) {
                self.record_index_error(registration, &error);
                first_error.get_or_insert(error);
                continue;
            }
            if let Err(error) = self.queue.clear_failure(&registration.consumer) {
                self.record_index_error(registration, &error);
                first_error.get_or_insert(error);
                continue;
            }
            if let Err(error) = registration.index.set_position(0).await {
                self.record_index_error(registration, &error);
                first_error.get_or_insert(error);
                continue;
            }
            if let Err(error) = registration.index.rebuild(&values).await {
                self.record_index_error(registration, &error);
                first_error.get_or_insert(error);
                continue;
            }
            if let Err(error) = self.sync_one(registration).await {
                self.record_index_error(registration, &error);
                first_error.get_or_insert(error);
            } else if let Err(error) = self.queue.clear_failure(&registration.consumer) {
                self.record_index_error(registration, &error);
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        self.index_status().await
    }

    /// Return bounded index readiness, lag, and adapter details.
    pub async fn index_status(&self) -> Result<Value, StoreError> {
        let (head, _) = self.journal.head()?;
        let mut consumers = Vec::with_capacity(self.indexes.len());
        let mut minimum_position = head;
        let mut maximum_lag = 0_u64;
        let mut all_ready = !self.indexes.is_empty();
        let mut first_error = None;
        let mut first_adapter = None;
        for registration in &self.indexes {
            let position = self.queue.position(&registration.consumer)?;
            let retry = self.queue.retry_state(&registration.consumer)?;
            let adapter_status = match registration.index.status().await {
                Ok(status) => status,
                Err(error) => {
                    self.record_index_error(registration, &error);
                    json!({
                        "ready": false,
                        "kind": "unavailable",
                        "status_error": error.to_string(),
                    })
                }
            };
            let adapter_ready = adapter_status
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let error = registration
                .last_error
                .lock()
                .map_err(adapter)?
                .clone()
                .or_else(|| retry.as_ref().map(|state| state.error.clone()));
            let lag = head.saturating_sub(position);
            let ready = adapter_ready && error.is_none() && retry.is_none() && position == head;
            minimum_position = minimum_position.min(position);
            maximum_lag = maximum_lag.max(lag);
            all_ready &= ready;
            if first_error.is_none() {
                first_error.clone_from(&error);
            }
            if first_adapter.is_none() {
                first_adapter = Some(adapter_status.clone());
            }
            consumers.push(json!({
                "consumer": registration.consumer,
                "ready": ready,
                "position": position,
                "journal_head": head,
                "lag": lag,
                "last_error": error,
                "retry": retry,
                "adapter": adapter_status,
            }));
        }
        Ok(json!({
            "ready": all_ready,
            "position": minimum_position,
            "journal_head": head,
            "lag": maximum_lag,
            "last_error": first_error,
            "adapter": first_adapter,
            "consumers": consumers,
        }))
    }

    fn validate_scope(&self, scope: &MemoryScope) -> Result<(), StoreError> {
        if let MemoryScope::Session(session_id) = scope {
            self.sessions
                .get_session(session_id)?
                .ok_or_else(|| StoreError::NotFound(format!("session {session_id}")))?;
        }
        Ok(())
    }

    async fn sync_best_effort(&self) {
        let _ = self.sync_index().await;
    }

    fn record_index_error(&self, registration: &MemoryIndexRegistration, error: &StoreError) {
        if let Ok(mut last_error) = registration.last_error.lock() {
            *last_error = Some(error.to_string());
        }
    }

    fn fallback(
        &self,
        query: &str,
        session_id: Option<&str>,
        repository_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let now = OffsetDateTime::now_utc();
        let terms = query
            .split(|character: char| !character.is_ascii_alphanumeric())
            .map(str::to_ascii_lowercase)
            .filter(|term| term.len() >= 2)
            .collect::<BTreeSet<_>>();
        let mut records = self
            .repository
            .list_active(MAX_LIST)?
            .into_iter()
            .filter(|record| memory_visible(record, session_id, repository_id, now))
            .filter_map(|record| {
                let searchable = format!("{} {} {}", record.kind, record.text, record.rationale)
                    .to_ascii_lowercase();
                let score = terms
                    .iter()
                    .filter(|term| searchable.contains(*term))
                    .count();
                (terms.is_empty() || score > 0).then_some((record, score))
            })
            .collect::<Vec<_>>();
        records.sort_by(|(left, left_score), (right, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| scope_rank(&left.scope).cmp(&scope_rank(&right.scope)))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        records.truncate(limit);
        Ok(records.into_iter().map(|(record, _)| record).collect())
    }
}

fn retry_classification(error: &StoreError) -> (bool, &'static str) {
    match error {
        StoreError::Conflict { .. } => (true, "external_work.conflict"),
        StoreError::KeyUnavailable(_) => (true, "external_work.key_unavailable"),
        StoreError::Adapter(_) => (true, "external_work.adapter"),
        StoreError::NotFound(_) => (false, "external_work.not_found"),
        StoreError::Verification(_) => (false, "external_work.verification"),
        StoreError::OutcomeUnknown(_) => (false, "external_work.outcome_unknown"),
        StoreError::RecoveryMode => (false, "external_work.recovery_mode"),
    }
}

fn bounded_retry_error(error: &StoreError) -> String {
    const MAX_BYTES: usize = 2_048;
    let source = error.to_string();
    let mut bounded = String::with_capacity(source.len().min(MAX_BYTES));
    for character in source.chars() {
        if bounded.len().saturating_add(character.len_utf8()) > MAX_BYTES {
            break;
        }
        bounded.push(character);
    }
    bounded
}

fn source_for_actor(actor: &Actor) -> &'static str {
    if actor.actor_type == ActorType::User {
        "user"
    } else {
        "agent"
    }
}

fn memory_metadata(record: &MemoryRecord) -> Value {
    json!({
        "scope": record.scope,
        "kind": record.kind,
        "confidence": record.confidence,
        "source": record.source,
        "status": record.status,
        "expires_at": record.expires_at,
    })
}

fn memory_visible(
    record: &MemoryRecord,
    session_id: Option<&str>,
    repository_id: Option<&str>,
    now: OffsetDateTime,
) -> bool {
    if record.status != MemoryStatus::Active || expired(record, now) {
        return false;
    }
    match &record.scope {
        MemoryScope::Global => true,
        MemoryScope::Repository(id) => repository_id == Some(id.as_str()),
        MemoryScope::Session(id) => session_id == Some(id.as_str()),
    }
}

fn scope_rank(scope: &MemoryScope) -> u8 {
    match scope {
        MemoryScope::Repository(_) => 0,
        MemoryScope::Session(_) => 1,
        MemoryScope::Global => 2,
    }
}
