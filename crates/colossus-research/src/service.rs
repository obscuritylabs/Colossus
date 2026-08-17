use super::*;
use crate::repository::{
    MAX_LIST, MAX_QUERY_BYTES, MAX_QUESTION_BYTES, MAX_REPORT_BYTES, adapter, citation_labels,
};

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

/// Optional model-assisted planner, claim worker, and report synthesizer.
/// Implementations must route every turn through the provider effect gateway.
#[async_trait]
pub trait ResearchModel: Send + Sync {
    /// Produce a bounded query plan. Invalid output is discarded by the service.
    async fn plan(&self, run: &ResearchRun) -> Result<Vec<String>, String>;

    /// Extract bounded claims supported by exactly one supplied canonical source.
    async fn extract(
        &self,
        run: &ResearchRun,
        source: &ResearchSource,
    ) -> Result<Vec<String>, String>;

    /// Produce Markdown using only canonical citation labels.
    async fn synthesize(
        &self,
        run: &ResearchRun,
        sources: &[ResearchSource],
        claims: &[ResearchClaim],
    ) -> Result<String, String>;
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
    model: Option<Arc<dyn ResearchModel>>,
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
        Self::new_with_model(repository, sessions, collector, None, limits)
    }

    /// Compose canonical state with gateway-bound collection and optional model roles.
    pub fn new_with_model(
        repository: Arc<dyn ResearchRepository>,
        sessions: Arc<dyn SessionRepository>,
        collector: Arc<dyn ResearchCollector>,
        model: Option<Arc<dyn ResearchModel>>,
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
            model,
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
        self.run_with_message_run_id(session_id, question, depth, source_kinds, None, actor)
            .await
    }

    /// Execute durable research while assigning the released report to a caller-owned run.
    pub async fn run_with_message_run_id(
        &self,
        session_id: &str,
        question: &str,
        depth: ResearchDepth,
        source_kinds: Vec<ResearchSourceKind>,
        message_run_id: Option<&str>,
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
            progress: Vec::new(),
            limitations: Vec::new(),
            report: String::new(),
            error: String::new(),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            completed_at: None,
        };
        self.repository.create_run(run.clone(), actor.clone())?;
        run.progress.push(progress(
            ResearchPhase::Planning,
            "queries",
            ResearchProgressStatus::Started,
            "Planning bounded research queries.",
            None,
            None,
        )?);
        self.persist(&mut run, &actor)?;
        let fallback_queries = plan_queries(&run.question, depth);
        let (queries, planning_status, planning_message) = match &self.model {
            Some(model) => match model.plan(&run).await.and_then(|queries| {
                normalize_queries(queries, query_budget(depth))
                    .ok_or_else(|| "planner returned invalid or empty queries".into())
            }) {
                Ok(queries) => (
                    queries,
                    ResearchProgressStatus::Completed,
                    "Accepted model-generated research queries.".to_owned(),
                ),
                Err(error) => (
                    fallback_queries,
                    ResearchProgressStatus::Fallback,
                    format!("Used deterministic queries: {}", bounded_message(&error)),
                ),
            },
            None => (
                fallback_queries,
                ResearchProgressStatus::Fallback,
                "Used deterministic queries because no research model is configured.".into(),
            ),
        };
        run.queries = queries;
        run.progress.push(progress(
            ResearchPhase::Planning,
            "queries",
            planning_status,
            &planning_message,
            Some(run.queries.len()),
            Some(run.queries.len()),
        )?);
        run.updated_at = now()?;
        self.repository.update_run(run.clone(), actor.clone())?;

        let source_limit = source_budget(depth).min(self.limits.max_sources);
        let mut worker_count = 0_usize;
        for query in run.queries.clone() {
            for kind in run.source_kinds.clone() {
                if worker_count >= self.limits.max_workers
                    || self.repository.list_sources(&run.id)?.len() >= source_limit
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
                    run.progress.push(progress(
                        ResearchPhase::Collecting,
                        format!("lane:{kind:?}"),
                        ResearchProgressStatus::Skipped,
                        &message,
                        None,
                        None,
                    )?);
                    self.persist(&mut run, &actor)?;
                    continue;
                }
                worker_count = worker_count.saturating_add(1);
                let remaining =
                    source_limit.saturating_sub(self.repository.list_sources(&run.id)?.len());
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
                run.progress.push(progress(
                    ResearchPhase::Collecting,
                    format!("lane:{kind:?}"),
                    if collection.status == ResearchLaneStatus::Completed {
                        ResearchProgressStatus::Completed
                    } else {
                        ResearchProgressStatus::Failed
                    },
                    &collection.message,
                    None,
                    None,
                )?);
                run.updated_at = now()?;
                self.repository.update_run(run.clone(), actor.clone())?;
            }
        }

        let sources = self.repository.list_sources(&run.id)?;
        for (index, source) in sources.iter().enumerate() {
            let fallback = first_evidence_sentence(&source.content)
                .into_iter()
                .collect::<Vec<_>>();
            let (texts, status, message) = match &self.model {
                Some(model) => match model.extract(&run, source).await.and_then(|claims| {
                    normalize_claims(claims).ok_or_else(|| "worker returned invalid claims".into())
                }) {
                    Ok(claims) => (
                        claims,
                        ResearchProgressStatus::Completed,
                        format!("Accepted claims for {}.", source.label),
                    ),
                    Err(error) => (
                        fallback,
                        ResearchProgressStatus::Fallback,
                        format!(
                            "Used deterministic extraction for {}: {}",
                            source.label,
                            bounded_message(&error)
                        ),
                    ),
                },
                None => (
                    fallback,
                    ResearchProgressStatus::Fallback,
                    format!("Used deterministic extraction for {}.", source.label),
                ),
            };
            for text in texts {
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
            run.progress.push(progress(
                ResearchPhase::Workers,
                format!("source:{}", source.label),
                status,
                &message,
                Some(index.saturating_add(1)),
                Some(sources.len()),
            )?);
            self.persist(&mut run, &actor)?;
        }
        let claims = self.repository.list_claims(&run.id)?;
        run.progress.push(progress(
            ResearchPhase::Synthesis,
            "report",
            ResearchProgressStatus::Started,
            "Assembling a citation-bounded report.",
            None,
            None,
        )?);
        self.persist(&mut run, &actor)?;
        let fallback_report = synthesize(&run, &sources, &claims);
        let (report, synthesis_status, synthesis_message) = match &self.model {
            Some(model) => match model.synthesize(&run, &sources, &claims).await {
                Ok(report) if report_citations_valid(&report, &sources) => {
                    let report = canonicalize_sources_section(&report, &sources);
                    if report.len() <= MAX_REPORT_BYTES {
                        (
                            report,
                            ResearchProgressStatus::Completed,
                            "Accepted model-synthesized cited report.".to_owned(),
                        )
                    } else {
                        (
                            fallback_report,
                            ResearchProgressStatus::Fallback,
                            "Used deterministic report because canonical sources exceeded the report bound."
                                .into(),
                        )
                    }
                }
                Ok(_) => (
                    fallback_report,
                    ResearchProgressStatus::Fallback,
                    "Used deterministic report because model citations were invalid.".into(),
                ),
                Err(error) => (
                    fallback_report,
                    ResearchProgressStatus::Fallback,
                    format!("Used deterministic report: {}", bounded_message(&error)),
                ),
            },
            None => (
                fallback_report,
                ResearchProgressStatus::Fallback,
                "Used deterministic report because no research model is configured.".into(),
            ),
        };
        run.report = report;
        run.progress.push(progress(
            ResearchPhase::Synthesis,
            "report",
            synthesis_status,
            &synthesis_message,
            None,
            None,
        )?);
        run.status = ResearchStatus::Completed;
        run.updated_at = now()?;
        run.completed_at = Some(run.updated_at.clone());
        self.repository.update_run(run.clone(), actor.clone())?;
        self.sessions.append_message(
            session_id,
            message_run_id.unwrap_or(&run.id),
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

    /// Mark every abandoned running process as interrupted without retrying collection or models.
    pub fn recover_interrupted(&self, actor: Actor) -> Result<Vec<ResearchRun>, StoreError> {
        let mut recovered = Vec::new();
        for mut run in self.repository.list_runs(None, MAX_LIST)? {
            if run.status != ResearchStatus::Running {
                continue;
            }
            let timestamp = now()?;
            run.status = ResearchStatus::Interrupted;
            run.error = "process exited before the research run reached a terminal event; outcome is interrupted and was not retried".into();
            run.progress.push(progress(
                ResearchPhase::Recovery,
                "interrupt",
                ResearchProgressStatus::Failed,
                &run.error,
                None,
                None,
            )?);
            run.updated_at = timestamp.clone();
            run.completed_at = Some(timestamp);
            self.repository.update_run(run.clone(), actor.clone())?;
            recovered.push(run);
        }
        Ok(recovered)
    }

    fn persist(&self, run: &mut ResearchRun, actor: &Actor) -> Result<(), StoreError> {
        run.updated_at = now()?;
        self.repository.update_run(run.clone(), actor.clone())?;
        Ok(())
    }
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

fn progress(
    phase: ResearchPhase,
    action: impl Into<String>,
    status: ResearchProgressStatus,
    message: &str,
    current: Option<usize>,
    total: Option<usize>,
) -> Result<ResearchProgress, StoreError> {
    Ok(ResearchProgress {
        id: Uuid::now_v7().to_string(),
        phase,
        action: action.into(),
        status,
        message: bounded_message(message),
        current,
        total,
        created_at: now()?,
    })
}

fn bounded_message(message: &str) -> String {
    message.chars().take(MAX_QUERY_BYTES).collect()
}

fn query_budget(depth: ResearchDepth) -> usize {
    match depth {
        ResearchDepth::Quick => 1,
        ResearchDepth::Standard => 3,
        ResearchDepth::Deep => 6,
    }
}

fn source_budget(depth: ResearchDepth) -> usize {
    match depth {
        ResearchDepth::Quick => 4,
        ResearchDepth::Standard => 10,
        ResearchDepth::Deep => 20,
    }
}

fn normalize_queries(queries: Vec<String>, limit: usize) -> Option<Vec<String>> {
    let mut unique = BTreeSet::new();
    let queries = queries
        .into_iter()
        .map(|query| query.trim().to_owned())
        .filter(|query| {
            !query.is_empty()
                && query.len() <= MAX_QUERY_BYTES
                && unique.insert(query.to_ascii_lowercase())
        })
        .take(limit)
        .collect::<Vec<_>>();
    (!queries.is_empty()).then_some(queries)
}

fn normalize_claims(claims: Vec<String>) -> Option<Vec<String>> {
    let mut unique = BTreeSet::new();
    let claims = claims
        .into_iter()
        .map(|claim| claim.trim().to_owned())
        .filter(|claim| {
            !claim.is_empty()
                && claim.len() <= MAX_QUESTION_BYTES
                && unique.insert(claim.to_ascii_lowercase())
        })
        .take(8)
        .collect::<Vec<_>>();
    (!claims.is_empty()).then_some(claims)
}

fn report_citations_valid(report: &str, sources: &[ResearchSource]) -> bool {
    if report.trim().is_empty() || report.len() > MAX_REPORT_BYTES {
        return false;
    }
    let known = sources
        .iter()
        .map(|source| source.label.clone())
        .collect::<BTreeSet<_>>();
    let cited = citation_labels(report);
    cited.iter().all(|label| known.contains(label)) && (sources.is_empty() || !cited.is_empty())
}

fn canonicalize_sources_section(report: &str, sources: &[ResearchSource]) -> String {
    let mut body = report
        .lines()
        .take_while(|line| !is_sources_heading(line))
        .collect::<Vec<_>>()
        .join("\n");
    body.truncate(body.trim_end().len());
    if sources.is_empty() {
        return body;
    }
    body.push_str("\n\n## Sources\n\n");
    for source in sources {
        let title = source
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let uri = source.uri.split_whitespace().collect::<Vec<_>>().join(" ");
        body.push_str(&format!("- [{}] {title} — {uri}\n", source.label));
    }
    body
}

fn is_sources_heading(line: &str) -> bool {
    let line = line.trim();
    let heading = line.trim_start_matches('#');
    heading.len() != line.len()
        && line.len().saturating_sub(heading.len()) <= 6
        && heading.trim().eq_ignore_ascii_case("sources")
}

fn plan_queries(question: &str, depth: ResearchDepth) -> Vec<String> {
    let count = query_budget(depth);
    [
        question.to_owned(),
        format!("{question} implementation"),
        format!("{question} tests limitations"),
        format!("{question} architecture boundaries"),
        format!("{question} security failure modes"),
        format!("{question} operations recovery"),
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
