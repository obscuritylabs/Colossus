use super::*;
use colossus_contracts::{
    Actor, ActorType, EventClassification, EventEnvelope, ExecutionContext, NewEvent,
    ProjectionWorkItem, SignedCheckpoint,
};
use colossus_ports::{EventJournal, StoreError, VerificationReport};
use colossus_testkit::InMemoryEventJournal;
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct ReadCountingJournal {
    inner: InMemoryEventJournal,
    full_stream_reads: AtomicUsize,
    ranged_stream_reads: AtomicUsize,
    ranged_events_returned: AtomicUsize,
    backwards_stream_reads: AtomicUsize,
    backwards_events_returned: AtomicUsize,
    global_reads: AtomicUsize,
    largest_ranged_limit: AtomicUsize,
    decrypted_payloads: AtomicUsize,
}

impl ReadCountingJournal {
    fn reset_read_counts(&self) {
        self.full_stream_reads.store(0, Ordering::Release);
        self.ranged_stream_reads.store(0, Ordering::Release);
        self.ranged_events_returned.store(0, Ordering::Release);
        self.backwards_stream_reads.store(0, Ordering::Release);
        self.backwards_events_returned.store(0, Ordering::Release);
        self.global_reads.store(0, Ordering::Release);
        self.largest_ranged_limit.store(0, Ordering::Release);
        self.decrypted_payloads.store(0, Ordering::Release);
    }
}

impl EventJournal for ReadCountingJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.inner.append(event)
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.append_batch(events)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.full_stream_reads.fetch_add(1, Ordering::AcqRel);
        self.inner.read_stream(stream_id)
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.ranged_stream_reads.fetch_add(1, Ordering::AcqRel);
        self.largest_ranged_limit.fetch_max(limit, Ordering::AcqRel);
        let events = self
            .inner
            .read_stream_from(stream_id, after_version, limit)?;
        self.ranged_events_returned
            .fetch_add(events.len(), Ordering::AcqRel);
        Ok(events)
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.backwards_stream_reads.fetch_add(1, Ordering::AcqRel);
        let events = self
            .inner
            .read_stream_backwards(stream_id, before_version, limit)?;
        self.backwards_events_returned
            .fetch_add(events.len(), Ordering::AcqRel);
        Ok(events)
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.inner.list_stream_ids(prefix, after, limit)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.global_reads.fetch_add(1, Ordering::AcqRel);
        self.inner.read_global(from_sequence, limit)
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.inner.read_projection_work(from_sequence, limit)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        self.inner.head()
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<serde_json::Value, StoreError> {
        self.decrypted_payloads.fetch_add(1, Ordering::AcqRel);
        self.inner.decrypt_payload(event)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.inner.verify()
    }

    fn is_recovery_mode(&self) -> bool {
        self.inner.is_recovery_mode()
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.inner.checkpoint()
    }
}

fn scope(name: &str) -> ApiScope {
    ApiScope::new(name).expect("valid scope")
}

fn principal(application_id: &str, granted: &[&str]) -> ApplicationPrincipal {
    ApplicationPrincipal::authenticated(
        application_id,
        format!("credential-{application_id}"),
        ApplicationKind::Enrolled,
        granted.iter().map(|value| scope(value)),
        ["assistant".to_owned()],
        std::iter::empty(),
    )
    .expect("authenticated principal")
}

fn caller_context(application_id: &str, request_id: &str, granted: &[&str]) -> CallerContext {
    CallerContext::authenticated(
        principal(application_id, granted),
        RequestId::new(request_id).expect("request id"),
    )
}

fn create_request(key: &str, text: &str) -> CreateRunRequest {
    CreateRunRequest {
        input: vec![ContentPart::Text { text: text.into() }],
        session_id: None,
        role: Some("assistant".into()),
        mode: RunMode::Execute,
        skill_ids: Vec::new(),
        plan_action: None,
        max_turns: 24,
        idempotency_key: IdempotencyKey::new(key).expect("idempotency key"),
    }
}

fn fixture() -> (
    Arc<InMemoryEventJournal>,
    EventSourcedRunRepository,
    CallerContext,
) {
    let journal = Arc::new(InMemoryEventJournal::default());
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository = EventSourcedRunRepository::new(durable);
    let caller = caller_context(
        "app:desktop-ui",
        "request-1",
        &[
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
            scopes::PROMPTS_RESPOND,
            scopes::APPROVALS_RESPOND,
        ],
    );
    (journal, repository, caller)
}

fn create_run(
    repository: &EventSourcedRunRepository,
    caller: &CallerContext,
    request: &CreateRunRequest,
    run_id: &str,
    session_id: &str,
) -> Run {
    let new_run = NewRun::from_request(run_id, session_id, "assistant", request).expect("new run");
    repository
        .create_run(caller, request, &new_run)
        .expect("create run")
        .value
}

#[test]
fn caller_context_is_exact_scope_deny_by_default_and_application_attributed() {
    let principal = principal("app:desktop-ui", &[scopes::RUNS_READ]);
    assert!(principal.has_scope(scopes::RUNS_READ));
    assert!(!principal.has_scope(scopes::RUNS_EXECUTE));
    assert!(principal.allows_role("assistant"));
    assert!(!principal.allows_role("assistant-admin"));
    assert!(!principal.allows_tool("process.run"));

    let caller = CallerContext::authenticated(
        principal,
        RequestId::new("request-attribution").expect("request"),
    );
    assert_eq!(
        caller.actor(),
        colossus_contracts::Actor {
            actor_type: ActorType::Application,
            id: "app:desktop-ui".into(),
        }
    );
    let denial = caller
        .require_scope(scopes::RUNS_EXECUTE)
        .expect_err("scope must deny");
    assert_eq!(denial.code, ApiErrorCode::PermissionDenied);
    assert_eq!(denial.reason, ApiErrorReason::ScopeDenied);
    assert_eq!(
        denial.correlation_id.as_ref().map(RequestId::as_str),
        Some("request-attribution")
    );
}

