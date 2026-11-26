use std::sync::Arc;

use crate::{Runtime, RuntimeError};
use colossus_contracts::SessionSummary;
use colossus_ports::{
    EventJournal, ProjectionStore, RunControl, SessionCatalog, SessionMessagePosition, StoreError,
};
use colossus_projection::{ProjectionWorker, SESSION_SUMMARY_PROJECTION};
use colossus_session::{IndexedSessionCatalog, SESSION_CATALOG_PROJECTION};

const BATCH: usize = 64;
const WRITE_BATCHES: usize = 16;
const WRITE_CATALOG_LAG: u64 = (BATCH * WRITE_BATCHES) as u64;
const MAX_TAIL_ROUNDS: usize = 65_536;
const MAX_READ_ATTEMPTS: usize = 3;

pub(super) fn required_for_open_and_checkpoint(name: &str) -> bool {
    name != SESSION_CATALOG_PROJECTION
}

struct CancelCatalogRead(RunControl);

impl Drop for CancelCatalogRead {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn check_cancelled(control: Option<&RunControl>) -> Result<(), StoreError> {
    if control.is_some_and(RunControl::is_cancelled) {
        return Err(StoreError::Adapter(
            "session catalog read was cancelled".into(),
        ));
    }
    Ok(())
}

impl Runtime {
    /// Read the terminal catalog without holding the terminal task during storage I/O.
    pub async fn terminal_session_catalog(
        self: &Arc<Self>,
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
        // Keep the runtime and its writer lease alive if the caller drops its wait.
        let runtime = Arc::clone(self);
        let control = RunControl::default();
        let _cancel_on_drop = CancelCatalogRead(control.clone());
        tokio::task::spawn_blocking(move || {
            runtime
                .session_catalog
                .read_recent(archived, limit.clamp(1, 100), Some(&control))
        })
        .await
        .map_err(|_| StoreError::Adapter("terminal session catalog task failed".into()))?
        .map_err(Into::into)
    }
}

pub(super) struct RuntimeSessionCatalog {
    index: IndexedSessionCatalog,
    journal: Arc<dyn EventJournal>,
    store: Arc<dyn ProjectionStore>,
    worker: Arc<ProjectionWorker>,
    opened_through: u64,
}

impl RuntimeSessionCatalog {
    pub(super) fn new(
        journal: Arc<dyn EventJournal>,
        store: Arc<dyn ProjectionStore>,
        worker: Arc<ProjectionWorker>,
    ) -> Result<Self, StoreError> {
        let opened_through = if journal.is_recovery_mode() {
            0
        } else {
            journal.head()?.0
        };
        Ok(Self {
            index: IndexedSessionCatalog::new(store.clone()),
            journal,
            store,
            worker,
            opened_through,
        })
    }

    fn position(&self) -> Result<u64, StoreError> {
        self.store.position(SESSION_CATALOG_PROJECTION)
    }

    fn current_recent(
        &self,
        through: u64,
        archived: bool,
        limit: usize,
        control: Option<&RunControl>,
    ) -> Result<Option<Vec<SessionSummary>>, StoreError> {
        self.worker.read_without_reset(|| {
            if self.position()? >= through {
                return self.index.recent(archived, limit).map(Some);
            }
            if self.store.position(SESSION_SUMMARY_PROJECTION)? != through {
                return Ok(None);
            }
            let sessions = self.index.recent_from_projected_summaries(
                self.journal.as_ref(),
                SESSION_SUMMARY_PROJECTION,
                archived,
                limit,
                control,
            )?;
            // Paging and canonical lifecycle reads use separate storage operations.
            // Retry if an append or projection advance could have mixed snapshots.
            let stable = self.journal.head()?.0 == through
                && self.store.position(SESSION_SUMMARY_PROJECTION)? == through;
            Ok(stable.then_some(sessions))
        })
    }

