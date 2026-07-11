//! Canonical event-sourced memories and the disposable Tantivy lexical index.

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, MemoryRecord, MemoryScope,
    MemoryStatus, NewEvent,
};
use colossus_ports::{EventJournal, MemoryIndex, MemoryRepository, SessionRepository, StoreError};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
    collector::{Count, TopDocs},
    doc,
    query::{QueryParser, TermQuery},
    schema::{Field, IndexRecordOption, STORED, STRING, Schema, TEXT, Value as _},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const MAX_MEMORY_TEXT_BYTES: usize = 64 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_LIST: usize = 1_000;
const POSITION_DOCUMENT_ID: &str = "__position__";

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

fn validate_record(record: &MemoryRecord) -> Result<(), StoreError> {
    if record.id.is_empty()
        || record.kind.is_empty()
        || !matches!(record.source.as_str(), "user" | "agent")
        || record.text.is_empty()
        || record.text.len() > MAX_MEMORY_TEXT_BYTES
        || record.rationale.len() > MAX_MEMORY_TEXT_BYTES
        || !record.confidence.is_finite()
        || !(0.0..=1.0).contains(&record.confidence)
        || record.status != MemoryStatus::Active
        || record.superseded_by.is_some()
        || matches!(&record.scope, MemoryScope::Repository(id) | MemoryScope::Session(id) if id.trim().is_empty())
    {
        return Err(StoreError::Adapter(
            "memory id/kind/source/text/confidence/status is invalid".into(),
        ));
    }
    if record
        .expires_at
        .as_deref()
        .is_some_and(|value| OffsetDateTime::parse(value, &Rfc3339).is_err())
    {
        return Err(StoreError::Adapter(
            "memory expiry must be UTC RFC3339 when supplied".into(),
        ));
    }
    let normalized = format!("{} {}", record.text, record.rationale).to_ascii_lowercase();
    if normalized.contains("private key-----")
        || normalized.contains("authorization: bearer ")
        || normalized.contains(" bearer ")
        || normalized.contains("password=")
        || normalized.contains("password:")
        || normalized.contains("api_key=")
        || normalized.contains("api-key:")
        || normalized.contains("apikey=")
        || normalized.contains("secret=")
        || normalized.contains("token=")
        || normalized.contains("github_pat_")
        || normalized.contains("ghp_")
        || normalized.contains(" sk-")
    {
        return Err(StoreError::Adapter(
            "memory text resembles prohibited secret material".into(),
        ));
    }
    Ok(())
}

/// Memory lifecycle repository backed only by immutable journal events.
pub struct EventSourcedMemoryRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedMemoryRepository {
    /// Bind canonical memory streams to the authoritative journal.
    /// Runtime mutation surfaces still place this adapter behind the effect gateway.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(id: &str) -> String {
        format!("memory:{id}")
    }

    fn event(
        id: &str,
        expected_stream_version: u64,
        event_type: &str,
        actor: Actor,
        payload: Value,
    ) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: Self::stream(id),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("memory:{id}"),
                ..ExecutionContext::default()
            },
            payload,
        }
    }
}

