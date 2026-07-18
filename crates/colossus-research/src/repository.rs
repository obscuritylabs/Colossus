use super::*;

const RUN_CREATED: &str = "research.run_created.v1";
const RUN_UPDATED: &str = "research.run_updated.v1";
const SOURCE_ADDED: &str = "research.source_added.v1";
const CLAIM_ADDED: &str = "research.claim_added.v1";
const MAX_ID_BYTES: usize = 128;
pub(super) const MAX_QUESTION_BYTES: usize = 64 * 1024;
pub(super) const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_SOURCE_CONTENT_BYTES: usize = 256 * 1024;
pub(super) const MAX_REPORT_BYTES: usize = 512 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_QUERIES: usize = 100;
const MAX_LANES: usize = 300;
const MAX_PROGRESS: usize = 2_000;
const MAX_SOURCES: usize = 100;
const MAX_CLAIMS: usize = 1_000;
pub(super) const MAX_LIST: usize = 1_000;

pub(super) fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_run(run: &ResearchRun) -> Result<(), StoreError> {
    let kinds = run.source_kinds.iter().copied().collect::<BTreeSet<_>>();
    let query_set = run.queries.iter().collect::<BTreeSet<_>>();
    let lane_ids = run
        .lanes
        .iter()
        .map(|lane| &lane.id)
        .collect::<BTreeSet<_>>();
    let lanes_valid = run.lanes.iter().all(|lane| {
        valid_id(&lane.id)
            && !lane.query.trim().is_empty()
            && lane.query.len() <= MAX_QUERY_BYTES
            && kinds.contains(&lane.kind)
            && query_set.contains(&lane.query)
            && lane.message.len() <= MAX_QUERY_BYTES
            && lane.source_count <= MAX_SOURCES
            && !lane.updated_at.is_empty()
            && (lane.status == ResearchLaneStatus::Completed || lane.source_count == 0)
    });
    let progress_ids = run
        .progress
        .iter()
        .map(|progress| &progress.id)
        .collect::<BTreeSet<_>>();
    let progress_valid = run.progress.iter().all(|progress| {
        valid_id(&progress.id)
            && !progress.action.trim().is_empty()
            && progress.action.len() <= MAX_QUERY_BYTES
            && progress.message.len() <= MAX_QUERY_BYTES
            && progress.created_at.len() <= 128
            && !progress.created_at.is_empty()
            && progress
                .current
                .zip(progress.total)
                .is_none_or(|(current, total)| {
                    total <= MAX_PROGRESS && current >= 1 && current <= total
                })
    });
    let lifecycle_valid = match run.status {
        ResearchStatus::Running => {
            run.completed_at.is_none() && run.report.is_empty() && run.error.is_empty()
        }
        ResearchStatus::Completed => {
            run.completed_at.is_some() && !run.report.trim().is_empty() && run.error.is_empty()
        }
        ResearchStatus::Failed | ResearchStatus::Interrupted => {
            run.completed_at.is_some() && !run.error.trim().is_empty()
        }
    };
    if !valid_id(&run.id)
        || !valid_id(&run.session_id)
        || run.question.trim().is_empty()
        || run.question.len() > MAX_QUESTION_BYTES
        || run.source_kinds.is_empty()
        || kinds.len() != run.source_kinds.len()
        || run.queries.len() > MAX_QUERIES
        || run
            .queries
            .iter()
            .any(|query| query.trim().is_empty() || query.len() > MAX_QUERY_BYTES)
        || run.lanes.len() > MAX_LANES
        || lane_ids.len() != run.lanes.len()
        || !lanes_valid
        || run.progress.len() > MAX_PROGRESS
        || progress_ids.len() != run.progress.len()
        || !progress_valid
        || run
            .limitations
            .iter()
            .any(|item| item.len() > MAX_QUERY_BYTES)
        || run.report.len() > MAX_REPORT_BYTES
        || run.error.len() > MAX_QUERY_BYTES
        || run.created_at.is_empty()
        || run.updated_at.is_empty()
        || !lifecycle_valid
    {
        return Err(StoreError::Adapter(
            "invalid research identity, bounds, lanes, lifecycle, or timestamps".into(),
        ));
    }
    Ok(())
}

fn validate_source(source: &ResearchSource) -> Result<(), StoreError> {
    let metadata_bytes = serde_json::to_vec(&source.metadata).map_err(adapter)?.len();
    if !valid_id(&source.id)
        || !valid_id(&source.run_id)
        || !valid_label(&source.label)
        || source.title.trim().is_empty()
        || source.title.len() > MAX_QUERY_BYTES
        || source.uri.len() > MAX_QUERY_BYTES
        || source.content.len() > MAX_SOURCE_CONTENT_BYTES
        || source.query.len() > MAX_QUERY_BYTES
        || metadata_bytes > MAX_METADATA_BYTES
        || source.created_at.is_empty()
    {
        return Err(StoreError::Adapter(
            "invalid research source identity, label, content, metadata, or timestamp".into(),
        ));
    }
    Ok(())
}

