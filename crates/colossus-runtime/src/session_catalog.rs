use std::sync::Arc;

use crate::{Runtime, RuntimeError};
use colossus_contracts::SessionSummary;
use colossus_ports::{
    EventJournal, ProjectionStore, SessionCatalog, SessionMessagePosition, StoreError,
};
use colossus_projection::ProjectionWorker;
use colossus_session::{IndexedSessionCatalog, SESSION_CATALOG_PROJECTION};

const BATCH: usize = 64;
const MAX_ROUNDS: usize = 65_536;
const MAX_WRITE_LAG: u64 = 1_024;

pub(super) fn required_for_open_and_checkpoint(name: &str) -> bool {
    name != SESSION_CATALOG_PROJECTION
}

impl Runtime {
    /// Read the terminal catalog, yielding between batches when upgrading an existing journal.
    pub async fn terminal_session_catalog(
        &self,
        archived: bool,
        limit: usize,
    ) -> Result<Vec<SessionSummary>, RuntimeError> {
        if self.journal.is_recovery_mode() {
            return if archived {
                self.list_archived_sessions(limit)
            } else {
                self.list_sessions(limit)
            };
        }
        let through = self.journal.head()?.0;
        let limit = limit.clamp(1, 100);
        for _ in 0..MAX_ROUNDS {
            if let Some(sessions) = self
                .session_catalog
                .indexed_recent(through, archived, limit)?
            {
                return Ok(sessions);
            }
            if !self.session_catalog.advance_batch()? {
                break;
            }
            tokio::task::yield_now().await;
        }
        self.session_catalog
            .indexed_recent(through, archived, limit)?
            .ok_or_else(|| catalog_rebuilding().into())
    }
}

fn catalog_rebuilding() -> StoreError {
    StoreError::Adapter("session catalog is still rebuilding".into())
}

pub(super) struct RuntimeSessionCatalog {
    index: IndexedSessionCatalog,
    journal: Arc<dyn EventJournal>,
    store: Arc<dyn ProjectionStore>,
    worker: Arc<ProjectionWorker>,
}

impl RuntimeSessionCatalog {
    pub(super) fn new(
        journal: Arc<dyn EventJournal>,
        store: Arc<dyn ProjectionStore>,
        worker: Arc<ProjectionWorker>,
    ) -> Self {
        Self {
            index: IndexedSessionCatalog::new(store.clone()),
            journal,
            store,
            worker,
        }
    }

    fn position(&self) -> Result<u64, StoreError> {
        self.store.position(SESSION_CATALOG_PROJECTION)
    }

    fn indexed_recent(
        &self,
        through: u64,
        archived: bool,
        limit: usize,
    ) -> Result<Option<Vec<SessionSummary>>, StoreError> {
        self.worker.read_without_reset(|| {
            if self.position()? < through {
                return Ok(None);
            }
            self.index.recent(archived, limit).map(Some)
        })
    }

    fn advance_batch(&self) -> Result<bool, StoreError> {
        self.worker
            .drain_selected(BATCH, 1, |name| name == SESSION_CATALOG_PROJECTION)
            .map(|report| report.applied > 0)
    }

    fn catch_up(&self, through: u64, max_rounds: usize) -> Result<(), StoreError> {
        for _ in 0..max_rounds {
            if self.position()? >= through {
                return Ok(());
            }
            // Give other projection work a chance to run during a legacy backfill.
            if !self.advance_batch()? {
                break;
            }
        }
        if self.position()? >= through {
            Ok(())
        } else {
            Err(catalog_rebuilding())
        }
    }

    fn advance_after_write(&self) -> Result<(), StoreError> {
        let through = self.journal.head()?.0;
        // Leave a legacy backfill to projection maintenance or an explicit catalog read.
        if through.saturating_sub(self.position()?) <= MAX_WRITE_LAG {
            self.catch_up(through, MAX_WRITE_LAG as usize / BATCH)?;
        }
        Ok(())
    }
}

impl SessionCatalog for RuntimeSessionCatalog {
    fn recent(&self, archived: bool, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        // The result must cover everything committed before this read.
        let through = self.journal.head()?.0;
        let read = || self.indexed_recent(through, archived, limit);
        if let Some(sessions) = read()? {
            return Ok(sessions);
        }
        self.catch_up(through, MAX_ROUNDS)?;
        read()?.ok_or_else(catalog_rebuilding)
    }

