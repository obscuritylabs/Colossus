use super::*;
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, ContentPart, CreateRunRequest,
    EventSourcedRunRepository, IdempotencyKey, NewRun, RequestId, RunMode, RunResult, RunStatus,
    scopes,
};
use colossus_testkit::InMemoryEventJournal;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod repository;

struct TerminalRaceRepository {
    inner: EventSourcedRunRepository,
    committed: AtomicBool,
    page_reads: AtomicUsize,
    hide_terminal_tail: bool,
}

fn result() -> RunUpdateKind {
    RunUpdateKind::Result {
        result: RunResult {
            output: "complete".into(),
            plan_id: None,
            plan_revision: None,
            plan_status: None,
            goal_id: None,
            profile: "test".into(),
            model_profile: "test".into(),
            provider_profile: "test".into(),
            model: "test".into(),
            elapsed_seconds: 0.1,
        },
    }
}

async fn terminal_race(hide_terminal_tail: bool) -> (Vec<ApiResult<RunUpdate>>, usize) {
    let caller = CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            "app:terminal-race",
            "test-credential",
            ApplicationKind::Enrolled,
            [scopes::RUNS_READ, scopes::RUNS_EXECUTE].map(|scope| ApiScope::new(scope).unwrap()),
            ["primary".into()],
            Vec::new(),
        )
        .unwrap(),
        RequestId::new("terminal-race").unwrap(),
    );
    let request = CreateRunRequest {
        input: vec![ContentPart::Text {
            text: "test".into(),
        }],
        session_id: None,
        end_user_id: None,
        role: Some("primary".into()),
        mode: RunMode::Execute,
        model_profile: None,
        reasoning_effort: None,
        goal_max_iterations: 0,
        research_depth: None,
        research_sources: vec![],
        skill_ids: vec![],
        plan_action: None,
        branch: None,
        max_turns: 1,
        idempotency_key: IdempotencyKey::new("terminal-race").unwrap(),
    };
    let repository = Arc::new(TerminalRaceRepository {
        inner: EventSourcedRunRepository::new(Arc::new(InMemoryEventJournal::default())),
        committed: AtomicBool::new(false),
        page_reads: AtomicUsize::new(0),
        hide_terminal_tail,
    });
    let new_run = NewRun::from_request("run", "session", "primary", &request).unwrap();
    repository
        .inner
        .create_run(&caller, &request, &new_run)
        .unwrap();
    repository
        .inner
        .append_update(
            &caller,
            "run",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    let feeds = Arc::new(RunFeeds::default());
    let watch = RunWatch {
        repository: repository.clone(),
        notifications: feeds.subscribe("run", 2),
        feeds,
        caller,
        run_id: "run".into(),
        cursor: 2,
    };
    let (sender, mut receiver) = mpsc::channel(8);
    // Completion must not need a notification after the terminal snapshot is seen.
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        watch.run(sender, false, || Ok(())),
    )
    .await
    .expect("watch reconciles or reports a gap without spinning");
    let mut updates = Vec::new();
    while let Some(update) = receiver.recv().await {
        updates.push(update);
    }
    (updates, repository.page_reads.load(Ordering::Acquire))
}

#[tokio::test]
async fn terminal_commit_after_empty_page_replays_the_missing_tail_before_eof() {
    let (updates, reads) = terminal_race(false).await;
    assert_eq!(reads, 2);
    assert_eq!(updates.len(), 1);
    let update = updates.into_iter().next().unwrap().unwrap();
    assert_eq!(update.sequence, 3);
    assert!(matches!(update.kind, RunUpdateKind::Result { .. }));
}

#[tokio::test]
async fn persistent_terminal_gap_reports_an_invariant_instead_of_spinning() {
    let (updates, reads) = terminal_race(true).await;
    assert_eq!(reads, 2);
    assert_eq!(updates.len(), 1);
    let error = updates.into_iter().next().unwrap().unwrap_err();
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
}
