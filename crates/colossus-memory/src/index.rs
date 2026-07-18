use super::*;

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