#[test]
fn identity_values_validate_and_idempotency_debug_is_redacted() {
    assert!(
        ApplicationPrincipal::authenticated(
            " bad",
            "credential",
            ApplicationKind::Enrolled,
            std::iter::empty(),
            std::iter::empty(),
            std::iter::empty(),
        )
        .is_err()
    );
    assert!(
        ApplicationPrincipal::authenticated(
            "desktop-ui",
            "credential",
            ApplicationKind::Enrolled,
            std::iter::empty(),
            std::iter::empty(),
            std::iter::empty(),
        )
        .is_err()
    );
    assert!(
        ApplicationPrincipal::authenticated(
            "app:",
            "credential",
            ApplicationKind::Enrolled,
            std::iter::empty(),
            std::iter::empty(),
            std::iter::empty(),
        )
        .is_err()
    );
    assert!(RequestId::new("has a space").is_err());
    assert!(ApiScope::new("runs:*").is_err());
    let key = IdempotencyKey::new("private-retry-key").expect("key");
    assert_eq!(format!("{key:?}"), "IdempotencyKey([REDACTED])");
    let decoded: Result<IdempotencyKey, _> = serde_json::from_str("\" invalid\"");
    assert!(decoded.is_err());
}

#[test]
fn run_result_reads_pre_profile_split_durable_payloads() {
    let result: RunResult = serde_json::from_value(serde_json::json!({
        "output": "legacy result",
        "profile": "legacy-provider",
        "model": "legacy-model",
        "elapsed_seconds": 1.25
    }))
    .expect("legacy run result");

    assert_eq!(result.profile, "legacy-provider");
    assert_eq!(result.model_profile, "legacy-provider");
    assert_eq!(result.provider_profile, "legacy-provider");
    assert_eq!(result.model, "legacy-model");
    assert_eq!(result.plan_id, None);
    assert!(
        serde_json::to_value(&result)
            .expect("serialize legacy run result")
            .get("plan_id")
            .is_none()
    );
}

#[test]
fn run_result_preserves_optional_plan_identity() {
    let result: RunResult = serde_json::from_value(serde_json::json!({
        "output": "Plan saved",
        "plan_id": "plan-1",
        "profile": "default",
        "model_profile": "default",
        "provider_profile": "default-provider",
        "model": "model",
        "elapsed_seconds": 1.25
    }))
    .expect("plan run result");

    assert_eq!(result.plan_id.as_deref(), Some("plan-1"));
    let value = serde_json::to_value(&result).expect("serialize run result");
    assert_eq!(value["plan_id"], "plan-1");
}

#[test]
fn run_cancellation_preserves_optional_plan_identity_and_reads_legacy_payloads() {
    let legacy: RunCancellation = serde_json::from_value(serde_json::json!({
        "turn": 1,
        "message": "cancelled"
    }))
    .expect("legacy run cancellation");
    assert_eq!(legacy.plan_id, None);
    assert!(
        serde_json::to_value(&legacy)
            .expect("serialize legacy cancellation")
            .get("plan_id")
            .is_none()
    );

    let cancellation: RunCancellation = serde_json::from_value(serde_json::json!({
        "turn": 2,
        "message": "cancelled after persistence",
        "plan_id": "plan-1"
    }))
    .expect("plan run cancellation");
    assert_eq!(cancellation.plan_id.as_deref(), Some("plan-1"));
}

#[test]
fn run_failure_reads_payloads_from_before_response_metadata_was_added() {
    let failure: RunFailure = serde_json::from_value(serde_json::json!({
        "code": "runtime.failed",
        "message": "the run failed with a known outcome",
        "outcome": "known"
    }))
    .expect("legacy run failure");

    assert_eq!(failure.code, "runtime.failed");
    assert!(!failure.recoverable);
    assert_eq!(failure.http_status, None);
    assert_eq!(failure.retry_after_ms, None);
}

#[test]
fn create_is_atomic_application_attributed_and_replays_exact_request() {
    let (journal, repository, caller) = fixture();
    let request = create_request("create-key", "Build the UI");
    let first = create_run(&repository, &caller, &request, "run-1", "session-1");
    assert_eq!(first.status, RunStatus::Queued);
    assert_eq!(first.last_sequence, 1);
    assert_eq!(first.title, "Build the UI");

    let retry_caller = caller_context(
        "app:desktop-ui",
        "request-2",
        &[
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
        ],
    );
    let retry_new = NewRun::from_request("run-ignored", "session-ignored", "assistant", &request)
        .expect("retry run");
    let replay = repository
        .create_run(&retry_caller, &request, &retry_new)
        .expect("idempotent replay");
    assert!(replay.replayed);
    assert_eq!(replay.value.id, "run-1");
    assert_eq!(replay.value.session_id, "session-1");

    let events = journal.read_global(1, 32).expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "api.run.created.v1")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "api.run.indexed.v1")
            .count(),
        1
    );
    assert!(events.iter().all(|event| {
        event.actor.actor_type == ActorType::Application && event.actor.id == "app:desktop-ui"
    }));
}

#[test]
fn run_title_is_safe_bounded_and_derived_from_visible_input() {
    let (_, repository, caller) = fixture();
    let mut request = create_request(
        "title-key",
        &format!("  Review\n\u{202e}{}  ", "workspace ".repeat(20)),
    );
    request.input.push(ContentPart::Text {
        text: "ignored continuation".into(),
    });

    let run = create_run(&repository, &caller, &request, "run-title", "session-title");

    assert!(run.title.starts_with("Review workspace workspace"));
    assert!(!run.title.contains('\n'));
    assert!(!run.title.contains('\u{202e}'));
    assert!(run.title.ends_with('…'));
    assert!(run.title.chars().count() <= 80);
}

#[test]
fn concurrent_distinct_creates_serialize_into_one_complete_owner_index() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository = Arc::new(EventSourcedRunRepository::new(durable));
    let caller = caller_context(
        "app:desktop-ui",
        "request-concurrent-create",
        &[scopes::RUNS_EXECUTE, scopes::RUNS_READ],
    );
    let barrier = Arc::new(Barrier::new(8));
    let mut workers = Vec::new();
    for index in 0..8 {
        let repository = Arc::clone(&repository);
        let caller = caller.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let request =
                create_request(&format!("concurrent-key-{index}"), &format!("Run {index}"));
            let new_run = NewRun::from_request(
                format!("run-concurrent-{index}"),
                format!("session-concurrent-{index}"),
                "assistant",
                &request,
            )
            .expect("new run");
            barrier.wait();
            repository
                .create_run(&caller, &request, &new_run)
                .expect("concurrent create")
                .value
                .id
        }));
    }
    let created = workers
        .into_iter()
        .map(|worker| worker.join().expect("create worker"))
        .collect::<BTreeSet<_>>();
    assert_eq!(created.len(), 8);

    let indexed = journal
        .read_global(1, 128)
        .expect("journal")
        .into_iter()
        .filter(|event| event.event_type == "api.run.indexed.v1")
        .count();
    assert_eq!(indexed, 8);

    let mut listed = BTreeSet::new();
    let mut page_token = None;
    loop {
        let response = repository
            .list_runs(
                &caller,
                &ListRunsRequest {
                    session_id: None,
                    statuses: Vec::new(),
                    page_size: 3,
                    page_token,
                },
            )
            .expect("indexed list");
        listed.extend(response.runs.into_iter().map(|run| run.id));
        let Some(next) = response.next_page_token else {
            break;
        };
        page_token = Some(next);
    }
    assert_eq!(listed, created);
}