fn validate_claim(claim: &ResearchClaim) -> Result<(), StoreError> {
    let labels = claim.source_labels.iter().collect::<BTreeSet<_>>();
    if !valid_id(&claim.id)
        || !valid_id(&claim.run_id)
        || claim.text.trim().is_empty()
        || claim.text.len() > MAX_QUESTION_BYTES
        || claim.source_labels.is_empty()
        || labels.len() != claim.source_labels.len()
        || claim.source_labels.iter().any(|label| !valid_label(label))
        || claim.created_at.is_empty()
    {
        return Err(StoreError::Adapter(
            "invalid research claim identity, text, labels, or timestamp".into(),
        ));
    }
    Ok(())
}

fn valid_label(label: &str) -> bool {
    label.strip_prefix('R').is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Immutable-journal implementation of the research lifecycle port.
pub struct EventSourcedResearchRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedResearchRepository {
    /// Bind canonical research streams to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(id: &str) -> String {
        format!("research:{id}")
    }

    fn event(
        run_id: &str,
        session_id: &str,
        expected_stream_version: u64,
        event_type: &str,
        actor: Actor,
        payload: Value,
    ) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: Self::stream(run_id),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("research:{run_id}"),
                session_id: Some(session_id.into()),
                run_id: Some(run_id.into()),
                ..ExecutionContext::default()
            },
            payload,
        }
    }

    fn events(&self, run_id: &str) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        self.journal.read_stream(&Self::stream(run_id))
    }

    fn expected_version(&self, run_id: &str) -> Result<u64, StoreError> {
        u64::try_from(self.events(run_id)?.len()).map_err(adapter)
    }

    fn run_ids(&self) -> Result<BTreeSet<String>, StoreError> {
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if event.event_type == RUN_CREATED
                    && let Some(id) = event.stream_id.strip_prefix("research:")
                {
                    ids.insert(id.into());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < 1_024 {
                break;
            }
        }
        Ok(ids)
    }

    fn record<T: serde::de::DeserializeOwned>(
        &self,
        event: &colossus_contracts::EventEnvelope,
        field: &str,
    ) -> Result<T, StoreError> {
        let payload = self.journal.decrypt_payload(event)?;
        serde_json::from_value(
            payload
                .get(field)
                .cloned()
                .ok_or_else(|| StoreError::Verification(format!("research {field} is absent")))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))
    }
}

impl ResearchRepository for EventSourcedResearchRepository {
    fn create_run(&self, run: ResearchRun, actor: Actor) -> Result<ResearchRun, StoreError> {
        validate_run(&run)?;
        if run.status != ResearchStatus::Running
            || !run.queries.is_empty()
            || !run.lanes.is_empty()
            || !run.progress.is_empty()
        {
            return Err(StoreError::Adapter(
                "new research runs must begin running before planning".into(),
            ));
        }
        self.journal.append(Self::event(
            &run.id,
            &run.session_id,
            0,
            RUN_CREATED,
            actor,
            json!({"run": &run}),
        ))?;
        Ok(run)
    }

    fn update_run(&self, run: ResearchRun, actor: Actor) -> Result<ResearchRun, StoreError> {
        validate_run(&run)?;
        let current = self
            .get_run(&run.id)?
            .ok_or_else(|| StoreError::NotFound(format!("research run {}", run.id)))?;
        if current.status != ResearchStatus::Running
            || current.session_id != run.session_id
            || current.question != run.question
            || current.depth != run.depth
            || current.source_kinds != run.source_kinds
            || current.created_at != run.created_at
            || !run.queries.starts_with(&current.queries)
            || !run.lanes.starts_with(&current.lanes)
            || !run.progress.starts_with(&current.progress)
            || !run.limitations.starts_with(&current.limitations)
        {
            return Err(StoreError::Adapter(
                "research provenance is immutable and terminal runs cannot be changed".into(),
            ));
        }
        if run.status == ResearchStatus::Completed {
            let sources = self.list_sources(&run.id)?;
            let labels = sources
                .iter()
                .map(|source| source.label.as_str())
                .collect::<BTreeSet<_>>();
            let claims = self.list_claims(&run.id)?;
            if claims.iter().any(|claim| {
                claim
                    .source_labels
                    .iter()
                    .any(|label| !labels.contains(label.as_str()))
            }) {
                return Err(StoreError::Verification(
                    "research report cannot complete with dangling claim citations".into(),
                ));
            }
            let cited = citation_labels(&run.report);
            if cited.iter().any(|label| !labels.contains(label.as_str()))
                || (!sources.is_empty() && cited.is_empty())
            {
                return Err(StoreError::Adapter(
                    "research report citations must resolve to canonical sources".into(),
                ));
            }
        }
        let expected = self.expected_version(&run.id)?;
        self.journal.append(Self::event(
            &run.id,
            &run.session_id,
            expected,
            RUN_UPDATED,
            actor,
            json!({"run": &run}),
        ))?;
        Ok(run)
    }