impl MemoryRepository for EventSourcedMemoryRepository {
    fn create(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError> {
        validate_record(&record)?;
        self.journal.append(Self::event(
            &record.id,
            0,
            "memory.created.v1",
            actor,
            json!({"record": &record}),
        ))?;
        Ok(record)
    }

    fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        let events = self.journal.read_stream(&Self::stream(id))?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        let payload = self.journal.decrypt_payload(first)?;
        let mut record: MemoryRecord = serde_json::from_value(
            payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("memory record is absent".into()))?,
        )
        .map_err(adapter)?;
        validate_record(&record).map_err(|error| {
            StoreError::Verification(format!("invalid canonical memory creation event: {error}"))
        })?;
        for event in events.iter().skip(1) {
            let payload = self.journal.decrypt_payload(event)?;
            match event.event_type.as_str() {
                "memory.archived.v1" => {
                    record.status = MemoryStatus::Archived;
                    record.updated_at = string(&payload, "updated_at")?;
                }
                "memory.superseded.v1" => {
                    record.status = MemoryStatus::Superseded;
                    record.updated_at = string(&payload, "updated_at")?;
                    record.superseded_by = Some(string(&payload, "replacement_id")?);
                }
                _ => {}
            }
        }
        Ok(Some(record))
    }

    fn list_active(&self, limit: usize) -> Result<Vec<MemoryRecord>, StoreError> {
        let mut records = self.list_memories(Some(MemoryStatus::Active), MAX_LIST)?;
        let current_time = OffsetDateTime::now_utc();
        records.retain(|record| !expired(record, current_time));
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn list_memories(
        &self,
        status: Option<MemoryStatus>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if event.event_type == "memory.created.v1"
                    && let Some(id) = event.stream_id.strip_prefix("memory:")
                {
                    ids.insert(id.to_owned());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < 1_024 {
                break;
            }
        }
        let mut records = ids
            .into_iter()
            .filter_map(|id| self.get_memory(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| status.is_none_or(|status| record.status == status));
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn archive(&self, id: &str, actor: Actor) -> Result<MemoryRecord, StoreError> {
        let record = self
            .get_memory(id)?
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        if record.status != MemoryStatus::Active {
            return Err(StoreError::Conflict {
                stream_id: Self::stream(id),
                expected: 1,
                actual: u64::try_from(self.journal.read_stream(&Self::stream(id))?.len())
                    .map_err(adapter)?,
            });
        }
        let events = self.journal.read_stream(&Self::stream(id))?;
        self.journal.append(Self::event(
            id,
            u64::try_from(events.len()).map_err(adapter)?,
            "memory.archived.v1",
            actor,
            json!({"updated_at": now()?}),
        ))?;
        self.get_memory(id)?
            .ok_or_else(|| StoreError::Verification("archived memory disappeared".into()))
    }

    fn supersede(
        &self,
        id: &str,
        replacement: MemoryRecord,
        actor: Actor,
    ) -> Result<(MemoryRecord, MemoryRecord), StoreError> {
        let current = self
            .get_memory(id)?
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        if current.status != MemoryStatus::Active || replacement.id == id {
            return Err(StoreError::Adapter(
                "only an active memory can be superseded by a different id".into(),
            ));
        }
        validate_record(&replacement)?;
        if self.get_memory(&replacement.id)?.is_some() {
            return Err(StoreError::Conflict {
                stream_id: Self::stream(&replacement.id),
                expected: 0,
                actual: 1,
            });
        }
        let old_events = self.journal.read_stream(&Self::stream(id))?;
        let timestamp = now()?;
        let replacement_id = replacement.id.clone();
        self.journal.append_batch(vec![
            Self::event(
                id,
                u64::try_from(old_events.len()).map_err(adapter)?,
                "memory.superseded.v1",
                actor.clone(),
                json!({"updated_at": timestamp, "replacement_id": replacement_id}),
            ),
            Self::event(
                &replacement.id,
                0,
                "memory.created.v1",
                actor,
                json!({"record": &replacement}),
            ),
        ])?;
        Ok((
            self.get_memory(id)?
                .ok_or_else(|| StoreError::Verification("superseded memory disappeared".into()))?,
            self.get_memory(&replacement.id)?
                .ok_or_else(|| StoreError::Verification("replacement memory disappeared".into()))?,
        ))
    }
}

fn string(value: &Value, field: &str) -> Result<String, StoreError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Verification(format!("memory field {field} is absent")))
}

fn expired(record: &MemoryRecord, now: OffsetDateTime) -> bool {
    record.expires_at.as_deref().is_some_and(|value| {
        OffsetDateTime::parse(value, &Rfc3339).is_ok_and(|expiry| expiry <= now)
    })
}

/// Offline disposable lexical memory projection.
pub struct TantivyMemoryIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter<TantivyDocument>>,
    id: Field,
    event_id: Field,
    text: Field,
    metadata: Field,
    active: Field,
}

/// Degraded index adapter preserving canonical-memory availability and visible lag.
pub struct UnavailableMemoryIndex {
    reason: String,
}

impl UnavailableMemoryIndex {
    /// Preserve a bounded adapter-open failure for readiness diagnostics.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl MemoryIndex for UnavailableMemoryIndex {
    fn position(&self) -> Result<u64, StoreError> {
        Err(StoreError::Adapter(self.reason.clone()))
    }

