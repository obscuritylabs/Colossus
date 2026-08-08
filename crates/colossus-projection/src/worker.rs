use super::*;

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
        })
    }

    /// Apply up to `limit_per_projection` pending events for every handler.
    pub fn run_once(&self, limit_per_projection: usize) -> Result<ProjectionRunReport, StoreError> {
        let mut applied = 0_u64;
        let mut passive_batches = Vec::new();
        let mut passive_applied = 0_u64;
        for handler in &self.handlers {
            let position = self.store.position(handler.name())?;
            let work = self
                .journal
                .read_projection_work(position.saturating_add(1), limit_per_projection)?;
            let mut through_sequence = position;
            let mut events = Vec::with_capacity(work.len());
            for item in work {
                let expected_sequence = through_sequence.saturating_add(1);
                if item.global_sequence != expected_sequence {
                    return Err(StoreError::Verification(format!(
                        "projection {} expected outbox sequence {expected_sequence}, got {}",
                        handler.name(),
                        item.global_sequence
                    )));
                }
                let event = self
                    .journal
                    .read_global(item.global_sequence, 1)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "projection outbox sequence {} has no journal event",
                            item.global_sequence
                        ))
                    })?;
                if event.global_sequence != item.global_sequence || event.event_id != item.event_id
                {
                    return Err(StoreError::Verification(format!(
                        "projection outbox sequence {} does not match its journal event",
                        item.global_sequence
                    )));
                }
                through_sequence = item.global_sequence;
                events.push(event);
            }
            if events.is_empty() {
                continue;
            }
            if events.iter().all(|event| !handler.applies_to(event)) {
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
            for event in events {
                let mutations = if handler.applies_to(&event) {
                    let payload = if handler.requires_payload() {
                        self.journal.decrypt_payload(&event)?
                    } else {
                        Value::Null
                    };
                    handler.project(self.store.as_ref(), &event, &payload)?
                } else {
                    Vec::new()
                };
                self.store.apply(ProjectionBatch {
                    projection: handler.name().into(),
                    expected_position: projected_position,
                    through_sequence: event.global_sequence,
                    mutations,
                })?;
                projected_position = event.global_sequence;
                applied = applied.saturating_add(1);
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

    /// Replay bounded batches until every projection is current.
    pub fn drain(
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
            let report = self.run_once(batch_limit)?;
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
        if !self.handlers.iter().any(|handler| handler.name() == name) {
            return Err(StoreError::NotFound(format!("projection {name}")));
        }
        self.store.reset(name)?;
        self.drain(256, 16_384)
    }

    /// Delete and rebuild every registered projection.
    pub fn rebuild_all(&self) -> Result<ProjectionRunReport, StoreError> {
        for handler in &self.handlers {
            self.store.reset(handler.name())?;
        }
        self.drain(256, 16_384)
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