    fn message_positions(
        &self,
        id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Option<Vec<SessionMessagePosition>>, StoreError> {
        if self.journal.is_recovery_mode() {
            return Ok(None);
        }
        self.worker.read_without_reset(|| {
            // A cursor already indexed remains usable even while new messages arrive.
            let cursor_indexed = match before_sequence {
                Some(sequence) => self.index.contains_message(id, sequence)?,
                None => false,
            };
            if !cursor_indexed && self.position()? < self.journal.head()?.0 {
                return Ok(None);
            }
            self.index.message_positions(id, before_sequence, limit)
        })
    }

    fn session_committed(&self) {
        if let Err(error) = self.advance_after_write() {
            // The append is durable. The outbox remains available for another catch-up.
            tracing::warn!(%error, "session catalog will retry after a committed append");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeConfig, RuntimeOpenOptions, terminal_actor, test_support::private_tempdir};
    use colossus_contracts::{ModelMessage, ModelMessageRole, SessionMessageAppend};
    use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
    use colossus_policy::DenyApproval;
    use colossus_ports::SessionRepository;
    use colossus_projection::default_handlers;
    use colossus_session::EventSourcedSessionRepository;

    #[test]
    fn legacy_catalog_backfills_after_open_and_tracks_committed_session_changes() {
        let workspace = private_tempdir();
        let path = workspace.path().join("state.redb");
        {
            let store = Arc::new(
                RedbEventJournal::open(
                    &path,
                    Arc::new(PlaintextKeyProvider),
                    Arc::new(DisabledCheckpointSigner),
                )
                .unwrap(),
            );
            let legacy = EventSourcedSessionRepository::new(store.clone());
            legacy
                .create_session("saved", None, terminal_actor())
                .unwrap();
            legacy
                .append_messages(
                    "saved",
                    "saved-run",
                    (1..=3)
                        .map(|sequence| SessionMessageAppend {
                            message: ModelMessage {
                                agent_message_origin: None,
                                role: ModelMessageRole::User,
                                content: format!("Saved message {sequence}").into(),
                                tool_call_id: None,
                                tool_calls: Vec::new(),
                            },
                            actor: terminal_actor(),
                        })
                        .collect(),
                )
                .unwrap();
            ProjectionWorker::new(store.clone(), store, default_handlers())
                .unwrap()
                .drain(64, 16)
                .unwrap();
        }
        let config = RuntimeConfig::offline_template(path);
        let open = || {
            Runtime::open_with_options(
                &config,
                Arc::new(DenyApproval),
                None,
                RuntimeOpenOptions::for_workspace(workspace.path()).unwrap(),
            )
            .unwrap()
        };
        let runtime = open();
        let catalog_position = |runtime: &Runtime| {
            runtime
                .projection_status()
                .unwrap()
                .into_iter()
                .find(|status| status.projection == SESSION_CATALOG_PROJECTION)
                .unwrap()
                .position
        };

        assert_eq!(catalog_position(&runtime), 0);
        let first = runtime.session_messages_page("saved", None, 1).unwrap();
        assert_eq!(first.before_sequence, Some(3));
        runtime.checkpoint().unwrap();
        assert_eq!(catalog_position(&runtime), 0);
        let latest = runtime.latest_session().unwrap();
        assert_eq!(latest.id, "saved");
        assert_eq!(latest.message_count, 3);
        assert_eq!(
            catalog_position(&runtime),
            runtime.journal().head().unwrap().0
        );

        runtime.set_session_archived("saved", true).unwrap();
        let position = catalog_position(&runtime);
        assert_eq!(position, runtime.journal().head().unwrap().0);
        assert!(runtime.list_sessions(10).unwrap().is_empty());
        assert_eq!(runtime.list_archived_sessions(10).unwrap(), [latest]);
        drop(runtime);

        let reopened = open();
        assert_eq!(catalog_position(&reopened), position);
        let older = reopened
            .session_messages_page("saved", first.before_sequence, 1)
            .unwrap();
        assert_eq!(older.before_sequence, Some(2));
        reopened.set_session_archived("saved", false).unwrap();
        assert_eq!(reopened.latest_session().unwrap().id, "saved");
        assert!(reopened.list_archived_sessions(10).unwrap().is_empty());
    }
}
