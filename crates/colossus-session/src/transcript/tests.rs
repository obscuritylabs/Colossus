use super::*;
use colossus_contracts::{
    ActorType, AgentMessageOrigin, AgentMessageSender, ModelImageDetail, ModelImageReference,
    ModelToolCall,
};
use colossus_ports::VerificationReport;
use colossus_testkit::InMemoryEventJournal;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct IndexedOnlyJournal {
    inner: InMemoryEventJournal,
    reading_export: AtomicBool,
    reads: AtomicUsize,
}

impl EventJournal for IndexedOnlyJournal {
    fn append(&self, event: NewEvent) -> Result<colossus_contracts::EventEnvelope, StoreError> {
        self.inner.append(event)
    }
    fn append_batch(
        &self,
        events: Vec<NewEvent>,
    ) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        self.inner.append_batch(events)
    }
    fn read_stream(&self, id: &str) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        assert!(
            !self.reading_export.load(Ordering::Relaxed),
            "export must never reconstruct a complete stream"
        );
        self.inner.read_stream(id)
    }
    fn read_stream_from(
        &self,
        id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        if self.reading_export.load(Ordering::Relaxed) {
            assert!(limit <= MAX_SCAN_EVENTS);
            self.reads.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.read_stream_from(id, after, limit)
    }
    fn read_stream_backwards(
        &self,
        id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        if self.reading_export.load(Ordering::Relaxed) {
            assert!(limit <= MAX_SCAN_EVENTS);
            self.reads.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.read_stream_backwards(id, before, limit)
    }
    fn list_stream_ids(
        &self,
        _: &str,
        _: Option<&str>,
        _: usize,
    ) -> Result<Vec<String>, StoreError> {
        panic!("transcript must not enumerate unrelated sessions")
    }
    fn read_global(
        &self,
        _: u64,
        _: usize,
    ) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        panic!("transcript must not scan global history")
    }
    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<colossus_contracts::ProjectionWorkItem>, StoreError> {
        self.inner.read_projection_work(from, limit)
    }
    fn head(&self) -> Result<(u64, String), StoreError> {
        self.inner.head()
    }
    fn decrypt_payload(
        &self,
        event: &colossus_contracts::EventEnvelope,
    ) -> Result<Value, StoreError> {
        self.inner.decrypt_payload(event)
    }
    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.inner.verify()
    }
    fn is_recovery_mode(&self) -> bool {
        self.inner.is_recovery_mode()
    }
    fn checkpoint(&self) -> Result<Option<colossus_contracts::SignedCheckpoint>, StoreError> {
        self.inner.checkpoint()
    }
}

fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "export-test".into(),
    }
}
fn message(role: ModelMessageRole, text: &str) -> ModelMessage {
    ModelMessage {
        role,
        content: text.into(),
        tool_calls: vec![],
        tool_call_id: None,
        agent_message_origin: None,
    }
}

#[test]
fn indexed_pages_advance_through_private_records_and_freeze_appends() {
    let journal = Arc::new(IndexedOnlyJournal::default());
    let canonical: Arc<dyn EventJournal> = journal.clone();
    let repository = EventSourcedSessionRepository::new(canonical.clone());
    repository
        .create_session("export-session", Some("Conversation"), actor())
        .unwrap();
    for _ in 0..MAX_SCAN_EVENTS {
        let mut tool = message(ModelMessageRole::Tool, "PRIVATE_TOOL_RESULT");
        tool.tool_call_id = Some("call".into());
        repository
            .append_message("export-session", "run-1", tool, actor())
            .unwrap();
    }
    repository
        .append_message(
            "export-session",
            "run-2",
            message(ModelMessageRole::User, "retained question"),
            actor(),
        )
        .unwrap();
    let reader = CanonicalTranscriptReader::new(canonical);
    journal.reading_export.store(true, Ordering::Relaxed);
    let first = reader
        .read("export-session", None, 32, TranscriptActivity::Active)
        .unwrap();
    assert!(first.messages.is_empty());
    let cursor = first.next_cursor.as_deref().unwrap();
    assert!(
        journal.reads.load(Ordering::Relaxed) <= MAX_SCAN_EVENTS / READ_BATCH_EVENTS + 4,
        "work stays independent of total history"
    );
    journal.reading_export.store(false, Ordering::Relaxed);
    repository
        .append_message(
            "export-session",
            "run-3",
            message(ModelMessageRole::Assistant, "later excluded append"),
            actor(),
        )
        .unwrap();
    journal.reading_export.store(true, Ordering::Relaxed);
    let last = reader
        .read("export-session", Some(cursor), 32, TranscriptActivity::Idle)
        .unwrap();
    assert_eq!(last.snapshot, first.snapshot);
    assert!(last.next_cursor.is_none());
    assert_eq!(last.messages.len(), 1);
    assert_eq!(last.messages[0].sequence, MAX_SCAN_EVENTS as u64 + 1);
    assert_eq!(last.messages[0].run_id, "run-2");
    assert!(
        !serde_json::to_string(&last)
            .unwrap()
            .contains("PRIVATE_TOOL_RESULT")
    );
}