#[test]
fn concurrent_idempotent_create_commits_exactly_one_index_entry() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository = Arc::new(EventSourcedRunRepository::new(durable));
    let caller = caller_context(
        "app:desktop-ui",
        "request-concurrent-replay",
        &[scopes::RUNS_EXECUTE, scopes::RUNS_READ],
    );
    let request = create_request("same-concurrent-key", "Create exactly once");
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for index in 0..2 {
        let repository = Arc::clone(&repository);
        let caller = caller.clone();
        let request = request.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let new_run = NewRun::from_request(
                format!("run-idempotent-{index}"),
                format!("session-idempotent-{index}"),
                "assistant",
                &request,
            )
            .expect("new run");
            barrier.wait();
            repository
                .create_run(&caller, &request, &new_run)
                .expect("idempotent concurrent create")
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().expect("create worker"))
        .collect::<Vec<_>>();
    assert_eq!(results[0].value.id, results[1].value.id);
    assert_ne!(results[0].replayed, results[1].replayed);
    let events = journal.read_global(1, 16).expect("journal");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "api.run.created.v1")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "api.run.indexed.v1")
            .count(),
        1
    );
}

#[test]
fn idempotency_key_reuse_with_different_request_fails_closed() {
    let (_, repository, caller) = fixture();
    let first = create_request("same-key", "First request");
    create_run(&repository, &caller, &first, "run-1", "session-1");

    let changed = create_request("same-key", "Different request");
    let new_run =
        NewRun::from_request("run-2", "session-2", "assistant", &changed).expect("new run");
    let error = repository
        .create_run(&caller, &changed, &new_run)
        .expect_err("key reuse must conflict");
    assert_eq!(error.code, ApiErrorCode::Conflict);
    assert_eq!(error.reason, ApiErrorReason::IdempotencyKeyReused);
}

#[test]
fn create_request_rejects_more_than_the_bounded_number_of_input_parts() {
    let mut request = create_request("many-input-parts", "x");
    request.input = (0..129)
        .map(|_| ContentPart::Text { text: "x".into() })
        .collect();

    let error = request
        .validate()
        .expect_err("an oversized input-part vector must fail before iteration");
    assert_eq!(error.code, ApiErrorCode::InvalidArgument);
    assert_eq!(error.reason, ApiErrorReason::InvalidArgument);
    assert_eq!(error.violations[0].field, "input");
}

#[test]
fn create_request_accepts_opaque_artifact_input_and_rejects_path_like_ids() {
    let mut request = create_request("artifact-input", "Review the attachment");
    request.input.push(ContentPart::Artifact {
        artifact_id: format!("artifact-{}", "a".repeat(64)),
    });
    request.validate().expect("opaque artifact input");

    request.input[1] = ContentPart::Artifact {
        artifact_id: "../private/report.md".into(),
    };
    let error = request
        .validate()
        .expect_err("artifact identifiers must never accept paths");
    assert_eq!(error.reason, ApiErrorReason::InvalidArgument);
    assert_eq!(error.violations[0].field, "input.artifact_id");
}

#[test]
fn create_request_rejects_public_skill_activation() {
    let mut request = create_request("skill-denied", "Do not load skills");
    request.skill_ids = vec!["private-skill".into()];

    let error = request
        .validate()
        .expect_err("public skills must fail closed until grants have a skill ceiling");
    assert_eq!(error.reason, ApiErrorReason::InvalidArgument);
    assert_eq!(error.violations[0].field, "skill_ids");
}

#[test]
fn plan_continuation_requires_exact_session_mode_revision_and_goal_budget() {
    let mut revise = create_request("revise-plan", "Change step two");
    revise.session_id = Some("session-1".into());
    revise.mode = RunMode::Plan;
    revise.plan_action = Some(PlanRunAction::Revise {
        source_run_id: "run-plan-source".into(),
        expected_revision: 3,
    });
    revise.validate().expect("exact Plan revision");

    revise.mode = RunMode::Execute;
    let error = revise
        .validate()
        .expect_err("revision cannot widen into Execute Mode");
    assert_eq!(error.violations[0].field, "mode");

    let mut execute = create_request("execute-plan", "Run the approved Plan");
    execute.session_id = Some("session-1".into());
    execute.plan_action = Some(PlanRunAction::Execute {
        source_run_id: "run-plan-source".into(),
        expected_revision: 3,
        strategy: PlanExecutionStrategy::Goal { max_iterations: 51 },
    });
    let error = execute
        .validate()
        .expect_err("Goal budget must remain bounded");
    assert_eq!(
        error.violations[0].field,
        "plan_action.strategy.max_iterations"
    );
}

#[test]
fn create_replay_can_be_resolved_without_claiming_another_run() {
    let (journal, repository, caller) = fixture();
    let request = create_request("resolve-create", "Run once");
    let created = create_run(&repository, &caller, &request, "run-1", "session-1");

    let replay = repository
        .resolve_create_run(&caller, &request)
        .expect("resolve replay")
        .expect("existing replay");
    assert_eq!(replay.id, created.id);
    assert_eq!(
        journal
            .read_global(1, 100)
            .expect("events")
            .iter()
            .filter(|event| event.event_type == "api.run.created.v1")
            .count(),
        1
    );
}