    async fn set_position(&self, _position: u64) -> Result<(), StoreError> {
        Err(StoreError::Adapter(self.reason.clone()))
    }

    async fn upsert(
        &self,
        _event_id: &str,
        _memory_id: &str,
        _text: &str,
        _metadata: &Value,
        _embedding: Option<&[f32]>,
    ) -> Result<(), StoreError> {
        Err(StoreError::Adapter(self.reason.clone()))
    }

    async fn remove(&self, _event_id: &str, _memory_id: &str) -> Result<(), StoreError> {
        Err(StoreError::Adapter(self.reason.clone()))
    }

    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<(String, f32)>, StoreError> {
        Err(StoreError::Adapter(self.reason.clone()))
    }

    async fn status(&self) -> Result<Value, StoreError> {
        Ok(json!({"ready": false, "kind": "unavailable", "reason": self.reason}))
    }

    async fn rebuild(&self, _records: &[(String, String, Value)]) -> Result<(), StoreError> {
        Err(StoreError::Adapter(self.reason.clone()))
    }
}

impl TantivyMemoryIndex {
    /// Open or create the offline lexical projection.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        std::fs::create_dir_all(path.as_ref()).map_err(adapter)?;
        let mut builder = Schema::builder();
        let id = builder.add_text_field("memory_id", STRING | STORED);
        let event_id = builder.add_text_field("event_id", STRING | STORED);
        let text = builder.add_text_field("text", TEXT);
        let metadata = builder.add_text_field("metadata", STORED);
        let active = builder.add_text_field("active", STRING);
        let schema = builder.build();
        let index = match Index::open_in_dir(path.as_ref()) {
            Ok(index) => index,
            Err(_) => Index::create_in_dir(path.as_ref(), schema).map_err(adapter)?,
        };
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(adapter)?;
        let writer = index.writer(50_000_000).map_err(adapter)?;
        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            id,
            event_id,
            text,
            metadata,
            active,
        })
    }

    fn processed(&self, event_id: &str) -> Result<bool, StoreError> {
        self.reader.reload().map_err(adapter)?;
        let searcher = self.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_text(self.event_id, event_id),
            IndexRecordOption::Basic,
        );
        Ok(!searcher
            .search(&query, &TopDocs::with_limit(1).order_by_score())
            .map_err(adapter)?
            .is_empty())
    }
}