#[test]
fn released_projection_keeps_full_prose_multipart_order_and_no_private_fields() {
    let large = "```rust\nlet quoted = \"é\\n\";\n```\n".repeat(6000);
    let mut assistant = message(ModelMessageRole::Assistant, &large);
    assistant.tool_calls.push(ModelToolCall {
        call_id: "call".into(),
        name: "private.tool".into(),
        arguments: json!({"token":"PRIVATE_ARGUMENT"}),
    });
    let record = |message| SessionMessage {
        session_id: "session".into(),
        run_id: "run".into(),
        sequence: 1,
        created_at: "2026-11-18T00:00:00Z".into(),
        message,
    };
    let projected = project(record(assistant)).unwrap();
    assert_eq!(projected.parts, vec![TranscriptPart::Text { text: large }]);
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("PRIVATE_ARGUMENT")
    );
    let mut peer = message(ModelMessageRole::User, "PRIVATE_PEER");
    peer.agent_message_origin = Some(AgentMessageOrigin {
        message_id: "message".into(),
        recipient_id: "recipient".into(),
        sender: AgentMessageSender::Participant {
            participant_id: "sender".into(),
        },
    });
    assert!(project(record(peer)).is_none());
    for role in [
        ModelMessageRole::System,
        ModelMessageRole::Tool,
        ModelMessageRole::ToolObservation,
    ] {
        assert!(project(record(message(role, "PRIVATE"))).is_none());
    }
    let mut input = message(ModelMessageRole::User, "");
    input.content = ModelContent::Parts(vec![
        ModelContentPart::Text {
            text: "before".into(),
        },
        ModelContentPart::Image {
            image: ModelImageReference {
                artifact_id: "PRIVATE_ARTIFACT_HANDLE".into(),
                file_name: "picture.png".into(),
                media_type: "image/png".into(),
                size_bytes: 42,
                sha256: "a".repeat(64),
                width_pixels: 1,
                height_pixels: 1,
                detail: ModelImageDetail::Auto,
            },
        },
        ModelContentPart::Text {
            text: "after".into(),
        },
    ]);
    let projected = project(record(input)).unwrap();
    assert!(
        matches!(&projected.parts[..], [TranscriptPart::Text{text:before}, TranscriptPart::Attachment{file_name,..}, TranscriptPart::Text{text:after}] if before=="before" && file_name=="picture.png" && after=="after")
    );
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("PRIVATE_ARTIFACT_HANDLE")
    );
}

#[test]
fn empty_sessions_are_complete_and_cursors_reject_another_session() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(journal.clone());
    repository.create_session("empty", None, actor()).unwrap();
    let reader = CanonicalTranscriptReader::new(journal);
    let page = reader
        .read("empty", None, 1, TranscriptActivity::Idle)
        .unwrap();
    assert!(page.messages.is_empty() && page.next_cursor.is_none());
    assert!(
        reader
            .read("not-materialized", None, 32, TranscriptActivity::Idle)
            .is_err()
    );
    let forged = serde_json::to_string(&Cursor {
        snapshot: TranscriptSnapshot {
            stream_version: 5,
            ..page.snapshot
        },
        after_version: 1,
        last_sequence: 0,
    })
    .unwrap();
    assert!(
        reader
            .read("empty", Some(&forged), 32, TranscriptActivity::Idle)
            .is_err()
    );
}

