use super::{
    EventSourcedResearchRepository, ResearchCollection, ResearchCollector, ResearchLimits,
    ResearchModel, ResearchService, ResearchSourceDraft,
};
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, ResearchClaim, ResearchDepth, ResearchLane, ResearchLaneStatus,
    ResearchPhase, ResearchProgressStatus, ResearchRun, ResearchSource, ResearchSourceKind,
    ResearchStatus,
};
use colossus_ports::{EventJournal, ResearchRepository, SessionRepository};
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::{InMemoryEventJournal, assert_research_repository_conformance};
use std::{collections::BTreeMap, sync::Arc};

struct OfflineCollector;

struct StructuredModel;

#[test]
fn event_sourced_research_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    assert_research_repository_conformance(|| {
        Box::new(EventSourcedResearchRepository::new(Arc::clone(&journal)))
    });
}

#[async_trait]
impl ResearchModel for StructuredModel {
    async fn plan(&self, _run: &ResearchRun) -> Result<Vec<String>, String> {
        Ok(vec!["model query".into()])
    }

    async fn extract(
        &self,
        _run: &ResearchRun,
        _source: &ResearchSource,
    ) -> Result<Vec<String>, String> {
        Ok(vec!["Model-backed claim.".into()])
    }

    async fn synthesize(
        &self,
        _run: &ResearchRun,
        _sources: &[ResearchSource],
        _claims: &[ResearchClaim],
    ) -> Result<String, String> {
        Ok("# Model report\n\nModel-backed claim [R1].".into())
    }
}

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
                    uri: format!("docs/develop/architecture.md#{query}"),
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
        progress: Vec::new(),
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
        uri: "docs/develop/architecture.md".into(),
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
    assert_eq!(run.queries.len(), 3);
    assert_eq!(run.lanes.len(), 6);
    assert!(run.limitations.iter().any(|item| item.contains("Web")));
    assert!(
        run.progress
            .iter()
            .any(|item| item.status == ResearchProgressStatus::Fallback)
    );
    assert!(run.report.contains("[R1]"));
    assert_eq!(repository.list_sources(&run.id).expect("sources").len(), 2);
    let messages = sessions.list_messages("session-1").expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].run_id, run.id);
    assert_eq!(messages[0].message.content, run.report);
}

#[tokio::test]
async fn valid_model_phases_replace_fallbacks_without_weakening_citations() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn ResearchRepository> =
        Arc::new(EventSourcedResearchRepository::new(journal.clone()));
    let sessions = Arc::new(EventSourcedSessionRepository::new(journal));
    sessions
        .create_session("session-1", Some("Research"), actor())
        .expect("session");
    let service = ResearchService::new_with_model(
        repository.clone(),
        sessions,
        Arc::new(OfflineCollector),
        Some(Arc::new(StructuredModel)),
        ResearchLimits {
            max_sources: 2,
            max_workers: 2,
        },
    )
    .expect("service");
    let run = service
        .run(
            "session-1",
            "How does audit work?",
            ResearchDepth::Quick,
            vec![ResearchSourceKind::Repo],
            actor(),
        )
        .await
        .expect("research");
    assert_eq!(run.queries, vec!["model query"]);
    assert_eq!(run.report, "# Model report\n\nModel-backed claim [R1].");
    assert!(
        run.progress
            .iter()
            .filter(|progress| matches!(
                progress.phase,
                ResearchPhase::Planning | ResearchPhase::Workers | ResearchPhase::Synthesis
            ))
            .all(|progress| progress.status != ResearchProgressStatus::Fallback)
    );
    assert_eq!(
        repository.list_claims(&run.id).expect("claims")[0].text,
        "Model-backed claim."
    );
}

#[test]
fn startup_recovery_interrupts_running_research_without_retrying() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn ResearchRepository> =
        Arc::new(EventSourcedResearchRepository::new(journal.clone()));
    let sessions = Arc::new(EventSourcedSessionRepository::new(journal));
    sessions
        .create_session("session-1", Some("Research"), actor())
        .expect("session");
    let running = repository.create_run(run(), actor()).expect("run");
    let service = ResearchService::new(
        repository.clone(),
        sessions,
        Arc::new(OfflineCollector),
        ResearchLimits::default(),
    )
    .expect("service");
    let recovered = service.recover_interrupted(actor()).expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].status, ResearchStatus::Interrupted);
    assert!(recovered[0].error.contains("not retried"));
    assert_eq!(
        service.recover_interrupted(actor()).expect("again").len(),
        0
    );
    assert_eq!(
        repository
            .get_run(&running.id)
            .expect("get")
            .expect("record")
            .status,
        ResearchStatus::Interrupted
    );
}
