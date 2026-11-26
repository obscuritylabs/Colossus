use super::*;
use colossus_ports::RunControl;
use std::collections::BTreeMap;

const SUMMARY_PAGE_SIZE: usize = 128;

#[derive(Deserialize)]
struct LegacySummary {
    id: String,
    stream_id: String,
    title: Option<String>,
    created_at: String,
    updated_at: String,
    message_count: u64,
    last_run_id: Option<String>,
}

impl LegacySummary {
    fn into_summary(self, key: &str) -> Result<SessionSummary, StoreError> {
        if self.id != key || self.stream_id != EventSourcedSessionRepository::stream(key) {
            return Err(invalid("legacy session summary identity mismatch"));
        }
        Ok(SessionSummary {
            id: self.id,
            title: self.title,
            created_at: self.created_at,
            updated_at: self.updated_at,
            message_count: self.message_count,
            last_run_id: self.last_run_id,
            // The old preview can be from a peer message presented to the model as User.
            last_user_preview: None,
        })
    }
}

impl IndexedSessionCatalog {
    /// List from existing summary rows while the ordered catalog is being rebuilt.
    ///
    /// The host checks the summary watermark and guards the read against projection
    /// resets. This reads at most one page plus the requested number of summaries
    /// into memory and consults canonical lifecycle metadata for archive membership.
    pub fn recent_from_projected_summaries(
        &self,
        journal: &dyn EventJournal,
        projection: &str,
        archived: bool,
        limit: usize,
        control: Option<&RunControl>,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let has_lifecycles = !journal
            .list_stream_ids(lifecycle::LIFECYCLE_STREAM_PREFIX, None, 1)?
            .is_empty();
        if archived && !has_lifecycles {
            return Ok(Vec::new());
        }
        let check_cancelled = || {
            if control.is_some_and(RunControl::is_cancelled) {
                return Err(StoreError::Adapter(
                    "session catalog read was cancelled".into(),
                ));
            }
            Ok(())
        };
        let mut most_recent = BTreeMap::new();
        let mut after = None;
        loop {
            check_cancelled()?;
            let page =
                self.store
                    .list_after(projection, "", after.as_deref(), SUMMARY_PAGE_SIZE)?;
            let Some((last, _)) = page.last() else {
                break;
            };
            if Some(last.as_str()) == after.as_deref() {
                return Err(invalid("legacy session summary page did not advance"));
            }
            after = Some(last.clone());
            for (id, row) in page {
                let summary = decode::<LegacySummary>(row)?.into_summary(&id)?;
                let key = recent_key(archived, &summary)?;
                if most_recent.len() == limit
                    && most_recent
                        .last_key_value()
                        .is_some_and(|(last, _)| key >= *last)
                {
                    continue;
                }
                if has_lifecycles
                    && lifecycle::read_lifecycle_tail(journal, &id)?.archived != archived
                {
                    continue;
                }
                most_recent.insert(key, summary);
                if most_recent.len() > limit {
                    most_recent.pop_last();
                }
            }
        }
        for summary in most_recent.values_mut() {
            check_cancelled()?;
            if let Some(row) = self
                .store
                .get(SESSION_CATALOG_PROJECTION, &format!("id:{}", summary.id))?
            {
                let indexed = decode::<CatalogEntry>(row)?.summary;
                if indexed.id != summary.id {
                    return Err(invalid("session catalog preview identity mismatch"));
                }
                // Only an appended message can change the human preview.
                if indexed.message_count == summary.message_count {
                    summary.last_user_preview = indexed.last_user_preview;
                }
            }
        }
        Ok(most_recent.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::ProjectionBatch;
    use colossus_testkit::{InMemoryEventJournal, InMemoryProjectionStore};

    #[test]
    fn newest_summary_can_be_beyond_the_first_legacy_page() {
        let store = Arc::new(InMemoryProjectionStore::default());
        let projection = "legacy-fixture";
        let mut mutations = Vec::new();
        for index in 0..=SUMMARY_PAGE_SIZE {
            let id = format!("session-{index:04}");
            let timestamp = if index == SUMMARY_PAGE_SIZE {
                "2026-11-01T00:00:00.100001Z"
            } else {
                "2026-11-01T00:00:00.1Z"
            };
            mutations.push(ProjectionMutation::Upsert {
                key: id.clone(),
                value: json!({
                    "id": id,
                    "stream_id": format!("session:{id}"),
                    "title": null,
                    "created_at": timestamp,
                    "updated_at": timestamp,
                    "message_count": 0,
                    "last_run_id": null,
                    "last_user_preview": "unverified legacy preview",
                }),
            });
        }
        store
            .apply(ProjectionBatch {
                projection: projection.to_owned(),
                expected_position: 0,
                through_sequence: 1,
                mutations,
            })
            .unwrap();

        let summaries = IndexedSessionCatalog::new(store)
            .recent_from_projected_summaries(
                &InMemoryEventJournal::default(),
                projection,
                false,
                2,
                None,
            )
            .unwrap();
        assert_eq!(summaries[0].id, format!("session-{SUMMARY_PAGE_SIZE:04}"));
        assert_eq!(
            summaries[1].id,
            format!("session-{:04}", SUMMARY_PAGE_SIZE - 1)
        );
        assert!(
            summaries
                .iter()
                .all(|summary| summary.last_user_preview.is_none())
        );
    }
}