#[test]
fn maximum_valid_escaped_messages_fit_without_truncation_and_split_pages() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(journal.clone());
    repository.create_session("large", None, actor()).unwrap();
    let large = "\"\\\n".repeat(160_000);
    assert!(
        serde_json::to_vec(&message(ModelMessageRole::User, &large))
            .unwrap()
            .len()
            < MAX_MESSAGE_BYTES
    );
    for run in ["one", "two"] {
        repository
            .append_message(
                "large",
                run,
                message(ModelMessageRole::User, &large),
                actor(),
            )
            .unwrap();
    }
    let reader = CanonicalTranscriptReader::new(journal);
    let first = reader
        .read("large", None, 32, TranscriptActivity::Idle)
        .unwrap();
    assert_eq!(first.messages.len(), 1);
    assert!(serde_json::to_vec(&first).unwrap().len() < MAX_TRANSCRIPT_PAGE_BYTES);
    let second = reader
        .read(
            "large",
            first.next_cursor.as_deref(),
            32,
            TranscriptActivity::Idle,
        )
        .unwrap();
    assert_eq!(second.messages.len(), 1);
    assert!(second.next_cursor.is_none());
    assert_eq!(first.snapshot, second.snapshot);
    for page in [first, second] {
        assert_eq!(
            page.messages[0].parts,
            vec![TranscriptPart::Text {
                text: large.clone()
            }]
        );
    }
}

#[test]
fn oversized_legacy_record_fails_instead_of_reporting_a_shortened_success() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(journal.clone());
    repository.create_session("legacy", None, actor()).unwrap();
    journal.append(NewEvent {
        event_version:1,stream_id:"session:legacy".into(),expected_stream_version:1,
        classification:EventClassification::Domain,event_type:MESSAGE_EVENT.into(),actor:actor(),
        context:ExecutionContext {session_id:Some("legacy".into()),run_id:Some("old-run".into()),..ExecutionContext::default()},
        payload:json!({"run_id":"old-run","sequence":1,"message":message(ModelMessageRole::User,&"x".repeat(MAX_TRANSCRIPT_PAGE_BYTES))}),
    }).unwrap();
    let reader = CanonicalTranscriptReader::new(journal);
    assert!(
        matches!(reader.read("legacy",None,32,TranscriptActivity::Idle),Err(StoreError::Adapter(message)) if message=="canonical transcript message exceeds the export page bound")
    );
}

#[test]
fn archived_history_and_a_resumed_snapshot_survive_reader_recreation() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(journal.clone());
    repository
        .create_session("retained", Some("Opening title"), actor())
        .unwrap();
    repository
        .set_title("retained", "Saved title", actor())
        .unwrap();
    for (run, text) in [("run-one", "first"), ("run-two", "second")] {
        repository
            .append_message(
                "retained",
                run,
                message(ModelMessageRole::User, text),
                actor(),
            )
            .unwrap();
    }
    repository
        .set_session_archived("retained", true, 0, actor())
        .unwrap();
    let first = CanonicalTranscriptReader::new(journal.clone())
        .read("retained", None, 1, TranscriptActivity::Idle)
        .unwrap();
    assert_eq!(first.snapshot.title.as_deref(), Some("Saved title"));
    repository
        .set_title("retained", "Later title", actor())
        .unwrap();
    let second = CanonicalTranscriptReader::new(journal)
        .read(
            "retained",
            first.next_cursor.as_deref(),
            1,
            TranscriptActivity::Idle,
        )
        .unwrap();
    assert_eq!(second.snapshot, first.snapshot);
    assert_eq!(second.messages[0].run_id, "run-two");
    // Follow any remaining metadata-only page through the frozen boundary.
    let mut page = second;
    while let Some(cursor) = page.next_cursor {
        page = CanonicalTranscriptReader::new(repository.journal.clone())
            .read("retained", Some(&cursor), 1, TranscriptActivity::Idle)
            .unwrap();
        assert_eq!(page.snapshot, first.snapshot);
    }
}