    fn get_run(&self, id: &str) -> Result<Option<ResearchRun>, StoreError> {
        let events = self.events(id)?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        if first.event_type != RUN_CREATED {
            return Err(StoreError::Verification(
                "research stream has no valid creation event".into(),
            ));
        }
        let event = events
            .iter()
            .rev()
            .find(|event| matches!(event.event_type.as_str(), RUN_CREATED | RUN_UPDATED))
            .ok_or_else(|| StoreError::Verification("research run record disappeared".into()))?;
        let run: ResearchRun = self.record(event, "run")?;
        validate_run(&run).map_err(|error| StoreError::Verification(error.to_string()))?;
        Ok(Some(run))
    }

    fn list_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResearchRun>, StoreError> {
        let mut runs = self
            .run_ids()?
            .into_iter()
            .filter_map(|id| self.get_run(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        runs.retain(|run| session_id.is_none_or(|id| run.session_id == id));
        runs.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        runs.truncate(limit.clamp(1, MAX_LIST));
        Ok(runs)
    }

    fn add_source(
        &self,
        source: ResearchSource,
        actor: Actor,
    ) -> Result<ResearchSource, StoreError> {
        validate_source(&source)?;
        let run = self
            .get_run(&source.run_id)?
            .ok_or_else(|| StoreError::NotFound(format!("research run {}", source.run_id)))?;
        if run.status != ResearchStatus::Running || !run.source_kinds.contains(&source.kind) {
            return Err(StoreError::Adapter(
                "sources require a running run and a declared evidence kind".into(),
            ));
        }
        let sources = self.list_sources(&source.run_id)?;
        if sources.len() >= MAX_SOURCES {
            return Err(StoreError::Adapter("research source limit reached".into()));
        }
        let expected_label = format!("R{}", sources.len().saturating_add(1));
        if source.label != expected_label
            || sources.iter().any(|existing| {
                existing.id == source.id || existing.uri == source.uri && !source.uri.is_empty()
            })
        {
            return Err(StoreError::Adapter(
                "source labels must be sequential and source identity/URI must be unique".into(),
            ));
        }
        let expected = self.expected_version(&source.run_id)?;
        self.journal.append(Self::event(
            &source.run_id,
            &run.session_id,
            expected,
            SOURCE_ADDED,
            actor,
            json!({"source": &source}),
        ))?;
        Ok(source)
    }

    fn list_sources(&self, run_id: &str) -> Result<Vec<ResearchSource>, StoreError> {
        self.events(run_id)?
            .iter()
            .filter(|event| event.event_type == SOURCE_ADDED)
            .map(|event| self.record(event, "source"))
            .collect()
    }

    fn add_claim(&self, claim: ResearchClaim, actor: Actor) -> Result<ResearchClaim, StoreError> {
        validate_claim(&claim)?;
        let run = self
            .get_run(&claim.run_id)?
            .ok_or_else(|| StoreError::NotFound(format!("research run {}", claim.run_id)))?;
        if run.status != ResearchStatus::Running {
            return Err(StoreError::Adapter(
                "claims require a running research run".into(),
            ));
        }
        let claims = self.list_claims(&claim.run_id)?;
        if claims.len() >= MAX_CLAIMS || claims.iter().any(|existing| existing.id == claim.id) {
            return Err(StoreError::Adapter(
                "research claim limit or identity violated".into(),
            ));
        }
        let labels = self
            .list_sources(&claim.run_id)?
            .into_iter()
            .map(|source| source.label)
            .collect::<BTreeSet<_>>();
        if claim
            .source_labels
            .iter()
            .any(|label| !labels.contains(label))
        {
            return Err(StoreError::Adapter(
                "claim citations must resolve to canonical sources".into(),
            ));
        }
        let expected = self.expected_version(&claim.run_id)?;
        self.journal.append(Self::event(
            &claim.run_id,
            &run.session_id,
            expected,
            CLAIM_ADDED,
            actor,
            json!({"claim": &claim}),
        ))?;
        Ok(claim)
    }

    fn list_claims(&self, run_id: &str) -> Result<Vec<ResearchClaim>, StoreError> {
        self.events(run_id)?
            .iter()
            .filter(|event| event.event_type == CLAIM_ADDED)
            .map(|event| self.record(event, "claim"))
            .collect()
    }
}

pub(super) fn citation_labels(report: &str) -> BTreeSet<String> {
    let bytes = report.as_bytes();
    let mut labels = BTreeSet::new();
    let mut index = 0;
    while index + 3 < bytes.len() {
        if bytes[index] == b'[' && bytes[index + 1] == b'R' {
            let start = index + 1;
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > index + 2 && end < bytes.len() && bytes[end] == b']' {
                labels.insert(report[start..end].into());
                index = end;
            }
        }
        index += 1;
    }
    labels
}