#[test]
fn released_updates_replay_in_sequence_and_reconstruct_after_restart() {
    let (journal, repository, caller) = fixture();
    let request = create_request("create-key", "Run");
    create_run(&repository, &caller, &request, "run-1", "session-1");
    let running = repository
        .append_update(
            &caller,
            "run-1",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    assert_eq!(running.sequence, 2);
    repository
        .append_update(
            &caller,
            "run-1",
            2,
            RunUpdateKind::Notice {
                notice: RunNotice {
                    reason: "run.phase.responding".into(),
                    message: "run phase changed to responding at turn 1".into(),
                },
            },
        )
        .expect("event");
    repository
        .append_update(
            &caller,
            "run-1",
            3,
            RunUpdateKind::Result {
                result: RunResult {
                    output: "Done".into(),
                    plan_id: None,
                    plan_revision: None,
                    plan_status: None,
                    goal_id: None,
                    profile: "default".into(),
                    model_profile: "default".into(),
                    provider_profile: "default-provider".into(),
                    model: "model".into(),
                    elapsed_seconds: 1.0,
                },
            },
        )
        .expect("result");

    let replay = repository
        .updates_after(&caller, "run-1", 1, 32)
        .expect("replay");
    assert_eq!(
        replay
            .iter()
            .map(|update| update.sequence)
            .collect::<Vec<_>>(),
        [2, 3, 4]
    );

    let durable: Arc<dyn EventJournal> = journal;
    let reopened = EventSourcedRunRepository::new(durable);
    let execution = reopened
        .execution_request(&caller, "run-1")
        .expect("execution request")
        .expect("durable execution request");
    assert_eq!(execution.request, request);
    assert_eq!(execution.application_id, "app:desktop-ui");
    assert!(
        execution
            .scopes
            .iter()
            .any(|scope| scope.as_str() == scopes::RUNS_EXECUTE)
    );
    assert_eq!(execution.allowed_roles, ["assistant"]);
    assert!(execution.allowed_tools.is_empty());
    let run = reopened
        .get_run(&caller, "run-1")
        .expect("get")
        .expect("run");
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(
        run.result.as_ref().map(|result| result.output.as_str()),
        Some("Done")
    );
    assert!(run.started_at.is_some());
    assert!(run.finished_at.is_some());
    assert_eq!(run.last_sequence, 4);
    assert_eq!(run.etag.len(), 64);

    let error = reopened
        .append_update(
            &caller,
            "run-1",
            4,
            RunUpdateKind::Notice {
                notice: RunNotice {
                    reason: "run.phase.completed".into(),
                    message: "run phase changed to completed at turn 1".into(),
                },
            },
        )
        .expect_err("terminal update");
    assert_eq!(error.reason, ApiErrorReason::InvalidRunTransition);
}

#[test]
fn update_catch_up_uses_only_bounded_ranged_stream_reads() {
    let journal = Arc::new(ReadCountingJournal::default());
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository = EventSourcedRunRepository::new(durable);
    let caller = caller_context(
        "app:desktop-ui",
        "request-ranged-catch-up",
        &[scopes::RUNS_EXECUTE, scopes::RUNS_READ],
    );
    let request = create_request("ranged-catch-up", "Run");
    create_run(
        &repository,
        &caller,
        &request,
        "run-ranged",
        "session-ranged",
    );
    for expected_sequence in 1_u64..=256 {
        repository
            .append_update(
                &caller,
                "run-ranged",
                expected_sequence,
                RunUpdateKind::OutputDelta {
                    text: format!("delta-{expected_sequence}"),
                },
            )
            .expect("append long run stream");
    }
    journal.reset_read_counts();

    let updates = repository
        .updates_after(&caller, "run-ranged", 250, 5)
        .expect("bounded catch-up");

    assert_eq!(
        updates
            .iter()
            .map(|update| update.sequence)
            .collect::<Vec<_>>(),
        [251, 252, 253, 254, 255]
    );
    assert_eq!(journal.full_stream_reads.load(Ordering::Acquire), 0);
    assert_eq!(journal.ranged_stream_reads.load(Ordering::Acquire), 2);
    assert_eq!(journal.ranged_events_returned.load(Ordering::Acquire), 6);
    assert_eq!(journal.largest_ranged_limit.load(Ordering::Acquire), 5);
}

#[test]
fn append_update_reads_only_the_authenticated_creation_and_tail() {
    let journal = Arc::new(ReadCountingJournal::default());
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository = EventSourcedRunRepository::new(durable);
    let caller = caller_context(
        "app:desktop-ui",
        "request-single-pass-append",
        &[scopes::RUNS_EXECUTE, scopes::RUNS_READ],
    );
    let request = create_request("single-pass-append", "Run");
    create_run(
        &repository,
        &caller,
        &request,
        "run-single-pass",
        "session-single-pass",
    );
    repository
        .append_update(
            &caller,
            "run-single-pass",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    repository
        .append_update(
            &caller,
            "run-single-pass",
            2,
            RunUpdateKind::Notice {
                notice: RunNotice {
                    reason: "run.phase.responding".into(),
                    message: "responding".into(),
                },
            },
        )
        .expect("notice");
    journal.reset_read_counts();

    repository
        .append_update(
            &caller,
            "run-single-pass",
            3,
            RunUpdateKind::OutputDelta {
                text: "visible".into(),
            },
        )
        .expect("single-pass append");

    assert_eq!(journal.full_stream_reads.load(Ordering::Acquire), 0);
    assert_eq!(journal.ranged_stream_reads.load(Ordering::Acquire), 1);
    assert_eq!(journal.ranged_events_returned.load(Ordering::Acquire), 1);
    assert_eq!(journal.backwards_stream_reads.load(Ordering::Acquire), 1);
    assert_eq!(journal.backwards_events_returned.load(Ordering::Acquire), 2);
    assert_eq!(journal.decrypted_payloads.load(Ordering::Acquire), 3);
}

#[test]
fn append_update_rejects_a_tail_projection_unlinked_from_its_predecessor() {
    let (journal, repository, caller) = fixture();
    let request = create_request("forged-tail-create", "Run");
    create_run(
        &repository,
        &caller,
        &request,
        "run-forged-tail",
        "session-forged-tail",
    );
    let running = RunUpdateKind::State {
        status: RunStatus::Running,
    };
    repository
        .append_update(&caller, "run-forged-tail", 1, running.clone())
        .expect("running");
    let prior = repository
        .get_run(&caller, "run-forged-tail")
        .expect("read prior run")
        .expect("prior run");
    let forged_kind = RunUpdateKind::OutputDelta {
        text: "forged".into(),
    };
    let mut forged =
        crate::repository::stored_update_payload_for_test(&caller, &prior, &[running], forged_kind)
            .expect("encoded forged update");
    forged["released_bytes_before"] = serde_json::json!(0);
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "api-run:run-forged-tail".into(),
            expected_stream_version: 2,
            classification: EventClassification::Domain,
            event_type: "api.run.update.v1".into(),
            actor: caller.actor(),
            context: ExecutionContext {
                correlation_id: "request-forged-tail".into(),
                session_id: Some("session-forged-tail".into()),
                run_id: Some("run-forged-tail".into()),
                ..ExecutionContext::default()
            },
            payload: forged,
        })
        .expect("append forged tail");

    let error = repository
        .append_update(
            &caller,
            "run-forged-tail",
            3,
            RunUpdateKind::OutputDelta {
                text: "must not append".into(),
            },
        )
        .expect_err("unlinked tail projection must fail closed");

    assert_eq!(error.code, ApiErrorCode::Internal);
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    assert_eq!(
        journal
            .read_stream("api-run:run-forged-tail")
            .expect("run events")
            .len(),
        3
    );
}

