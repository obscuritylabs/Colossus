use super::*;
use std::{collections::BTreeMap, sync::Mutex};

/// Pure event-to-projection reducer.
pub trait ProjectionHandler: Send + Sync {
    /// Stable name containing a schema version.
    fn name(&self) -> &'static str;

    /// Return whether this event can change the projection.
    fn applies_to(&self, _event: &EventEnvelope) -> bool {
        true
    }

    /// Return whether projection logic needs the decrypted event payload.
    fn requires_payload(&self) -> bool {
        true
    }

    /// Produce record mutations for one journal event.
    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError>;
}

/// Result of one bounded worker or drain operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRunReport {
    /// Number of event-handler applications committed.
    pub applied: u64,
    /// Current state of every registered projection.
    pub projections: Vec<ProjectionStatus>,
}

/// Replays journal outbox entries into disposable projections.
pub struct ProjectionWorker {
    journal: Arc<dyn EventJournal>,
    store: Arc<dyn ProjectionStore>,
    handlers: Vec<Arc<dyn ProjectionHandler>>,
    operation_gate: Mutex<()>,
}

impl ProjectionWorker {
    /// Build a worker with an explicit journal, store, and handler set.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        store: Arc<dyn ProjectionStore>,
        handlers: Vec<Arc<dyn ProjectionHandler>>,
    ) -> Result<Self, StoreError> {
        let mut names = handlers
            .iter()
            .map(|handler| handler.name())
            .collect::<Vec<_>>();
        names.sort_unstable();
        if names.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(StoreError::Adapter(
                "projection handler names must be unique".into(),
            ));
        }
        Ok(Self {
            journal,
            store,
            handlers,
            operation_gate: Mutex::new(()),
        })
    }

    /// Apply up to `limit_per_projection` pending events for every handler.
    pub fn run_once(&self, limit_per_projection: usize) -> Result<ProjectionRunReport, StoreError> {
        let _operation = self.operation_gate.lock().map_err(|_| {
            StoreError::Adapter("projection worker operation lock was poisoned".into())
        })?;
        self.run_once_inner(limit_per_projection)
    }

    fn run_once_inner(
        &self,
        limit_per_projection: usize,
    ) -> Result<ProjectionRunReport, StoreError> {
        let mut applied = 0_u64;
        let mut passive_batches = Vec::new();
        let mut passive_applied = 0_u64;
        let mut pages = BTreeMap::new();
        for handler in &self.handlers {
            let position = self.store.position(handler.name())?;
            let from_sequence = position.saturating_add(1);
            let page = match pages.get(&from_sequence) {
                Some(page) => Arc::clone(page),
                None => {
                    let page = Arc::new(self.read_page(
                        handler.name(),
                        from_sequence,
                        limit_per_projection,
                    )?);
                    pages.insert(from_sequence, Arc::clone(&page));
                    page
                }
            };
            let Some((_, last_event)) = page.last() else {
                continue;
            };
            let through_sequence = last_event.global_sequence;
            if page.iter().all(|(_, event)| !handler.applies_to(event)) {
                passive_batches.push(ProjectionBatch {
                    projection: handler.name().into(),
                    expected_position: position,
                    through_sequence,
                    mutations: Vec::new(),
                });
                passive_applied =
                    passive_applied.saturating_add(through_sequence.saturating_sub(position));
                continue;
            }
            if !passive_batches.is_empty() {
                self.store.apply_all(&passive_batches)?;
                applied = applied.saturating_add(passive_applied);
                passive_batches.clear();
                passive_applied = 0;
            }

            let mut projected_position = position;
            let mut passive_through = None;
            for (_, event) in page.iter() {
                if !handler.applies_to(event) {
                    passive_through = Some(event.global_sequence);
                    continue;
                }
                if let Some(through_sequence) = passive_through.take() {
                    self.apply_passive_span(
                        handler.name(),
                        &mut projected_position,
                        through_sequence,
                        &mut applied,
                    )?;
                }
                let payload = if handler.requires_payload() {
                    self.journal.decrypt_payload(event)?
                } else {
                    Value::Null
                };
                let mutations = handler.project(self.store.as_ref(), event, &payload)?;
                self.store.apply(ProjectionBatch {
                    projection: handler.name().into(),
                    expected_position: projected_position,
                    through_sequence: event.global_sequence,
                    mutations,
                })?;
                projected_position = event.global_sequence;
                applied = applied.saturating_add(1);
            }
            if let Some(through_sequence) = passive_through {
                self.apply_passive_span(
                    handler.name(),
                    &mut projected_position,
                    through_sequence,
                    &mut applied,
                )?;
            }
        }
        if !passive_batches.is_empty() {
            self.store.apply_all(&passive_batches)?;
            applied = applied.saturating_add(passive_applied);
        }
        Ok(ProjectionRunReport {
            applied,
            projections: self.status()?,
        })
    }

    fn read_page(
        &self,
        projection: &str,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<(ProjectionWorkItem, EventEnvelope)>, StoreError> {
        let work = self.journal.read_projection_work(from_sequence, limit)?;
        let mut expected_sequence = from_sequence;
        for item in &work {
            if item.global_sequence != expected_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {projection} expected outbox sequence {expected_sequence}, got {}",
                    item.global_sequence
                )));
            }
            expected_sequence = expected_sequence.saturating_add(1);
        }
        let events = self.journal.read_global(from_sequence, work.len())?;
        if events.len() != work.len() {
            let missing_sequence = work
                .get(events.len())
                .map_or(expected_sequence, |item| item.global_sequence);
            return Err(StoreError::Verification(format!(
                "projection outbox sequence {missing_sequence} has no journal event"
            )));
        }
        work.into_iter()
            .zip(events)
            .map(|(item, event)| {
                if event.global_sequence != item.global_sequence || event.event_id != item.event_id
                {
                    return Err(StoreError::Verification(format!(
                        "projection outbox sequence {} does not match its journal event",
                        item.global_sequence
                    )));
                }
                Ok((item, event))
            })
            .collect()
    }

    fn apply_passive_span(
        &self,
        projection: &str,
        projected_position: &mut u64,
        through_sequence: u64,
        applied: &mut u64,
    ) -> Result<(), StoreError> {
        self.store.apply(ProjectionBatch {
            projection: projection.into(),
            expected_position: *projected_position,
            through_sequence,
            mutations: Vec::new(),
        })?;
        *applied = applied.saturating_add(through_sequence.saturating_sub(*projected_position));
        *projected_position = through_sequence;
        Ok(())
    }

    /// Replay bounded batches until every projection is current.
    pub fn drain(
        &self,
        batch_limit: usize,
        max_rounds: usize,
    ) -> Result<ProjectionRunReport, StoreError> {
        let _operation = self.operation_gate.lock().map_err(|_| {
            StoreError::Adapter("projection worker operation lock was poisoned".into())
        })?;
        self.drain_inner(batch_limit, max_rounds)
    }

    fn drain_inner(
        &self,
        batch_limit: usize,
        max_rounds: usize,
    ) -> Result<ProjectionRunReport, StoreError> {
        if batch_limit == 0 || max_rounds == 0 {
            return Err(StoreError::Adapter(
                "projection drain bounds must be greater than zero".into(),
            ));
        }
        let mut applied = 0_u64;
        for _ in 0..max_rounds {
            let report = self.run_once_inner(batch_limit)?;
            applied = applied.saturating_add(report.applied);
            if report.projections.iter().all(|status| status.ready) {
                return Ok(ProjectionRunReport {
                    applied,
                    projections: report.projections,
                });
            }
            if report.applied == 0 {
                break;
            }
        }
        Ok(ProjectionRunReport {
            applied,
            projections: self.status()?,
        })
    }

    /// Delete one named projection and rebuild it from sequence one.
    pub fn rebuild(&self, name: &str) -> Result<ProjectionRunReport, StoreError> {
        let _operation = self.operation_gate.lock().map_err(|_| {
            StoreError::Adapter("projection worker operation lock was poisoned".into())
        })?;
        if !self.handlers.iter().any(|handler| handler.name() == name) {
            return Err(StoreError::NotFound(format!("projection {name}")));
        }
        self.store.reset(name)?;
        self.drain_inner(256, 16_384)
    }

    /// Delete and rebuild every registered projection.
    pub fn rebuild_all(&self) -> Result<ProjectionRunReport, StoreError> {
        let _operation = self.operation_gate.lock().map_err(|_| {
            StoreError::Adapter("projection worker operation lock was poisoned".into())
        })?;
        for handler in &self.handlers {
            self.store.reset(handler.name())?;
        }
        self.drain_inner(256, 16_384)
    }

    /// Report current journal head, position, lag, and readiness.
    pub fn status(&self) -> Result<Vec<ProjectionStatus>, StoreError> {
        let (head, _) = self.journal.head()?;
        self.handlers
            .iter()
            .map(|handler| {
                let position = self.store.position(handler.name())?;
                Ok(ProjectionStatus {
                    projection: handler.name().into(),
                    position,
                    journal_head: head,
                    lag: head.saturating_sub(position),
                    ready: !self.journal.is_recovery_mode() && position == head,
                })
            })
            .collect()
    }
}
