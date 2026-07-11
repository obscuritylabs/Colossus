//! Canonical event-sourced research runs, sources, claims, and cited reports.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, ModelMessage, ModelMessageRole,
    NewEvent, ResearchClaim, ResearchDepth, ResearchLane, ResearchLaneStatus, ResearchRun,
    ResearchSource, ResearchSourceKind, ResearchStatus,
};
use colossus_ports::{EventJournal, ResearchRepository, SessionRepository, StoreError};
use serde_json::{Value, json};
use std::{collections::BTreeMap, collections::BTreeSet, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const RUN_CREATED: &str = "research.run_created.v1";
const RUN_UPDATED: &str = "research.run_updated.v1";
const SOURCE_ADDED: &str = "research.source_added.v1";
const CLAIM_ADDED: &str = "research.claim_added.v1";
const MAX_ID_BYTES: usize = 128;
const MAX_QUESTION_BYTES: usize = 64 * 1024;
const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_SOURCE_CONTENT_BYTES: usize = 256 * 1024;
const MAX_REPORT_BYTES: usize = 512 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_QUERIES: usize = 100;
const MAX_LANES: usize = 300;
const MAX_SOURCES: usize = 100;
const MAX_CLAIMS: usize = 1_000;
const MAX_LIST: usize = 1_000;

fn adapter(error: impl std::fmt::Display) -> StoreError {
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
    let lifecycle_valid = match run.status {
        ResearchStatus::Running => {
            run.completed_at.is_none() && run.report.is_empty() && run.error.is_empty()
        }
        ResearchStatus::Completed => {
            run.completed_at.is_some() && !run.report.trim().is_empty() && run.error.is_empty()
        }
        ResearchStatus::Failed => run.completed_at.is_some() && !run.error.trim().is_empty(),
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
        if run.status != ResearchStatus::Running || !run.queries.is_empty() || !run.lanes.is_empty()
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

fn citation_labels(report: &str) -> BTreeSet<String> {
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

/// Bounded uncommitted evidence returned by a policy-bound collector adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchSourceDraft {
    /// Evidence lane that produced the draft.
    pub kind: ResearchSourceKind,
    /// Human-readable title.
    pub title: String,
    /// Repository path or external URI.
    pub uri: String,
    /// Released bounded evidence content.
    pub content: String,
    /// Bounded non-secret metadata.
    pub metadata: BTreeMap<String, String>,
}

/// Known collector outcome. Denial and unavailability become limitations, not lost work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchCollection {
    /// Durable lane outcome.
    pub status: ResearchLaneStatus,
    /// Bounded human-readable outcome or limitation.
    pub message: String,
    /// Released candidate evidence, ignored unless the status is completed.
    pub sources: Vec<ResearchSourceDraft>,
}

/// Replaceable evidence collector. Implementations must cross the effect gateway themselves.
#[async_trait]
pub trait ResearchCollector: Send + Sync {
    /// Collect bounded released evidence for one query and lane.
    async fn collect(
        &self,
        run: &ResearchRun,
        kind: ResearchSourceKind,
        query: &str,
        limit: usize,
    ) -> ResearchCollection;
}

/// Bounds for one durable research orchestration run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResearchLimits {
    /// Canonical source ceiling.
    pub max_sources: usize,
    /// Maximum independent query/lane collection jobs.
    pub max_workers: usize,
}

impl Default for ResearchLimits {
    fn default() -> Self {
        Self {
            max_sources: 20,
            max_workers: 4,
        }
    }
}

/// Durable four-phase research orchestration over replaceable collectors.
pub struct ResearchService {
    repository: Arc<dyn ResearchRepository>,
    sessions: Arc<dyn SessionRepository>,
    collector: Arc<dyn ResearchCollector>,
    limits: ResearchLimits,
}

impl ResearchService {
    /// Compose canonical state with a gateway-bound collector.
    pub fn new(
        repository: Arc<dyn ResearchRepository>,
        sessions: Arc<dyn SessionRepository>,
        collector: Arc<dyn ResearchCollector>,
        limits: ResearchLimits,
    ) -> Result<Self, StoreError> {
        if !(1..=100).contains(&limits.max_sources) || !(1..=16).contains(&limits.max_workers) {
            return Err(StoreError::Adapter(
                "research limits require max_sources 1..=100 and max_workers 1..=16".into(),
            ));
        }
        Ok(Self {
            repository,
            sessions,
            collector,
            limits,
        })
    }

    /// Execute planning, collection, claim extraction, and cited synthesis durably.
    pub async fn run(
        &self,
        session_id: &str,
        question: &str,
        depth: ResearchDepth,
        source_kinds: Vec<ResearchSourceKind>,
        actor: Actor,
    ) -> Result<ResearchRun, StoreError> {
        if self.sessions.get_session(session_id)?.is_none() {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        let timestamp = now()?;
        let mut run = ResearchRun {
            id: Uuid::now_v7().to_string(),
            session_id: session_id.into(),
            question: question.trim().into(),
            depth,
            source_kinds,
            status: ResearchStatus::Running,
            queries: Vec::new(),
            lanes: Vec::new(),
            limitations: Vec::new(),
            report: String::new(),
            error: String::new(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            completed_at: None,
        };
        self.repository.create_run(run.clone(), actor.clone())?;
        run.queries = plan_queries(&run.question, depth);
        run.updated_at = now()?;
        self.repository.update_run(run.clone(), actor.clone())?;

        let mut worker_count = 0_usize;
        for query in run.queries.clone() {
            for kind in run.source_kinds.clone() {
                if worker_count >= self.limits.max_workers
                    || self.repository.list_sources(&run.id)?.len() >= self.limits.max_sources
                {
                    let message = "bounded research worker or source budget exhausted".to_owned();
                    run.limitations.push(format!("{kind:?}: {message}"));
                    run.lanes.push(lane(
                        &query,
                        kind,
                        ResearchLaneStatus::Skipped,
                        &message,
                        0,
                    )?);
                    continue;
                }
                worker_count = worker_count.saturating_add(1);
                let remaining = self
                    .limits
                    .max_sources
                    .saturating_sub(self.repository.list_sources(&run.id)?.len());
                let collection = self.collector.collect(&run, kind, &query, remaining).await;
                let mut saved = 0_usize;
                if collection.status == ResearchLaneStatus::Completed {
                    for draft in collection.sources.into_iter().take(remaining) {
                        if draft.kind != kind {
                            continue;
                        }
                        if !draft.uri.is_empty()
                            && self
                                .repository
                                .list_sources(&run.id)?
                                .iter()
                                .any(|source| source.uri == draft.uri)
                        {
                            continue;
                        }
                        let index = self
                            .repository
                            .list_sources(&run.id)?
                            .len()
                            .saturating_add(1);
                        let source = ResearchSource {
                            id: Uuid::now_v7().to_string(),
                            run_id: run.id.clone(),
                            label: format!("R{index}"),
                            kind,
                            title: draft.title,
                            uri: draft.uri,
                            content: draft.content,
                            query: query.clone(),
                            metadata: draft.metadata,
                            created_at: now()?,
                        };
                        self.repository.add_source(source, actor.clone())?;
                        saved = saved.saturating_add(1);
                    }
                } else {
                    run.limitations
                        .push(format!("{kind:?}: {}", collection.message));
                }
                run.lanes.push(lane(
                    &query,
                    kind,
                    collection.status,
                    &collection.message,
                    saved,
                )?);
                run.updated_at = now()?;
                self.repository.update_run(run.clone(), actor.clone())?;
            }
        }

        let sources = self.repository.list_sources(&run.id)?;
        for source in &sources {
            if let Some(text) = first_evidence_sentence(&source.content) {
                self.repository.add_claim(
                    ResearchClaim {
                        id: Uuid::now_v7().to_string(),
                        run_id: run.id.clone(),
                        text,
                        source_labels: vec![source.label.clone()],
                        created_at: now()?,
                    },
                    actor.clone(),
                )?;
            }
        }
        let claims = self.repository.list_claims(&run.id)?;
        run.report = synthesize(&run, &sources, &claims);
        run.status = ResearchStatus::Completed;
        run.updated_at = now()?;
        run.completed_at = Some(run.updated_at.clone());
        self.repository.update_run(run.clone(), actor.clone())?;
        self.sessions.append_message(
            session_id,
            &run.id,
            ModelMessage {
                role: ModelMessageRole::Assistant,
                content: run.report.clone(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            Actor {
                actor_type: ActorType::System,
                id: "research-synthesizer".into(),
            },
        )?;
        Ok(run)
    }
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

fn plan_queries(question: &str, depth: ResearchDepth) -> Vec<String> {
    let count = match depth {
        ResearchDepth::Quick => 1,
        ResearchDepth::Standard => 2,
        ResearchDepth::Deep => 3,
    };
    [
        question.to_owned(),
        format!("{question} implementation"),
        format!("{question} tests limitations"),
    ]
    .into_iter()
    .take(count)
    .collect()
}

fn lane(
    query: &str,
    kind: ResearchSourceKind,
    status: ResearchLaneStatus,
    message: &str,
    source_count: usize,
) -> Result<ResearchLane, StoreError> {
    Ok(ResearchLane {
        id: Uuid::now_v7().to_string(),
        query: query.into(),
        kind,
        status,
        message: message.into(),
        source_count,
        updated_at: now()?,
    })
}

fn first_evidence_sentence(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(2_000).collect())
}

fn synthesize(run: &ResearchRun, sources: &[ResearchSource], claims: &[ResearchClaim]) -> String {
    let mut report = format!("# Research: {}\n\n", run.question);
    if claims.is_empty() {
        report.push_str("No source-backed claims were available.\n");
    } else {
        report.push_str("## Findings\n\n");
        for claim in claims {
            let citations = claim
                .source_labels
                .iter()
                .map(|label| format!("[{label}]"))
                .collect::<Vec<_>>()
                .join(" ");
            report.push_str(&format!("- {} {}\n", claim.text, citations));
        }
    }
    if !run.limitations.is_empty() {
        report.push_str("\n## Limitations\n\n");
        for limitation in &run.limitations {
            report.push_str(&format!("- {limitation}\n"));
        }
    }
    if !sources.is_empty() {
        report.push_str("\n## Sources\n\n");
        for source in sources {
            report.push_str(&format!(
                "- [{}] {} — {}\n",
                source.label, source.title, source.uri
            ));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::{
        EventSourcedResearchRepository, ResearchCollection, ResearchCollector, ResearchLimits,
        ResearchService, ResearchSourceDraft,
    };
    use async_trait::async_trait;
    use colossus_contracts::{
        Actor, ActorType, ResearchClaim, ResearchDepth, ResearchLane, ResearchLaneStatus,
        ResearchRun, ResearchSource, ResearchSourceKind, ResearchStatus,
    };
    use colossus_ports::{ResearchRepository, SessionRepository};
    use colossus_session::EventSourcedSessionRepository;
    use colossus_testkit::InMemoryEventJournal;
    use std::{collections::BTreeMap, sync::Arc};

    struct OfflineCollector;

    #[async_trait]
    impl ResearchCollector for OfflineCollector {
        async fn collect(
            &self,
            _run: &ResearchRun,
            kind: ResearchSourceKind,
            query: &str,
            _limit: usize,
        ) -> ResearchCollection {
            match kind {
                ResearchSourceKind::Repo => ResearchCollection {
                    status: ResearchLaneStatus::Completed,
                    message: "repository evidence released".into(),
                    sources: vec![ResearchSourceDraft {
                        kind,
                        title: "Architecture".into(),
                        uri: format!("docs/ARCHITECTURE.md#{query}"),
                        content: format!("Evidence for {query}"),
                        metadata: BTreeMap::new(),
                    }],
                },
                _ => ResearchCollection {
                    status: ResearchLaneStatus::Disabled,
                    message: "adapter is not configured".into(),
                    sources: Vec::new(),
                },
            }
        }
    }

    fn actor() -> Actor {
        Actor {
            actor_type: ActorType::User,
            id: "user-1".into(),
        }
    }

    fn run() -> ResearchRun {
        ResearchRun {
            id: "research-1".into(),
            session_id: "session-1".into(),
            question: "What is implemented?".into(),
            depth: ResearchDepth::Standard,
            source_kinds: vec![ResearchSourceKind::Repo, ResearchSourceKind::Web],
            status: ResearchStatus::Running,
            queries: Vec::new(),
            lanes: Vec::new(),
            limitations: Vec::new(),
            report: String::new(),
            error: String::new(),
            created_at: "2026-07-11T12:00:00Z".into(),
            updated_at: "2026-07-11T12:00:00Z".into(),
            completed_at: None,
        }
    }

    fn source(label: &str) -> ResearchSource {
        ResearchSource {
            id: format!("source-{label}"),
            run_id: "research-1".into(),
            label: label.into(),
            kind: ResearchSourceKind::Repo,
            title: "Architecture".into(),
            uri: "docs/ARCHITECTURE.md".into(),
            content: "The runtime is event sourced.".into(),
            query: "architecture".into(),
            metadata: BTreeMap::new(),
            created_at: "2026-07-11T12:01:00Z".into(),
        }
    }

    #[test]
    fn reconstructs_cited_runs_after_restart() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedResearchRepository::new(journal.clone());
        let mut run = repository.create_run(run(), actor()).expect("create");
        run.queries = vec!["architecture".into()];
        run.lanes = vec![ResearchLane {
            id: "lane-1".into(),
            query: "architecture".into(),
            kind: ResearchSourceKind::Repo,
            status: ResearchLaneStatus::Completed,
            message: "one result".into(),
            source_count: 1,
            updated_at: "2026-07-11T12:01:00Z".into(),
        }];
        run.updated_at = "2026-07-11T12:01:00Z".into();
        repository.update_run(run.clone(), actor()).expect("plan");
        repository
            .add_source(source("R1"), actor())
            .expect("source");
        repository
            .add_claim(
                ResearchClaim {
                    id: "claim-1".into(),
                    run_id: run.id.clone(),
                    text: "The runtime is event sourced.".into(),
                    source_labels: vec!["R1".into()],
                    created_at: "2026-07-11T12:02:00Z".into(),
                },
                actor(),
            )
            .expect("claim");
        run.status = ResearchStatus::Completed;
        run.report = "The runtime is event sourced [R1].".into();
        run.updated_at = "2026-07-11T12:03:00Z".into();
        run.completed_at = Some(run.updated_at.clone());
        repository
            .update_run(run.clone(), actor())
            .expect("complete");

        let reopened = EventSourcedResearchRepository::new(journal);
        assert_eq!(reopened.get_run(&run.id).expect("get"), Some(run));
        assert_eq!(
            reopened.list_sources("research-1").expect("sources").len(),
            1
        );
        assert_eq!(reopened.list_claims("research-1").expect("claims").len(), 1);
    }

    #[test]
    fn rejects_dangling_labels_and_post_terminal_mutation() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedResearchRepository::new(journal);
        let mut run = repository.create_run(run(), actor()).expect("create");
        assert!(repository.add_source(source("R2"), actor()).is_err());
        repository
            .add_source(source("R1"), actor())
            .expect("source");
        let invalid = ResearchClaim {
            id: "claim-1".into(),
            run_id: run.id.clone(),
            text: "Unsupported".into(),
            source_labels: vec!["R2".into()],
            created_at: "2026-07-11T12:02:00Z".into(),
        };
        assert!(repository.add_claim(invalid, actor()).is_err());
        run.status = ResearchStatus::Completed;
        run.report = "Unsupported [R2].".into();
        run.updated_at = "2026-07-11T12:03:00Z".into();
        run.completed_at = Some(run.updated_at.clone());
        assert!(repository.update_run(run.clone(), actor()).is_err());
        run.report = "Supported [R1].".into();
        repository
            .update_run(run.clone(), actor())
            .expect("complete");
        assert!(repository.update_run(run, actor()).is_err());
    }

    #[tokio::test]
    async fn offline_orchestration_persists_progress_limit_and_session_report() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn ResearchRepository> =
            Arc::new(EventSourcedResearchRepository::new(journal.clone()));
        let sessions = Arc::new(EventSourcedSessionRepository::new(journal));
        sessions
            .create_session("session-1", Some("Research"), actor())
            .expect("session");
        let service = ResearchService::new(
            repository.clone(),
            sessions.clone(),
            Arc::new(OfflineCollector),
            ResearchLimits {
                max_sources: 2,
                max_workers: 3,
            },
        )
        .expect("service");
        let run = service
            .run(
                "session-1",
                "How does audit work?",
                ResearchDepth::Standard,
                vec![ResearchSourceKind::Repo, ResearchSourceKind::Web],
                actor(),
            )
            .await
            .expect("research");
        assert_eq!(run.status, ResearchStatus::Completed);
        assert_eq!(run.queries.len(), 2);
        assert_eq!(run.lanes.len(), 4);
        assert!(run.limitations.iter().any(|item| item.contains("Web")));
        assert!(run.report.contains("[R1]"));
        assert_eq!(repository.list_sources(&run.id).expect("sources").len(), 2);
        let messages = sessions.list_messages("session-1").expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].run_id, run.id);
        assert_eq!(messages[0].message.content, run.report);
    }
}