#[test]
fn preview_run_journal_golden_updates_remain_replayable() {
    let (_journal, repository, caller) = fixture();
    let request = create_request("preview-golden-create", "Replay preview journal");
    let initial = create_run(
        &repository,
        &caller,
        &request,
        "run-preview-golden",
        "session-preview-golden",
    );
    let replay = |contents: &str| {
        crate::repository::replay_preview_stored_update_for_test(
            &initial,
            2,
            "2026-01-01T00:00:03Z",
            serde_json::from_str(contents).expect("golden JSON"),
        )
        .expect("preview update replay")
    };

    let completed = replay(include_str!(
        "../tests/fixtures/run-journal-preview-completed.json"
    ));
    assert_eq!(completed.status, RunStatus::Completed);
    let result = completed.result.expect("completed result");
    assert_eq!(result.profile, "preview-default");
    assert_eq!(result.model_profile, "preview-default");
    assert_eq!(result.provider_profile, "preview-default");

    let failed = replay(include_str!(
        "../tests/fixtures/run-journal-preview-failed.json"
    ));
    assert_eq!(failed.status, RunStatus::Failed);
    let failure = failed.failure.expect("failure");
    assert!(!failure.recoverable);
    assert_eq!(failure.http_status, None);
    assert_eq!(failure.retry_after_ms, None);

    let waiting = replay(include_str!(
        "../tests/fixtures/run-journal-preview-approval-pending.json"
    ));
    assert_eq!(waiting.status, RunStatus::Waiting);
    assert_eq!(
        waiting.pending_interaction.expect("pending approval").id,
        "approval-preview"
    );

    let resumed = replay(include_str!(
        "../tests/fixtures/run-journal-preview-resumed.json"
    ));
    assert_eq!(resumed.status, RunStatus::Running);
    assert!(resumed.pending_interaction.is_none());
}

#[test]
fn cancellation_claim_and_state_transition_are_atomic_and_replayable() {
    let (journal, repository, caller) = fixture();
    let request = create_request("create-key", "Run");
    create_run(&repository, &caller, &request, "run-1", "session-1");
    let cancel_key = IdempotencyKey::new("cancel-key").expect("cancel key");
    let first = repository
        .request_cancellation(&caller, "run-1", &cancel_key)
        .expect("cancel");
    assert!(!first.replayed);
    assert_eq!(first.value.status, RunStatus::Cancelling);
    assert_eq!(first.value.last_sequence, 2);

    let replay = repository
        .request_cancellation(&caller, "run-1", &cancel_key)
        .expect("cancel replay");
    assert!(replay.replayed);
    assert_eq!(replay.value.status, RunStatus::Cancelling);

    let no_op_key = IdempotencyKey::new("cancel-no-op-key").expect("cancel no-op key");
    let no_op = repository
        .request_cancellation(&caller, "run-1", &no_op_key)
        .expect("claim cancelling no-op");
    assert!(!no_op.replayed);
    assert_eq!(no_op.value.status, RunStatus::Cancelling);
    let no_op_replay = repository
        .request_cancellation(&caller, "run-1", &no_op_key)
        .expect("replay cancelling no-op");
    assert!(no_op_replay.replayed);
    assert_eq!(no_op_replay.value.status, RunStatus::Cancelling);

    let events = journal.read_stream("api-run:run-1").expect("run events");
    assert_eq!(events.len(), 2);
}

