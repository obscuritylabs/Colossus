use super::*;
use colossus_contracts::{EventEnvelope, ProjectionWorkItem, SignedCheckpoint};
use colossus_ports::VerificationReport;

#[test]
fn failed_final_grant_event_rolls_back_receipt_and_cache_for_edits_replacements_and_renames() {
    for scenario in ["edit", "replacement", "rename"] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let parent = root.join(".agents/plugins");
        let original = parent.join("Review");
        write_plugin(&original);
        let first = capture_workspace_plugin(&root, Path::new(".agents/plugins/Review")).unwrap();
        let store = PluginStore::new(root.join("store")).unwrap();
        let installed = store
            .accept_workspace_plugin(&first, &BTreeSet::new(), actor())
            .unwrap();
        let relative = match scenario {
            "edit" => ".agents/plugins/Review",
            "replacement" => {
                write_plugin(&parent.join("replacement"));
                ".agents/plugins/replacement"
            }
            _ => {
                fs::rename(&original, parent.join("rename-in-progress")).unwrap();
                fs::rename(parent.join("rename-in-progress"), parent.join("review")).unwrap();
                ".agents/plugins/review"
            }
        };
        fs::write(
            root.join(relative).join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Updated review\n---\nUpdated captured instructions\n",
        )
        .unwrap();
        let candidate = capture_workspace_plugin(&root, Path::new(relative)).unwrap();
        store.with_write(|repository| {
            let grants = serde_json::to_value(repository.workspace_grants()?).unwrap();
            let cache = repository.workspace_cache()?;
            let head = repository.journal.head()?;
            let failing = EventSourcedPluginRepository::new(Arc::new(FailingGrantJournal {
                inner: Arc::clone(&repository.journal),
            }));
            let error = store.accept_workspace_plugin_locked(
                &failing, &candidate, &BTreeSet::new(), actor()).unwrap_err();
            assert!(matches!(error, StoreError::Conflict { stream_id, .. } if stream_id == SOURCES_STREAM));
            assert_eq!(serde_json::to_value(repository.workspace_grants()?).unwrap(), grants);
            assert_eq!(repository.workspace_cache()?, cache);
            assert_eq!(repository.journal.head()?, head);
            assert!(repository.reduce_installation(&candidate.source.name,
                &candidate.artifact.manifest_digest)?.is_none());
            assert!(Path::new(&installed.root).is_dir());
            Ok(())
        }).unwrap();
        // A fresh store sees the original durable state; the orphan publication
        // is collected before a successful retry commits the complete batch.
        let reopened = PluginStore::new(root.join("store")).unwrap();
        assert_eq!(reopened.workspace_plugin_grants().unwrap().len(), 1);
        assert_eq!(reopened.list(10_000).unwrap().len(), 1);
        if scenario != "edit" {
            assert!(
                reopened
                    .snapshot_workspace_plugin(&candidate, &BTreeSet::new(), actor())
                    .is_err()
            );
        }
        reopened
            .accept_workspace_plugin(&candidate, &BTreeSet::new(), actor())
            .unwrap();
        assert!(reopened.workspace_plugin_grants().unwrap()[relative].enabled);
    }
}

/// Cause the real journal transaction to fail on its final source-grant event,
/// after it has staged any preceding receipt/cache events in the transaction.
struct FailingGrantJournal {
    inner: Arc<dyn EventJournal>,
}

fn fail_grant(event: &mut NewEvent) {
    if event.stream_id == SOURCES_STREAM {
        event.expected_stream_version = u64::MAX;
    }
}

impl EventJournal for FailingGrantJournal {
    fn append(&self, mut event: NewEvent) -> Result<EventEnvelope, StoreError> {
        fail_grant(&mut event);
        self.inner.append(event)
    }
    fn append_batch(&self, mut events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        for event in &mut events {
            fail_grant(event);
        }
        self.inner.append_batch(events)
    }
    fn read_stream(&self, stream: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream(stream)
    }
    fn read_stream_from(
        &self,
        stream: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_from(stream, after, limit)
    }
    fn read_stream_backwards(
        &self,
        stream: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_backwards(stream, before, limit)
    }
    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.inner.list_stream_ids(prefix, after, limit)
    }
    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_global(from, limit)
    }
    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.inner.read_projection_work(from, limit)
    }
    fn head(&self) -> Result<(u64, String), StoreError> {
        self.inner.head()
    }
    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        self.inner.decrypt_payload(event)
    }
    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.inner.verify()
    }
    fn is_recovery_mode(&self) -> bool {
        self.inner.is_recovery_mode()
    }
    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.inner.checkpoint()
    }
}