    fn advance_committed_tail(
        &self,
        through: u64,
        rounds: usize,
        max_existing_catalog_tail: u64,
        control: Option<&RunControl>,
    ) -> Result<(), StoreError> {
        let catalog_floor = if self.opened_through == 0 {
            0
        } else {
            self.opened_through
                .saturating_sub(max_existing_catalog_tail)
                .max(1)
        };
        let (projection, floor) = if self.position()? >= catalog_floor {
            (SESSION_CATALOG_PROJECTION, catalog_floor)
        } else {
            (SESSION_SUMMARY_PROJECTION, self.opened_through)
        };
        for _ in 0..rounds {
            check_cancelled(control)?;
            let position = self.store.position(projection)?;
            // Discovery only advances events committed since open. After a write,
            // a populated catalog can also close a small carried-over tail.
            if position < floor || position >= through {
                break;
            }
            if self
                .worker
                .drain_selected(BATCH, 1, |name| name == projection)?
                .applied
                == 0
            {
                break;
            }
        }
        Ok(())
    }
    fn read_recent(
        &self,
        archived: bool,
        limit: usize,
        control: Option<&RunControl>,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        for _ in 0..MAX_READ_ATTEMPTS {
            check_cancelled(control)?;
            let through = self.journal.head()?.0;
            if let Some(sessions) = self.current_recent(through, archived, limit, control)? {
                return Ok(sessions);
            }
            self.advance_committed_tail(through, MAX_TAIL_ROUNDS, 0, control)?;
            if let Some(sessions) = self.current_recent(through, archived, limit, control)? {
                return Ok(sessions);
            }
        }
        Err(StoreError::Adapter(
            "session catalog is still rebuilding".into(),
        ))
    }
}

impl SessionCatalog for RuntimeSessionCatalog {
    fn recent(&self, archived: bool, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        self.read_recent(archived, limit, None)
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
        let advance = || {
            self.advance_committed_tail(
                self.journal.head()?.0,
                WRITE_BATCHES,
                WRITE_CATALOG_LAG,
                None,
            )
        };
        if let Err(error) = advance() {
            // The append is durable. The outbox remains available for another catch-up.
            tracing::warn!(%error, "session catalog will retry after a committed append");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeConfig, RuntimeOpenOptions, terminal_actor, test_support::private_tempdir};
    use colossus_contracts::{
        EventClassification, ExecutionContext, ModelMessage, ModelMessageRole, NewEvent,
        SessionMessageAppend,
    };
    use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
    use colossus_policy::DenyApproval;
    use colossus_ports::SessionRepository;
    use colossus_projection::default_handlers;
    use colossus_session::EventSourcedSessionRepository;

    fn messages(count: u64) -> Vec<SessionMessageAppend> {
        (1..=count)
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
            .collect()
    }

    #[tokio::test]
    async fn legacy_discovery_uses_summaries_until_background_catalog_catches_up() {
        let message_count = (BATCH * (WRITE_BATCHES + 2)) as u64;
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
                .append_messages("saved", "saved-run", messages(message_count))
                .unwrap();
            ProjectionWorker::new(store.clone(), store, default_handlers())
                .unwrap()
                .drain(64, 32)
                .unwrap();
        }
        let config = RuntimeConfig::offline_template(path);
        let open = || {
            Arc::new(
                Runtime::open_with_options(
                    &config,
                    Arc::new(DenyApproval),
                    None,
                    RuntimeOpenOptions::for_workspace(workspace.path()).unwrap(),
                )
                .unwrap(),
            )
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
        assert_eq!(first.before_sequence, Some(message_count));
        runtime.checkpoint().unwrap();
        assert_eq!(catalog_position(&runtime), 0);
        let latest = runtime.latest_session().unwrap();
        assert_eq!(latest.id, "saved");
        assert_eq!(latest.message_count, message_count);
        assert!(latest.last_user_preview.is_none());
        assert_eq!(
            runtime
                .terminal_session_catalog(false, 10)
                .await
                .unwrap()
                .as_slice(),
            std::slice::from_ref(&latest)
        );
        assert_eq!(catalog_position(&runtime), 0);

        runtime
            .projections
            .drain_selected(BATCH, 1, |name| name == SESSION_CATALOG_PROJECTION)
            .unwrap();
        let partial = catalog_position(&runtime);
        assert_eq!(partial, BATCH as u64);
        assert_eq!(runtime.latest_session().unwrap(), latest);
        assert_eq!(catalog_position(&runtime), partial);

        runtime.set_session_archived("saved", true).unwrap();
        assert_eq!(
            runtime
                .projection_status()
                .unwrap()
                .into_iter()
                .find(|status| status.projection == SESSION_SUMMARY_PROJECTION)
                .unwrap()
                .position,
            runtime.journal().head().unwrap().0,
        );
        assert!(runtime.list_sessions(10).unwrap().is_empty());
        assert_eq!(
            runtime
                .terminal_session_catalog(true, 10)
                .await
                .unwrap()
                .as_slice(),
            std::slice::from_ref(&latest)
        );
        assert_eq!(catalog_position(&runtime), partial);

        runtime
            .projections
            .drain_selected(BATCH, 2, |name| name == SESSION_CATALOG_PROJECTION)
            .unwrap();
        let nearly_ready = catalog_position(&runtime);
        assert!(runtime.journal().head().unwrap().0 - nearly_ready < WRITE_CATALOG_LAG);
        assert_eq!(
            runtime.list_archived_sessions(10).unwrap().as_slice(),
            std::slice::from_ref(&latest)
        );
        assert_eq!(catalog_position(&runtime), nearly_ready);

        runtime.drain_projections().unwrap();
        let position = catalog_position(&runtime);
        assert_eq!(position, runtime.journal().head().unwrap().0);
        let mut indexed = latest;
        indexed.last_user_preview = Some(format!("Saved message {message_count}"));
        assert_eq!(runtime.list_archived_sessions(10).unwrap(), [indexed]);
        drop(runtime);

        let reopened = open();
        assert_eq!(catalog_position(&reopened), position);
        let older = reopened
            .session_messages_page("saved", first.before_sequence, 1)
            .unwrap();
        assert_eq!(older.before_sequence, Some(message_count - 1));
        reopened.set_session_archived("saved", false).unwrap();
        let restored = reopened.latest_session().unwrap();
        assert_eq!(restored.id, "saved");
        assert!(reopened.list_archived_sessions(10).unwrap().is_empty());

        let indexed_through = catalog_position(&reopened);
        reopened
            .journal()
            .append(NewEvent {
                event_version: 1,
                stream_id: "test:catalog-non-session".into(),
                expected_stream_version: 0,
                classification: EventClassification::Domain,
                event_type: "test.non_session.v1".into(),
                actor: terminal_actor(),
                context: ExecutionContext::default(),
                payload: serde_json::json!({}),
            })
            .unwrap();
        reopened.checkpoint().unwrap();
        drop(reopened);

        let reopened = open();
        assert!(reopened.journal().head().unwrap().0 > indexed_through);
        assert_eq!(catalog_position(&reopened), indexed_through);
        assert_eq!(reopened.latest_session().unwrap(), restored);
        assert_eq!(catalog_position(&reopened), indexed_through);

        let mut fresh = messages(1);
        fresh[0].message.content = "New after reopening".into();
        reopened
            .session_repository()
            .append_messages("saved", "resumed-run", fresh)
            .unwrap();
        assert_eq!(
            catalog_position(&reopened),
            reopened.journal().head().unwrap().0
        );
        let updated = reopened.latest_session().unwrap();
        assert_eq!(updated.message_count, message_count + 1);
        assert_eq!(
            updated.last_user_preview.as_deref(),
            Some("New after reopening")
        );
    }

    #[test]
    fn discovery_covers_a_large_append_to_an_open_journal() {
        let workspace = private_tempdir();
        let runtime = Runtime::open_with_options(
            &RuntimeConfig::offline_template(workspace.path().join("state.redb")),
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(workspace.path()).unwrap(),
        )
        .unwrap();
        let repository = runtime.session_repository();
        repository
            .create_session("bulk", None, terminal_actor())
            .unwrap();
        let count = (BATCH * WRITE_BATCHES + 1) as u64;
        repository
            .append_messages("bulk", "bulk-run", messages(count))
            .unwrap();

        let position = runtime.session_catalog.position().unwrap();
        assert!(position < runtime.journal().head().unwrap().0);
        let control = RunControl::default();
        drop(CancelCatalogRead(control.clone()));
        assert!(
            runtime
                .session_catalog
                .read_recent(false, 1, Some(&control))
                .is_err()
        );
        assert_eq!(runtime.session_catalog.position().unwrap(), position);

        let latest = runtime.latest_session().unwrap();
        assert_eq!(latest.id, "bulk");
        assert_eq!(latest.message_count, count);
        assert_eq!(
            latest.last_user_preview,
            Some(format!("Saved message {count}"))
        );
        assert!(
            runtime
                .projection_status()
                .unwrap()
                .iter()
                .any(|status| status.projection == SESSION_CATALOG_PROJECTION && status.ready)
        );
    }
}
