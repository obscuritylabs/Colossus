//! Canonical event-sourced memories and the disposable Tantivy lexical index.

use async_trait::async_trait;
use colossus_contracts::{
    Actor, EventClassification, ExecutionContext, MemoryRecord, MemoryStatus, NewEvent,
};
use colossus_ports::{EventJournal, MemoryIndex, MemoryRepository, StoreError};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
    collector::TopDocs,
    doc,
    query::{QueryParser, TermQuery},
    schema::{Field, IndexRecordOption, STORED, STRING, Schema, TEXT, Value as _},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_MEMORY_TEXT_BYTES: usize = 64 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

fn validate_record(record: &MemoryRecord) -> Result<(), StoreError> {
    if record.id.is_empty()
        || record.kind.is_empty()
        || record.source.is_empty()
        || record.text.is_empty()
        || record.text.len() > MAX_MEMORY_TEXT_BYTES
        || !record.confidence.is_finite()
        || !(0.0..=1.0).contains(&record.confidence)
        || record.status != MemoryStatus::Active
    {
        return Err(StoreError::Adapter(
            "memory id/kind/source/text/confidence/status is invalid".into(),
        ));
    }
    let normalized = record.text.to_ascii_lowercase();
    if normalized.contains("-----begin private key-----")
        || normalized.contains("authorization: bearer ")
        || normalized.contains("password=")
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
    // Kept private until the runtime exposes it only through a permit-requiring effect
    // executor. Public construction here would create a durable-mutation bypass.
    #[allow(dead_code)]
    fn new(journal: Arc<dyn EventJournal>) -> Self {
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
        let mut ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "memory.created.v1"
                && let Some(id) = event.stream_id.strip_prefix("memory:")
            {
                ids.insert(id.to_owned());
            }
        }
        let current_time = OffsetDateTime::now_utc();
        let mut records = Vec::new();
        for id in ids {
            let Some(record) = self.get_memory(&id)? else {
                continue;
            };
            let expired = record.expires_at.as_deref().is_some_and(|value| {
                OffsetDateTime::parse(value, &Rfc3339).is_ok_and(|expiry| expiry <= current_time)
            });
            if record.status == MemoryStatus::Active && !expired {
                records.push(record);
                if records.len() >= limit {
                    break;
                }
            }
        }
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

impl TantivyMemoryIndex {
    // Kept private until indexing is wired through the gateway as a configured adapter.
    #[allow(dead_code)]
    fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
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
                self.id => memory_id.to_owned(),
                self.event_id => event_id.to_owned(),
                self.metadata => "{}",
                self.active => "false",
            ))
            .map_err(adapter)?;
        writer.commit().map_err(adapter)?;
        self.reader.reload().map_err(adapter)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>, StoreError> {
        self.reader.reload().map_err(adapter)?;
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.text]);
        let query = parser.parse_query(query).map_err(adapter)?;
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
        Ok(json!({
            "ready": true,
            "kind": "tantivy",
            "documents": self.reader.searcher().num_docs(),
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

#[cfg(test)]
mod tests {
    use super::{EventSourcedMemoryRepository, TantivyMemoryIndex};
    use colossus_contracts::{Actor, ActorType, MemoryRecord, MemoryScope, MemoryStatus};
    use colossus_ports::{EventJournal, MemoryIndex, MemoryRepository};
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
}