#[async_trait]
impl MemoryIndex for TantivyMemoryIndex {
    fn position(&self) -> Result<u64, StoreError> {
        self.reader.reload().map_err(adapter)?;
        let searcher = self.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_text(self.id, POSITION_DOCUMENT_ID),
            IndexRecordOption::Basic,
        );
        let Some((_score, address)) = searcher
            .search(&query, &TopDocs::with_limit(1).order_by_score())
            .map_err(adapter)?
            .into_iter()
            .next()
        else {
            return Ok(0);
        };
        let document: TantivyDocument = searcher.doc(address).map_err(adapter)?;
        document
            .get_first(self.metadata)
            .and_then(|value| value.as_str())
            .ok_or_else(|| StoreError::Verification("memory index position is absent".into()))?
            .parse::<u64>()
            .map_err(adapter)
    }

    async fn set_position(&self, position: u64) -> Result<(), StoreError> {
        let mut writer = self.writer.lock().map_err(adapter)?;
        writer.delete_term(Term::from_field_text(self.id, POSITION_DOCUMENT_ID));
        writer
            .add_document(doc!(
                self.id => POSITION_DOCUMENT_ID,
                self.event_id => format!("position:{position}"),
                self.metadata => position.to_string(),
                self.active => "marker",
            ))
            .map_err(adapter)?;
        writer.commit().map_err(adapter)?;
        self.reader.reload().map_err(adapter)
    }

    async fn upsert(
        &self,
        event_id: &str,
        memory_id: &str,
        text: &str,
        metadata: &Value,
        _embedding: Option<&[f32]>,
    ) -> Result<(), StoreError> {
        if self.processed(event_id)? {
            return Ok(());
        }
        let metadata = serde_json::to_string(metadata).map_err(adapter)?;
        if text.len() > MAX_MEMORY_TEXT_BYTES || metadata.len() > MAX_METADATA_BYTES {
            return Err(StoreError::Adapter(
                "memory index text or metadata exceeds the bounded projection size".into(),
            ));
        }
        let mut writer = self.writer.lock().map_err(adapter)?;
        writer.delete_term(Term::from_field_text(self.id, memory_id));
        writer
            .add_document(doc!(
                self.id => memory_id.to_owned(),
                self.event_id => event_id.to_owned(),
                self.text => text.to_owned(),
                self.metadata => metadata,
                self.active => "true",
            ))
            .map_err(adapter)?;
        writer
            .add_document(doc!(
                self.id => format!("__event__:{event_id}"),
                self.event_id => event_id.to_owned(),
                self.metadata => "{}",
                self.active => "marker",
            ))
            .map_err(adapter)?;
        writer.commit().map_err(adapter)?;
        self.reader.reload().map_err(adapter)
    }

    async fn remove(&self, event_id: &str, memory_id: &str) -> Result<(), StoreError> {
        if self.processed(event_id)? {
            return Ok(());
        }
        let mut writer = self.writer.lock().map_err(adapter)?;
        writer.delete_term(Term::from_field_text(self.id, memory_id));
        writer
            .add_document(doc!(
                self.id => format!("__event__:{event_id}"),
                self.event_id => event_id.to_owned(),
                self.metadata => "{}",
                self.active => "marker",
            ))
            .map_err(adapter)?;
        writer.commit().map_err(adapter)?;
        self.reader.reload().map_err(adapter)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>, StoreError> {
        self.reader.reload().map_err(adapter)?;
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.text]);
        let (query, _errors) = parser.parse_query_lenient(query);
        let top = searcher
            .search(&query, &TopDocs::with_limit(limit).order_by_score())
            .map_err(adapter)?;
        top.into_iter()
            .map(|(score, address)| {
                let document: TantivyDocument = searcher.doc(address).map_err(adapter)?;
                let id = document
                    .get_first(self.id)
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        StoreError::Verification("indexed memory id is absent".into())
                    })?;
                Ok((id.to_owned(), score))
            })
            .collect()
    }

    async fn status(&self) -> Result<Value, StoreError> {
        self.reader.reload().map_err(adapter)?;
        let searcher = self.reader.searcher();
        let active_query = TermQuery::new(
            Term::from_field_text(self.active, "true"),
            IndexRecordOption::Basic,
        );
        let documents = searcher.search(&active_query, &Count).map_err(adapter)?;
        Ok(json!({
            "ready": true,
            "kind": "tantivy",
            "documents": documents,
        }))
    }

    async fn rebuild(&self, records: &[(String, String, Value)]) -> Result<(), StoreError> {
        let mut writer = self.writer.lock().map_err(adapter)?;
        writer.delete_all_documents().map_err(adapter)?;
        for (id, text, metadata) in records {
            let metadata = serde_json::to_string(metadata).map_err(adapter)?;
            writer
                .add_document(doc!(
                    self.id => id.clone(),
                    self.event_id => format!("rebuild:{id}"),
                    self.text => text.clone(),
                    self.metadata => metadata,
                    self.active => "true",
                ))
                .map_err(adapter)?;
        }
        writer.commit().map_err(adapter)?;
        self.reader.reload().map_err(adapter)
    }
}

/// Canonical memory lifecycle, disposable-index synchronization, and re-filtered retrieval.
pub struct MemoryService {
    journal: Arc<dyn EventJournal>,
    repository: Arc<dyn MemoryRepository>,
    index: Arc<dyn MemoryIndex>,
    sessions: Arc<dyn SessionRepository>,
    index_position: AtomicU64,
    last_index_error: Mutex<Option<String>>,
}