#[test]
fn terminal_cancellation_no_op_claims_its_idempotency_key() {
    let (_, repository, caller) = fixture();
    let request = create_request("terminal-cancel-create", "Run");
    create_run(
        &repository,
        &caller,
        &request,
        "run-terminal",
        "session-terminal",
    );
    repository
        .append_update(
            &caller,
            "run-terminal",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    repository
        .append_update(
            &caller,
            "run-terminal",
            2,
            RunUpdateKind::Result {
                result: RunResult {
                    output: "done".into(),
                    plan_id: None,
                    plan_revision: None,
                    plan_status: None,
                    goal_id: None,
                    profile: "default".into(),
                    model_profile: "default".into(),
                    provider_profile: "default-provider".into(),
                    model: "model".into(),
                    elapsed_seconds: 1.0,
                },
            },
        )
        .expect("terminal");

    let key = IdempotencyKey::new("terminal-cancel-key").expect("cancel key");
    let first = repository
        .request_cancellation(&caller, "run-terminal", &key)
        .expect("claim terminal no-op");
    assert!(!first.replayed);
    assert_eq!(first.value.status, RunStatus::Completed);
    let replay = repository
        .request_cancellation(&caller, "run-terminal", &key)
        .expect("replay terminal no-op");
    assert!(replay.replayed);
    assert_eq!(replay.value.status, RunStatus::Completed);

    let other_request = create_request("other-terminal-cancel-create", "Other run");
    create_run(
        &repository,
        &caller,
        &other_request,
        "run-other",
        "session-other",
    );
    let error = repository
        .request_cancellation(&caller, "run-other", &key)
        .expect_err("claimed key cannot cancel another run");
    assert_eq!(error.reason, ApiErrorReason::IdempotencyKeyReused);
}

#[test]
fn cancellation_atomically_closes_a_pending_interaction() {
    let (journal, repository, caller) = fixture();
    let request = create_request("create-key", "Ask then stop");
    create_run(&repository, &caller, &request, "run-1", "session-1");
    repository
        .append_update(
            &caller,
            "run-1",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    repository
        .append_update(
            &caller,
            "run-1",
            2,
            RunUpdateKind::Interaction {
                interaction: Interaction {
                    id: "interaction-1".into(),
                    kind: InteractionKind::Prompt,
                    status: InteractionStatus::Pending,
                    application_id: "app:desktop-ui".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                    prompt: "Continue?".into(),
                    choices: Vec::new(),
                    allow_free_form: true,
                    request_hash: None,
                    action: None,
                    resource: None,
                    risk: None,
                    expires_at: "2999-01-01T00:00:00Z".into(),
                    response: None,
                    responded_at: None,
                },
            },
        )
        .expect("pending interaction");
    let blocked = repository
        .append_update(
            &caller,
            "run-1",
            3,
            RunUpdateKind::Notice {
                notice: RunNotice {
                    reason: "run.waiting".into(),
                    message: "must not duplicate the pending projection".into(),
                },
            },
        )
        .expect_err("pending interactions must block unrelated updates");
    assert_eq!(blocked.reason, ApiErrorReason::InvalidRunTransition);

    let cancelled = repository
        .request_cancellation(
            &caller,
            "run-1",
            &IdempotencyKey::new("cancel-key").expect("cancel key"),
        )
        .expect("cancel");
    assert_eq!(cancelled.value.status, RunStatus::Cancelling);
    assert!(cancelled.value.pending_interaction.is_none());
    assert_eq!(cancelled.value.last_sequence, 5);

    let updates = repository
        .updates_after(&caller, "run-1", 3, 10)
        .expect("cancellation updates");
    assert!(matches!(
        &updates[0].kind,
        RunUpdateKind::Interaction { interaction }
            if interaction.status == InteractionStatus::Cancelled
    ));
    assert!(matches!(
        &updates[1].kind,
        RunUpdateKind::State {
            status: RunStatus::Cancelling
        }
    ));
    assert_eq!(
        journal
            .read_stream("api-run:run-1")
            .expect("run events")
            .len(),
        5
    );
}

#[test]
fn maximum_valid_cancellation_lifecycle_remains_listable() {
    let (_journal, repository, caller) = fixture();
    let request = create_request("max-lifecycle-create", "Exercise lifecycle headroom");
    create_run(
        &repository,
        &caller,
        &request,
        "run-max-lifecycle",
        "session-max-lifecycle",
    );
    repository
        .append_update(
            &caller,
            "run-max-lifecycle",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    for expected_sequence in 2_u64..=4_094 {
        repository
            .append_update(
                &caller,
                "run-max-lifecycle",
                expected_sequence,
                RunUpdateKind::OutputDelta {
                    text: String::new(),
                },
            )
            .expect("fill nonterminal sequence budget");
    }
    repository
        .append_update(
            &caller,
            "run-max-lifecycle",
            4_095,
            RunUpdateKind::Interaction {
                interaction: Interaction {
                    id: "interaction-max-lifecycle".into(),
                    kind: InteractionKind::Prompt,
                    status: InteractionStatus::Pending,
                    application_id: "app:desktop-ui".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                    prompt: "Stop?".into(),
                    choices: Vec::new(),
                    allow_free_form: true,
                    request_hash: None,
                    action: None,
                    resource: None,
                    risk: None,
                    expires_at: "2999-01-01T00:00:00Z".into(),
                    response: None,
                    responded_at: None,
                },
            },
        )
        .expect("last ordinary nonterminal update");
    let cancelling = repository
        .request_cancellation(
            &caller,
            "run-max-lifecycle",
            &IdempotencyKey::new("max-lifecycle-cancel").expect("cancel key"),
        )
        .expect("cancelling");
    assert_eq!(cancelling.value.last_sequence, 4_098);
    assert_eq!(cancelling.value.status, RunStatus::Cancelling);
    repository
        .append_update(
            &caller,
            "run-max-lifecycle",
            4_098,
            RunUpdateKind::Cancellation {
                cancellation: RunCancellation {
                    turn: 0,
                    message: "cancelled".into(),
                    plan_id: None,
                    plan_revision: None,
                    plan_status: None,
                    goal_id: None,
                },
            },
        )
        .expect("terminal cancellation");

    let listed = repository
        .list_runs(
            &caller,
            &ListRunsRequest {
                session_id: None,
                statuses: Vec::new(),
                page_size: 1,
                page_token: None,
            },
        )
        .expect("maximum valid stream remains listable");
    assert_eq!(listed.runs.len(), 1);
    assert_eq!(listed.runs[0].id, "run-max-lifecycle");
    assert_eq!(listed.runs[0].status, RunStatus::Cancelled);
    assert_eq!(listed.runs[0].last_sequence, 4_099);
}

#[test]
fn prompt_response_is_principal_bound_one_use_and_idempotent() {
    let (_, repository, caller) = fixture();
    let request = create_request("create-key", "Ask");
    create_run(&repository, &caller, &request, "run-1", "session-1");
    repository
        .append_update(
            &caller,
            "run-1",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    let interaction = Interaction {
        id: "interaction-1".into(),
        kind: InteractionKind::Prompt,
        status: InteractionStatus::Pending,
        application_id: "app:desktop-ui".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        prompt: "Continue?".into(),
        choices: vec!["Yes".into(), "No".into()],
        allow_free_form: false,
        request_hash: None,
        action: None,
        resource: None,
        risk: None,
        expires_at: "2999-01-01T00:00:00Z".into(),
        response: None,
        responded_at: None,
    };
    let mut misbound = interaction.clone();
    misbound.application_id = "app:other-ui".into();
    let binding_error = repository
        .append_update(
            &caller,
            "run-1",
            2,
            RunUpdateKind::Interaction {
                interaction: misbound,
            },
        )
        .expect_err("misbound interaction");
    assert_eq!(binding_error.reason, ApiErrorReason::InternalInvariant);
    repository
        .append_update(
            &caller,
            "run-1",
            2,
            RunUpdateKind::Interaction {
                interaction: interaction.clone(),
            },
        )
        .expect("interaction");
    let waiting = repository
        .get_run(&caller, "run-1")
        .expect("get")
        .expect("run");
    assert_eq!(waiting.status, RunStatus::Waiting);
    let response = InteractionResponse::Prompt {
        answer: "Yes".into(),
        selected_index: Some(0),
    };
    let key = IdempotencyKey::new("respond-key").expect("key");
    let first = repository
        .respond_interaction(
            &caller,
            "run-1",
            "interaction-1",
            &waiting.etag,
            &key,
            response.clone(),
        )
        .expect("respond");
    assert!(!first.replayed);
    assert_eq!(first.value.status, InteractionStatus::Responded);

    let replay = repository
        .respond_interaction(
            &caller,
            "run-1",
            "interaction-1",
            &waiting.etag,
            &key,
            response.clone(),
        )
        .expect("response replay");
    assert!(replay.replayed);
    assert_eq!(replay.value.status, InteractionStatus::Responded);

    let rotated_without_prompt_scope =
        caller_context("app:desktop-ui", "request-rotated", &[scopes::RUNS_READ]);
    let scope_error = repository
        .respond_interaction(
            &rotated_without_prompt_scope,
            "run-1",
            "interaction-1",
            &waiting.etag,
            &key,
            response,
        )
        .expect_err("idempotent replay must enforce the current response scope");
    assert_eq!(scope_error.code, ApiErrorCode::PermissionDenied);
    assert_eq!(scope_error.reason, ApiErrorReason::ScopeDenied);

    let other = caller_context(
        "app:other-ui",
        "request-other",
        &[scopes::PROMPTS_RESPOND, scopes::RUNS_READ],
    );
    let error = repository
        .respond_interaction(
            &other,
            "run-1",
            "interaction-1",
            &waiting.etag,
            &IdempotencyKey::new("other-key").expect("key"),
            InteractionResponse::Prompt {
                answer: "Yes".into(),
                selected_index: Some(0),
            },
        )
        .expect_err("wrong principal");
    assert_eq!(error.reason, ApiErrorReason::RunNotFound);
}

#[test]
fn persisted_private_approval_display_fails_closed_on_reconstruction_and_feed_replay() {
    let (journal, repository, caller) = fixture();
    let request = create_request("private-approval-create", "Request approval");
    create_run(
        &repository,
        &caller,
        &request,
        "run-private-approval",
        "session-private-approval",
    );
    repository
        .append_update(
            &caller,
            "run-private-approval",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");
    let prior = repository
        .get_run(&caller, "run-private-approval")
        .expect("read prior run")
        .expect("prior run");
    let private_action = "filesystem.write.customer-secret";
    let private_resource = "/Users/alex/private/customer-secret.txt";
    let malicious = RunUpdateKind::Interaction {
        interaction: Interaction {
            id: "interaction-private-approval".into(),
            kind: InteractionKind::Approval,
            status: InteractionStatus::Pending,
            application_id: "app:desktop-ui".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            prompt: "An effect requires explicit approval".into(),
            choices: Vec::new(),
            allow_free_form: false,
            request_hash: Some("ab".repeat(32)),
            action: Some(private_action.into()),
            resource: Some(private_resource.into()),
            risk: Some(ApprovalRisk::High),
            expires_at: "2999-01-01T00:00:00Z".into(),
            response: None,
            responded_at: None,
        },
    };
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "api-run:run-private-approval".into(),
            expected_stream_version: 2,
            classification: EventClassification::Domain,
            event_type: "api.run.update.v1".into(),
            actor: caller.actor(),
            context: ExecutionContext {
                correlation_id: "persisted-private-approval".into(),
                session_id: Some("session-private-approval".into()),
                run_id: Some("run-private-approval".into()),
                ..ExecutionContext::default()
            },
            payload: crate::repository::stored_update_payload_for_test(
                &caller,
                &prior,
                &[RunUpdateKind::State {
                    status: RunStatus::Running,
                }],
                malicious,
            )
            .expect("malicious persisted update"),
        })
        .expect("simulate a pre-fix persisted approval");

    for error in [
        repository
            .get_run(&caller, "run-private-approval")
            .expect_err("reconstruction must reject private approval display"),
        repository
            .updates_after(&caller, "run-private-approval", 2, 1)
            .expect_err("feed replay must reject private approval display"),
    ] {
        assert_eq!(error.code, ApiErrorCode::Internal);
        assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
        let display = error.to_string();
        assert!(!display.contains(private_action));
        assert!(!display.contains(private_resource));
        assert!(!display.contains("customer-secret"));
    }
}

#[test]
fn run_ownership_isolated_by_application_but_survives_credential_rotation() {
    let (_, repository, owner) = fixture();
    let request = create_request("create-key", "Private run");
    create_run(
        &repository,
        &owner,
        &request,
        "run-private",
        "session-private",
    );

    let foreign = caller_context(
        "app:other-ui",
        "foreign-request",
        &[
            scopes::RUNS_READ,
            scopes::RUNS_EXECUTE,
            scopes::RUNS_CONTROL,
        ],
    );
    assert!(
        repository
            .get_run(&foreign, "run-private")
            .expect("foreign get")
            .is_none()
    );
    assert!(
        repository
            .execution_request(&foreign, "run-private")
            .expect("foreign execution request")
            .is_none()
    );
    assert!(
        repository
            .list_runs(
                &foreign,
                &ListRunsRequest {
                    session_id: None,
                    statuses: Vec::new(),
                    page_size: 10,
                    page_token: None,
                },
            )
            .expect("foreign list")
            .runs
            .is_empty()
    );
    let watch_error = repository
        .updates_after(&foreign, "run-private", 0, 10)
        .expect_err("foreign watch");
    assert_eq!(watch_error.reason, ApiErrorReason::RunNotFound);
    let update_error = repository
        .append_update(
            &foreign,
            "run-private",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect_err("foreign update");
    assert_eq!(update_error.reason, ApiErrorReason::RunNotFound);
    let cancel_error = repository
        .request_cancellation(
            &foreign,
            "run-private",
            &IdempotencyKey::new("foreign-cancel").expect("key"),
        )
        .expect_err("foreign cancel");
    assert_eq!(cancel_error.reason, ApiErrorReason::RunNotFound);

    let rotated_principal = ApplicationPrincipal::authenticated(
        "app:desktop-ui",
        "credential-rotated",
        ApplicationKind::Enrolled,
        [
            scope(scopes::RUNS_READ),
            scope(scopes::RUNS_EXECUTE),
            scope(scopes::RUNS_CONTROL),
        ],
        ["assistant".to_owned()],
        std::iter::empty(),
    )
    .expect("rotated principal");
    let rotated = CallerContext::authenticated(
        rotated_principal,
        RequestId::new("rotated-request").expect("request"),
    );
    let visible = repository
        .get_run(&rotated, "run-private")
        .expect("rotated get")
        .expect("owned run");
    assert_eq!(visible.id, "run-private");
    assert!(
        repository
            .execution_request(&rotated, "run-private")
            .expect("rotated execution request")
            .is_some()
    );
    repository
        .append_update(
            &rotated,
            "run-private",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("rotated update");
}

#[test]
fn orphan_recovery_uses_the_accepted_grant_without_borrowing_trigger_authority() {
    let (_, repository, owner) = fixture();
    let request = create_request("recovery-create", "Recover this run");
    create_run(
        &repository,
        &owner,
        &request,
        "run-recovery",
        "session-recovery",
    );

    let prompt_only_trigger = caller_context(
        "app:desktop-ui",
        "recovery-trigger",
        &[scopes::PROMPTS_RESPOND],
    );
    let (run, accepted) = repository
        .recoverable_run(&prompt_only_trigger, "run-recovery")
        .expect("same-application recovery lookup")
        .expect("recoverable run");
    assert_eq!(run.id, "run-recovery");
    assert_eq!(accepted.request, request);
    assert_eq!(accepted.application_id, "app:desktop-ui");
    assert_eq!(accepted.application_kind, ApplicationKind::Enrolled);
    assert!(
        accepted
            .scopes
            .iter()
            .any(|scope| scope.as_str() == scopes::RUNS_EXECUTE)
    );
    assert!(
        !prompt_only_trigger
            .principal()
            .has_scope(scopes::RUNS_EXECUTE)
    );

    let foreign_trigger = caller_context(
        "app:other-ui",
        "foreign-recovery-trigger",
        &[scopes::RUNS_CONTROL],
    );
    assert!(
        repository
            .recoverable_run(&foreign_trigger, "run-recovery")
            .expect("foreign lookup remains indistinguishable from absence")
            .is_none()
    );

    let unrelated_scope = caller_context(
        "app:desktop-ui",
        "invalid-recovery-trigger",
        &["system:read"],
    );
    let denial = repository
        .recoverable_run(&unrelated_scope, "run-recovery")
        .expect_err("a non-management scope cannot trigger recovery");
    assert_eq!(denial.code, ApiErrorCode::PermissionDenied);
    assert_eq!(denial.reason, ApiErrorReason::ScopeDenied);
}

#[test]
fn repeated_status_filter_and_opaque_pagination_are_stable() {
    let (_, repository, caller) = fixture();
    let first_request = create_request("create-1", "First");
    create_run(&repository, &caller, &first_request, "run-1", "session-1");
    let second_request = create_request("create-2", "Second");
    create_run(&repository, &caller, &second_request, "run-2", "session-2");
    repository
        .append_update(
            &caller,
            "run-2",
            1,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .expect("running");

    let queued = repository
        .list_runs(
            &caller,
            &ListRunsRequest {
                session_id: None,
                statuses: vec![RunStatus::Queued],
                page_size: 10,
                page_token: None,
            },
        )
        .expect("list");
    assert_eq!(queued.runs.len(), 1);
    assert_eq!(queued.runs[0].id, "run-1");

    let first_page = repository
        .list_runs(
            &caller,
            &ListRunsRequest {
                session_id: None,
                statuses: Vec::new(),
                page_size: 1,
                page_token: None,
            },
        )
        .expect("first page");
    assert_eq!(first_page.runs.len(), 1);
    let second_page = repository
        .list_runs(
            &caller,
            &ListRunsRequest {
                session_id: None,
                statuses: Vec::new(),
                page_size: 1,
                page_token: first_page.next_page_token,
            },
        )
        .expect("second page");
    assert_eq!(second_page.runs.len(), 1);
    assert_ne!(first_page.runs[0].id, second_page.runs[0].id);
}

#[test]
fn owner_index_listing_is_bounded_stable_and_independent_of_global_growth() {
    let journal = Arc::new(ReadCountingJournal::default());
    journal
        .append_batch(
            (0_u64..1_025)
                .map(|version| NewEvent {
                    event_version: 1,
                    stream_id: "unrelated-global-growth".into(),
                    expected_stream_version: version,
                    classification: EventClassification::System,
                    event_type: "test.unrelated.v1".into(),
                    actor: Actor {
                        actor_type: ActorType::System,
                        id: "test".into(),
                    },
                    context: ExecutionContext {
                        correlation_id: "unrelated-global-growth".into(),
                        ..ExecutionContext::default()
                    },
                    payload: serde_json::json!({"version": version}),
                })
                .collect(),
        )
        .expect("unrelated global growth");
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository = EventSourcedRunRepository::new(durable);
    let caller = caller_context(
        "app:indexed-ui",
        "request-indexed-list",
        &[scopes::RUNS_EXECUTE, scopes::RUNS_READ],
    );
    for index in 0..4 {
        let request = create_request(&format!("indexed-create-{index}"), "Indexed run");
        create_run(
            &repository,
            &caller,
            &request,
            &format!("run-indexed-{index}"),
            &format!("session-indexed-{index}"),
        );
    }

    journal.reset_read_counts();
    let request = ListRunsRequest {
        session_id: None,
        statuses: Vec::new(),
        page_size: 2,
        page_token: None,
    };
    let first = repository
        .list_runs(&caller, &request)
        .expect("first owner-index page");
    assert_eq!(
        first
            .runs
            .iter()
            .map(|run| run.id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-indexed-3", "run-indexed-2"]
    );
    let token = first.next_page_token.expect("continuation");
    assert_eq!(journal.global_reads.load(Ordering::Acquire), 0);
    assert_eq!(journal.full_stream_reads.load(Ordering::Acquire), 0);
    assert_eq!(journal.backwards_stream_reads.load(Ordering::Acquire), 1);
    assert_eq!(journal.backwards_events_returned.load(Ordering::Acquire), 4);

    let changed_filter = repository
        .list_runs(
            &caller,
            &ListRunsRequest {
                session_id: None,
                statuses: vec![RunStatus::Queued],
                page_size: 2,
                page_token: Some(token.clone()),
            },
        )
        .expect_err("cursor must remain bound to its original filters");
    assert_eq!(changed_filter.code, ApiErrorCode::InvalidArgument);
    assert_eq!(changed_filter.reason, ApiErrorReason::InvalidArgument);

    let newest_request = create_request("indexed-create-new", "New snapshot run");
    create_run(
        &repository,
        &caller,
        &newest_request,
        "run-indexed-new",
        "session-indexed-new",
    );
    journal.reset_read_counts();
    let second = repository
        .list_runs(
            &caller,
            &ListRunsRequest {
                page_token: Some(token),
                ..request
            },
        )
        .expect("second owner-index page");
    assert_eq!(
        second
            .runs
            .iter()
            .map(|run| run.id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-indexed-1", "run-indexed-0"]
    );
    assert!(second.next_page_token.is_none());
    assert_eq!(journal.global_reads.load(Ordering::Acquire), 0);
    assert_eq!(journal.full_stream_reads.load(Ordering::Acquire), 0);
}

#[test]
fn storage_mapping_never_exposes_adapter_or_uncertain_detail() {
    let request_id = RequestId::new("request-errors").expect("request");
    let error = ApiError::from_store(
        &StoreError::Adapter("password=do-not-leak /private/path".into()),
        &request_id,
    );
    let encoded = serde_json::to_string(&error).expect("encode");
    assert!(!encoded.contains("do-not-leak"));
    assert!(!encoded.contains("/private/path"));
    assert_eq!(error.reason, ApiErrorReason::StorageFailure);

    let uncertain = ApiError::from_store(
        &StoreError::OutcomeUnknown("private provider response".into()),
        &request_id,
    );
    assert_eq!(uncertain.outcome, OutcomeCertainty::Unknown);
    assert!(!uncertain.retryable);
    assert!(!uncertain.message.contains("private provider response"));
}