impl MemoryService {
    /// Compose memory behavior from replaceable repository and index ports.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        repository: Arc<dyn MemoryRepository>,
        index: Arc<dyn MemoryIndex>,
        sessions: Arc<dyn SessionRepository>,
    ) -> Self {
        let index_position = index.position().unwrap_or(0);
        Self {
            journal,
            repository,
            index,
            sessions,
            index_position: AtomicU64::new(index_position),
            last_index_error: Mutex::new(None),
        }
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
        let sync_ok = self.sync_index().await.is_ok();
        if query.trim().is_empty() || !sync_ok {
            return self.fallback(query, session_id, repository_id, limit);
        }
        let candidates = match self.index.search(query, limit.saturating_mul(4)).await {
            Ok(candidates) => candidates,
            Err(error) => {
                self.record_index_error(&error);
                return self.fallback(query, session_id, repository_id, limit);
            }
        };
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
        match self.sync_index_inner().await {
            Ok(position) => Ok(position),
            Err(error) => {
                self.record_index_error(&error);
                Err(error)
            }
        }
    }

    async fn sync_index_inner(&self) -> Result<u64, StoreError> {
        let mut position = self.index_position.load(Ordering::Acquire);
        loop {
            let events = self.journal.read_global(position.saturating_add(1), 256)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if let Some(id) = event.stream_id.strip_prefix("memory:") {
                    match event.event_type.as_str() {
                        "memory.created.v1" => {
                            let payload = self.journal.decrypt_payload(event)?;
                            let record: MemoryRecord = serde_json::from_value(
                                payload.get("record").cloned().ok_or_else(|| {
                                    StoreError::Verification(
                                        "memory index event has no canonical record".into(),
                                    )
                                })?,
                            )
                            .map_err(adapter)?;
                            self.index
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
                            self.index.remove(&event.event_id, id).await?;
                        }
                        _ => {}
                    }
                }
                position = event.global_sequence;
            }
            self.index.set_position(position).await?;
            self.index_position.store(position, Ordering::Release);
            if events.len() < 256 {
                break;
            }
        }
        *self.last_index_error.lock().map_err(adapter)? = None;
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
        self.index.rebuild(&values).await?;
        let (head, _) = self.journal.head()?;
        self.index.set_position(head).await?;
        self.index_position.store(head, Ordering::Release);
        *self.last_index_error.lock().map_err(adapter)? = None;
        self.index_status().await
    }

    /// Return bounded index readiness, lag, and adapter details.
    pub async fn index_status(&self) -> Result<Value, StoreError> {
        let (head, _) = self.journal.head()?;
        let position = self.index_position.load(Ordering::Acquire);
        let adapter_status = self.index.status().await?;
        let error = self.last_index_error.lock().map_err(adapter)?.clone();
        Ok(json!({
            "ready": error.is_none() && position == head,
            "position": position,
            "journal_head": head,
            "lag": head.saturating_sub(position),
            "last_error": error,
            "adapter": adapter_status,
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
        if let Err(error) = self.sync_index().await {
            self.record_index_error(&error);
        }
    }

    fn record_index_error(&self, error: &StoreError) {
        if let Ok(mut last_error) = self.last_index_error.lock() {
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

#[cfg(test)]
mod tests {
    use super::{
        EventSourcedMemoryRepository, MemoryService, TantivyMemoryIndex, UnavailableMemoryIndex,
    };
    use colossus_contracts::{Actor, ActorType, MemoryRecord, MemoryScope, MemoryStatus};
    use colossus_ports::{EventJournal, MemoryIndex, MemoryRepository, SessionRepository};
    use colossus_session::EventSourcedSessionRepository;
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn actor() -> Actor {
        Actor {
            actor_type: ActorType::User,
            id: "test-user".into(),
        }
    }

    fn memory(id: &str, text: &str) -> MemoryRecord {
        MemoryRecord {
            id: id.into(),
            scope: MemoryScope::Global,
            kind: "preference".into(),
            confidence: 0.9,
            source: "user".into(),
            status: MemoryStatus::Active,
            text: text.into(),
            rationale: "test".into(),
            created_at: "2026-07-09T00:00:00Z".into(),
            updated_at: "2026-07-09T00:00:00Z".into(),
            expires_at: None,
            superseded_by: None,
        }
    }

    #[test]
    fn canonical_lifecycle_is_reconstructed_and_supersession_is_atomic() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedMemoryRepository::new(Arc::clone(&journal));
        repository
            .create(memory("old", "Use Rust"), actor())
            .expect("create");
        let (old, replacement) = repository
            .supersede("old", memory("new", "Use Rust 1.96"), actor())
            .expect("supersede");
        assert_eq!(old.status, MemoryStatus::Superseded);
        assert_eq!(old.superseded_by.as_deref(), Some("new"));
        assert_eq!(replacement.status, MemoryStatus::Active);
        assert_eq!(
            repository.list_active(10).expect("active"),
            vec![replacement]
        );
    }

    #[tokio::test]
    async fn tantivy_index_is_idempotent_disposable_and_returns_candidate_ids() {
        let directory = tempdir().expect("tempdir");
        let index = TantivyMemoryIndex::open(directory.path()).expect("index");
        index
            .upsert(
                "event-1",
                "memory-1",
                "Rust workflow runtime",
                &json!({}),
                None,
            )
            .await
            .expect("upsert");
        index
            .upsert("event-1", "memory-1", "ignored duplicate", &json!({}), None)
            .await
            .expect("idempotent upsert");
        assert_eq!(
            index.search("Rust", 10).await.expect("search")[0].0,
            "memory-1"
        );
        index.remove("event-2", "memory-1").await.expect("remove");
        assert!(
            index
                .search("Rust", 10)
                .await
                .expect("removed search")
                .is_empty()
        );
        index
            .rebuild(&[("memory-2".into(), "Chroma candidate".into(), json!({}))])
            .await
            .expect("rebuild");
        assert_eq!(
            index.search("Chroma", 10).await.expect("rebuilt")[0].0,
            "memory-2"
        );
    }

    #[tokio::test]
    async fn service_replays_index_events_without_resurrecting_archived_records() {
        let directory = tempdir().expect("tempdir");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        sessions
            .create_session("session-1", None, actor())
            .expect("session");
        let repository: Arc<dyn MemoryRepository> =
            Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
        let index: Arc<dyn MemoryIndex> =
            Arc::new(TantivyMemoryIndex::open(directory.path()).expect("index"));
        let service = MemoryService::new(
            Arc::clone(&journal),
            Arc::clone(&repository),
            Arc::clone(&index),
            Arc::clone(&sessions),
        );
        let created = service
            .create(
                MemoryScope::Session("session-1".into()),
                "preference",
                1.0,
                "Run Rust tests before completion",
                "user preference",
                None,
                actor(),
            )
            .await
            .expect("create");
        assert_eq!(
            service
                .search("Rust tests", Some("session-1"), None, 8)
                .await
                .expect("search"),
            vec![created.clone()]
        );
        assert!(
            service
                .search("Rust tests", Some("other-session"), None, 8)
                .await
                .expect("scope filtered")
                .is_empty()
        );
        service
            .archive(&created.id, actor())
            .await
            .expect("archive");

        let reopened = MemoryService::new(journal, repository, index, sessions);
        let persisted_status = reopened.index_status().await.expect("persisted status");
        assert_eq!(persisted_status["ready"], true);
        assert_eq!(persisted_status["lag"], 0);
        reopened.sync_index().await.expect("replay");
        assert!(
            reopened
                .search("Rust tests", Some("session-1"), None, 8)
                .await
                .expect("search after restart")
                .is_empty()
        );
        assert_eq!(
            reopened
                .get(&created.id)
                .expect("get")
                .expect("record")
                .status,
            MemoryStatus::Archived
        );
    }

    #[tokio::test]
    async fn unavailable_index_leaves_canonical_memory_usable_and_lag_visible() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        sessions
            .create_session("session-1", None, actor())
            .expect("session");
        let repository: Arc<dyn MemoryRepository> =
            Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
        let index: Arc<dyn MemoryIndex> =
            Arc::new(UnavailableMemoryIndex::new("index unavailable"));
        let service = MemoryService::new(journal, repository, index, sessions);
        let record = service
            .create(
                MemoryScope::Global,
                "warning",
                0.8,
                "Canonical memory remains available",
                "fallback test",
                None,
                actor(),
            )
            .await
            .expect("canonical create");
        assert_eq!(
            service
                .search("Canonical", Some("session-1"), None, 8)
                .await
                .expect("fallback search"),
            vec![record]
        );
        let status = service.index_status().await.expect("status");
        assert_eq!(status["ready"], false);
        assert!(status["lag"].as_u64().is_some_and(|lag| lag > 0));
        assert!(status["last_error"].as_str().is_some());
    }
}
