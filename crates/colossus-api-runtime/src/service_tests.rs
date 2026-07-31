use crate::{
    PublicInteractionRouter, RunAdmissionConfig, RuntimeAgentRunApi,
    interactions::PublicInteractionRouter as InteractionRouter, service::ExecutionTestFault,
    writer::RunWriter,
};
use colossus_api::{
    AgentRunApi, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, CancelRunRequest,
    ContentPart, CreateArtifactUploadRequest, CreateRunRequest, EventSourcedArtifactApi,
    EventSourcedRunRepository, GetRunRequest, IdempotencyKey, InteractionKind, InteractionResponse,
    NewRun, OutcomeCertainty, PlanRunAction, PlanStatus as PublicPlanStatus, RequestId,
    RespondInteractionRequest, Run, RunMode, RunRepository, RunResult, RunStatus, RunUpdateKind,
    WatchRunRequest, scopes,
};
use colossus_contracts::{
    DecisionOutcome, EventEnvelope, NewEvent, PlanStep, PolicyDecision, PolicyObligations,
    ProjectionWorkItem, SignedCheckpoint, UserPromptRequest,
};
use colossus_policy::{DenyApproval, effect_request};
use colossus_ports::{
    ApprovalProvider, EventJournal, StoreError, UserPromptProvider, VerificationReport,
};
use colossus_runtime::{KeyConfig, Runtime, RuntimeConfig, RuntimeOpenOptions};
use colossus_testkit::InMemoryEventJournal;
use futures::StreamExt as _;
use sha2::{Digest as _, Sha256};
use std::{
    env, fs,
    process::Command,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;

struct RuntimeFixture {
    runtime: Arc<Runtime>,
    _directory: TempDir,
}

#[derive(Default)]
struct PostCommitReconciliationGapJournal {
    inner: InMemoryEventJournal,
    fail_next_batch_after_commit: AtomicBool,
    fail_next_stream_read: AtomicBool,
}

impl PostCommitReconciliationGapJournal {
    fn fail_next_batch_after_commit(&self) {
        self.fail_next_batch_after_commit
            .store(true, Ordering::Release);
    }
}

impl EventJournal for PostCommitReconciliationGapJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.inner.append(event)
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let persisted = self.inner.append_batch(events)?;
        if self
            .fail_next_batch_after_commit
            .swap(false, Ordering::AcqRel)
        {
            self.fail_next_stream_read.store(true, Ordering::Release);
            Err(StoreError::OutcomeUnknown(
                "the test batch committed before acknowledgement".into(),
            ))
        } else {
            Ok(persisted)
        }
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        if self.fail_next_stream_read.swap(false, Ordering::AcqRel) {
            return Err(StoreError::Adapter(
                "the test reconciliation read is unavailable".into(),
            ));
        }
        self.inner.read_stream(stream_id)
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_from(stream_id, after_version, limit)
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner
            .read_stream_backwards(stream_id, before_version, limit)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
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

fn runtime_fixture() -> RuntimeFixture {
    let directory = tempfile::tempdir().expect("runtime directory");
    // Keep every absolute fixture root in the same canonical namespace as
    // RuntimeOpenOptions. On macOS, tempfile may expose `/var/...` while the native
    // workspace picker/runtime identity resolves the same object as `/private/var/...`;
    // production skill roots intentionally reject symlinked absolute components.
    let root = fs::canonicalize(directory.path()).expect("canonical runtime directory");
    let state_path = root.join("state.redb");
    let anchor_path = root.join("anchor.json");
    let suffix = Uuid::now_v7().simple().to_string();
    let journal_variable = "COLOSSUS_API_RUNTIME_TEST_JOURNAL";
    let signing_variable = "COLOSSUS_API_RUNTIME_TEST_SIGNING";

    let mut config = RuntimeConfig::offline_template(&state_path);
    config.storage.keys = KeyConfig::Environment {
        journal_variable: journal_variable.into(),
        journal_key_id: format!("journal-{suffix}"),
        signing_variable: signing_variable.into(),
        anchor_path,
    };
    config.workflows.repository = root.join("workflows-bundled");
    config.workflows.user = root.join("workflows-user");
    config.skills.bundled = root.join("skills-bundled");
    config.skills.repository = root.join("skills-repository");
    config.skills.user = root.join("skills-user");
    config.packs.install_root = root.join("packs");
    for path in [
        &config.workflows.repository,
        &config.workflows.user,
        &config.skills.bundled,
        &config.skills.repository,
        &config.skills.user,
        &config.packs.install_root,
    ] {
        fs::create_dir_all(path).expect("fixture directory");
    }

    RuntimeFixture {
        runtime: Arc::new(
            Runtime::open_with_options(
                &config,
                Arc::new(DenyApproval),
                None,
                RuntimeOpenOptions::for_workspace(&root).expect("workspace options"),
            )
            .expect("runtime"),
        ),
        _directory: directory,
    }
}

fn caller(application_id: &str, request_id: &str) -> CallerContext {
    caller_with_additional_scopes(application_id, request_id, &[])
}

fn caller_with_additional_scopes(
    application_id: &str,
    request_id: &str,
    additional_scopes: &[&str],
) -> CallerContext {
    let scopes = [
        scopes::RUNS_EXECUTE,
        scopes::RUNS_READ,
        scopes::RUNS_CONTROL,
        scopes::PROMPTS_RESPOND,
        scopes::APPROVALS_RESPOND,
    ]
    .into_iter()
    .chain(additional_scopes.iter().copied())
    .map(|scope| ApiScope::new(scope).expect("scope"));
    CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            application_id,
            format!("credential-{request_id}"),
            ApplicationKind::Enrolled,
            scopes,
            ["primary".to_owned()],
            std::iter::empty(),
        )
        .expect("principal"),
        RequestId::new(request_id).expect("request id"),
    )
}

async fn caller_owned_text_artifacts_are_rendered_as_bounded_run_input(runtime: Arc<Runtime>) {
    use colossus_api::{ArtifactApi as _, ArtifactChunk, ArtifactPurpose};

    let service = service(Arc::clone(&runtime), RunAdmissionConfig::default());
    let application_id = format!("app:attachment-{}", Uuid::now_v7().simple());
    let owner = caller_with_additional_scopes(
        &application_id,
        "attachment-create",
        &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
    );
    let artifacts = EventSourcedArtifactApi::new(runtime.journal());
    let bytes = b"# Review\nrelease boundary".to_vec();
    let reservation = artifacts
        .create_upload(
            &owner,
            CreateArtifactUploadRequest {
                file_name: "review.md".into(),
                media_type: "text/markdown".into(),
                size_bytes: u64::try_from(bytes.len()).expect("artifact length"),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                purpose: ArtifactPurpose::RunInput,
                idempotency_key: IdempotencyKey::new("attachment-upload").expect("upload key"),
            },
        )
        .await
        .expect("reserve artifact");
    let artifact = artifacts
        .upload(
            &owner,
            &reservation.upload_id,
            vec![ArtifactChunk {
                offset: 0,
                data: bytes,
            }],
        )
        .await
        .expect("upload artifact");
    let created = service
        .create_run(
            &owner,
            CreateRunRequest {
                input: vec![
                    ContentPart::Text {
                        text: "Inspect the attachment".into(),
                    },
                    ContentPart::Artifact {
                        artifact_id: artifact.artifact_id,
                    },
                ],
                session_id: None,
                role: Some("primary".into()),
                mode: RunMode::Execute,
                skill_ids: Vec::new(),
                plan_action: None,
                max_turns: 1,
                idempotency_key: IdempotencyKey::new("attachment-run").expect("run key"),
            },
        )
        .await
        .expect("create attached run")
        .run;
    let terminal = wait_terminal(&service, &owner, &created.id).await;
    assert_eq!(terminal.status, RunStatus::Completed);
    wait_inactive(&service).await;
}

fn request(idempotency_key: &str, prompt: &str) -> CreateRunRequest {
    CreateRunRequest {
        input: vec![ContentPart::Text {
            text: prompt.into(),
        }],
        session_id: None,
        role: Some("primary".into()),
        mode: RunMode::Execute,
        skill_ids: Vec::new(),
        plan_action: None,
        max_turns: 1,
        idempotency_key: IdempotencyKey::new(idempotency_key).expect("idempotency key"),
    }
}

fn service(runtime: Arc<Runtime>, admission: RunAdmissionConfig) -> Arc<RuntimeAgentRunApi> {
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    Arc::new(RuntimeAgentRunApi::with_admission(
        runtime,
        interactions,
        "primary",
        "You are a deterministic conformance test.",
        admission,
    ))
}

async fn wait_inactive(service: &RuntimeAgentRunApi) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.active_execution_count() == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("public execution became inactive");
}

async fn wait_terminal(service: &RuntimeAgentRunApi, caller: &CallerContext, run_id: &str) -> Run {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = service
                .get_run(
                    caller,
                    GetRunRequest {
                        run_id: run_id.into(),
                    },
                )
                .await
                .expect("get run");
            if run.status.is_terminal() {
                return run;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run became terminal")
}

async fn plan_continuation_is_bound_to_owner_source_session_and_exact_revision(
    runtime: Arc<Runtime>,
) {
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(runtime.journal()));
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let service = Arc::new(RuntimeAgentRunApi::with_repository(
        Arc::clone(&runtime),
        Arc::clone(&repository),
        interactions,
        "primary",
        "Plan safely.",
    ));
    let owner = caller(
        &format!("app:plan-owner-{}", Uuid::now_v7().simple()),
        "plan-owner",
    );
    let bootstrap = service
        .create_run(
            &owner,
            request("plan-session-bootstrap", "Open the Plan session"),
        )
        .await
        .expect("create caller-owned session");
    let session = wait_terminal(&service, &owner, &bootstrap.run.id).await;
    let plan = runtime
        .create_plan(
            &session.session_id,
            "Plan the release",
            "A bounded release plan.",
            vec![PlanStep {
                index: 1,
                title: "Verify".into(),
                detail: "Run focused checks.".into(),
                requires_mutation: false,
            }],
        )
        .await
        .expect("draft Plan");
    let mut source_request = request("plan-source", "Create the Plan");
    source_request.session_id = Some(session.session_id.clone());
    source_request.mode = RunMode::Plan;
    let new_run = NewRun::from_request(
        "run-plan-source",
        &session.session_id,
        "primary",
        &source_request,
    )
    .expect("source run");
    let source = repository
        .create_run(&owner, &source_request, &new_run)
        .expect("create source")
        .value;
    let writer = RunWriter::new(
        Arc::clone(&repository),
        Arc::new(crate::feed::RunFeeds::default()),
        owner.clone(),
        &source,
    );
    writer
        .append(RunUpdateKind::State {
            status: RunStatus::Running,
        })
        .expect("start source");
    writer
        .append(RunUpdateKind::Result {
            result: RunResult {
                output: "Plan saved".into(),
                plan_id: Some(plan.id.clone()),
                plan_revision: Some(plan.revision),
                plan_status: Some(PublicPlanStatus::Draft),
                goal_id: None,
                profile: "offline".into(),
                model_profile: "offline".into(),
                provider_profile: "offline".into(),
                model: "offline".into(),
                elapsed_seconds: 0.1,
            },
        })
        .expect("complete source");

    let mut stale = request("stale-plan-revision", "Revise the Plan");
    stale.session_id = Some(session.session_id.clone());
    stale.mode = RunMode::Plan;
    stale.plan_action = Some(PlanRunAction::Revise {
        source_run_id: source.id.clone(),
        expected_revision: plan.revision + 1,
    });
    let error = service
        .create_run(&owner, stale)
        .await
        .expect_err("stale visible revision");
    assert_eq!(error.code, colossus_api::ApiErrorCode::InvalidArgument);

    let other = caller(
        &format!("app:other-plan-owner-{}", Uuid::now_v7().simple()),
        "other-plan-owner",
    );
    let mut foreign = request("foreign-plan-source", "Revise the Plan");
    foreign.session_id = Some(session.session_id.clone());
    foreign.mode = RunMode::Plan;
    foreign.plan_action = Some(PlanRunAction::Revise {
        source_run_id: source.id.clone(),
        expected_revision: plan.revision,
    });
    let error = service
        .create_run(&other, foreign)
        .await
        .expect_err("another application cannot resolve the source");
    assert_eq!(error.code, colossus_api::ApiErrorCode::NotFound);

    let mut revise = request("valid-plan-revision", "Clarify the verification step");
    revise.session_id = Some(session.session_id.clone());
    revise.mode = RunMode::Plan;
    revise.plan_action = Some(PlanRunAction::Revise {
        source_run_id: source.id,
        expected_revision: plan.revision,
    });
    let accepted = service
        .create_run(&owner, revise)
        .await
        .expect("owner-bound Plan continuation");
    assert_eq!(accepted.run.session_id, session.session_id);
    assert_eq!(accepted.run.mode, RunMode::Plan);
    service.request_shutdown();
}

async fn concurrent_exact_create_key_executes_the_provider_once(runtime: Arc<Runtime>) {
    let service = service(Arc::clone(&runtime), RunAdmissionConfig::default());
    let application_id = format!("app:create-{}", Uuid::now_v7().simple());
    let first_caller = caller(&application_id, "concurrent-create-one");
    let second_caller = caller(&application_id, "concurrent-create-two");
    let request = request(
        &format!("create-{}", Uuid::now_v7().simple()),
        "exactly once",
    );
    let barrier = Arc::new(Barrier::new(2));
    let handle = tokio::runtime::Handle::current();

    let first = {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        let handle = handle.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            barrier.wait();
            handle.block_on(service.create_run(&first_caller, request))
        })
    };
    let second = {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        tokio::task::spawn_blocking(move || {
            barrier.wait();
            handle.block_on(service.create_run(&second_caller, request))
        })
    };

    let first = first.await.expect("first task").expect("first create").run;
    let second = second
        .await
        .expect("second task")
        .expect("second create")
        .run;
    assert_eq!(first.id, second.id);

    let reader = caller(&application_id, "concurrent-create-reader");
    let terminal = wait_terminal(&service, &reader, &first.id).await;
    assert_eq!(terminal.status, RunStatus::Completed);
    wait_inactive(&service).await;
    let executions = runtime
        .journal()
        .read_stream(&format!("run:{}", first.id))
        .expect("canonical run stream")
        .into_iter()
        .filter(|event| event.event_type == "run.started.v1")
        .count();
    assert_eq!(executions, 1);
}

async fn cancellation_before_the_spawned_task_runs_is_terminal_at_turn_zero(runtime: Arc<Runtime>) {
    let service = service(runtime, RunAdmissionConfig::default());
    let application_id = format!("app:cancel-{}", Uuid::now_v7().simple());
    let creator = caller(&application_id, "cancel-create");
    let created = service
        .create_run(
            &creator,
            request(
                &format!("cancel-create-{}", Uuid::now_v7().simple()),
                "cancel before execution",
            ),
        )
        .await
        .expect("create")
        .run;

    let controller = caller(&application_id, "cancel-control");
    service
        .cancel_run(
            &controller,
            CancelRunRequest {
                run_id: created.id.clone(),
                idempotency_key: IdempotencyKey::new(format!("cancel-{}", Uuid::now_v7().simple()))
                    .expect("cancel key"),
            },
        )
        .await
        .expect("cancel");

    let terminal = wait_terminal(&service, &controller, &created.id).await;
    assert_eq!(terminal.status, RunStatus::Cancelled);
    assert_eq!(
        terminal.cancellation.expect("cancellation evidence").turn,
        0
    );
    wait_inactive(&service).await;
}

async fn execution_panic_is_durable_outcome_unknown_and_releases_admission(runtime: Arc<Runtime>) {
    let service = service(Arc::clone(&runtime), RunAdmissionConfig::default());
    service.inject_next_execution_fault(ExecutionTestFault::Panic);
    let application_id = format!("app:panic-{}", Uuid::now_v7().simple());
    let creator = caller(&application_id, "panic-create");
    let created = service
        .create_run(
            &creator,
            request(
                &format!("panic-{}", Uuid::now_v7().simple()),
                "panic after durable start",
            ),
        )
        .await
        .expect("create")
        .run;

    wait_inactive(&service).await;
    let repository = EventSourcedRunRepository::new(runtime.journal());
    let terminal = repository
        .get_run(&creator, &created.id)
        .expect("read panic outcome")
        .expect("run");
    assert_eq!(terminal.status, RunStatus::OutcomeUnknown);
    assert_eq!(
        terminal.failure.expect("failure").outcome,
        OutcomeCertainty::Unknown
    );
    assert_eq!(service.active_execution_count(), 0);
}

async fn failed_terminal_append_recovers_conservatively_and_wakes_watchers(runtime: Arc<Runtime>) {
    let service = service(Arc::clone(&runtime), RunAdmissionConfig::default());
    service.inject_next_execution_fault(ExecutionTestFault::FailTerminalAppend);
    let application_id = format!("app:terminal-{}", Uuid::now_v7().simple());
    let creator = caller(&application_id, "terminal-create");
    let created = service
        .create_run(
            &creator,
            request(
                &format!("terminal-{}", Uuid::now_v7().simple()),
                "drop final public append",
            ),
        )
        .await
        .expect("create")
        .run;

    let mut watch = service
        .watch_run(
            &creator,
            WatchRunRequest {
                run_id: created.id.clone(),
                after_sequence: 0,
            },
        )
        .await
        .expect("watch");
    let terminal_update = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let update = watch
                .next()
                .await
                .expect("watch remains open")
                .expect("watch update");
            if matches!(
                &update.kind,
                RunUpdateKind::Failure {
                    status: RunStatus::OutcomeUnknown,
                    ..
                }
            ) {
                return update;
            }
        }
    })
    .await
    .expect("watch received conservative terminal state");
    assert!(matches!(
        terminal_update.kind,
        RunUpdateKind::Failure {
            status: RunStatus::OutcomeUnknown,
            ..
        }
    ));
    assert!(watch.next().await.is_none());

    wait_inactive(&service).await;
    assert_eq!(service.active_execution_count(), 0);
    let repository = EventSourcedRunRepository::new(runtime.journal());
    let recovered = repository
        .get_run(&creator, &created.id)
        .expect("read recovered run")
        .expect("run");
    assert_eq!(recovered.status, RunStatus::OutcomeUnknown);
    assert_eq!(
        recovered.failure.expect("failure").outcome,
        OutcomeCertainty::Unknown
    );
}

async fn dropping_a_watch_releases_its_per_application_permit(runtime: Arc<Runtime>) {
    let admission = RunAdmissionConfig::default()
        .with_watch_limits(1, 1)
        .expect("watch limits");
    let service = service(runtime, admission);
    let application_id = format!("app:watch-{}", Uuid::now_v7().simple());
    let creator = caller(&application_id, "watch-create");
    let created = service
        .create_run(
            &creator,
            request(
                &format!("watch-{}", Uuid::now_v7().simple()),
                "complete for watch",
            ),
        )
        .await
        .expect("create")
        .run;
    let terminal = wait_terminal(&service, &creator, &created.id).await;

    let first = service
        .watch_run(
            &creator,
            WatchRunRequest {
                run_id: terminal.id.clone(),
                after_sequence: 0,
            },
        )
        .await
        .expect("first watch");
    drop(first);

    let mut second = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match service
                .watch_run(
                    &creator,
                    WatchRunRequest {
                        run_id: terminal.id.clone(),
                        after_sequence: terminal.last_sequence,
                    },
                )
                .await
            {
                Ok(stream) => return stream,
                Err(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("watch permit released");
    assert!(second.next().await.is_none());
}

async fn orphan_recovery_above_the_application_cap_drains_in_order(runtime: Arc<Runtime>) {
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(runtime.journal()));
    let admission = RunAdmissionConfig::new(1, 1, 100, 100, 100, 100).expect("admission");
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let service = RuntimeAgentRunApi::with_repository_and_admission(
        Arc::clone(&runtime),
        Arc::clone(&repository),
        interactions,
        "primary",
        "You are a deterministic recovery test.",
        admission,
    );
    let application_id = format!("app:recovery-{}", Uuid::now_v7().simple());
    let owner = caller(&application_id, "recovery-owner");

    let first_request = request(
        &format!("orphan-one-{}", Uuid::now_v7().simple()),
        "first orphan",
    );
    let second_request = request(
        &format!("orphan-two-{}", Uuid::now_v7().simple()),
        "second orphan",
    );
    let first_new = NewRun::from_request(
        Uuid::now_v7().to_string(),
        Uuid::now_v7().to_string(),
        "primary",
        &first_request,
    )
    .expect("first run");
    let second_new = NewRun::from_request(
        Uuid::now_v7().to_string(),
        Uuid::now_v7().to_string(),
        "primary",
        &second_request,
    )
    .expect("second run");
    let first = repository
        .create_run(&owner, &first_request, &first_new)
        .expect("first orphan")
        .value;
    let second = repository
        .create_run(&owner, &second_request, &second_new)
        .expect("second orphan")
        .value;

    service
        .get_run(
            &owner,
            GetRunRequest {
                run_id: first.id.clone(),
            },
        )
        .await
        .expect("recover first");
    service
        .get_run(
            &owner,
            GetRunRequest {
                run_id: second.id.clone(),
            },
        )
        .await
        .expect("queue second recovery");
    assert_eq!(service.active_execution_count(), 1);
    assert_eq!(service.pending_recovery_count(), 1);

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.active_execution_count() == 0 && service.pending_recovery_count() == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("queued recoveries drained");

    for run_id in [first.id, second.id] {
        let run = repository
            .get_run(&owner, &run_id)
            .expect("read recovered run")
            .expect("run");
        assert_eq!(run.status, RunStatus::Completed);
        let starts = runtime
            .journal()
            .read_stream(&format!("run:{run_id}"))
            .expect("canonical run")
            .into_iter()
            .filter(|event| event.event_type == "run.started.v1")
            .count();
        assert_eq!(starts, 1);
    }
}

async fn committed_interaction_retry_resynchronizes_and_delivers(runtime: Arc<Runtime>) {
    let journal = Arc::new(PostCommitReconciliationGapJournal::default());
    let durable: Arc<dyn EventJournal> = journal.clone();
    let repository: Arc<dyn RunRepository> = Arc::new(EventSourcedRunRepository::new(durable));
    let owner = caller(
        &format!("app:replay-{}", Uuid::now_v7().simple()),
        "replay-owner",
    );
    let create = request(
        &format!("replay-{}", Uuid::now_v7().simple()),
        "interaction replay fixture",
    );
    let new_run =
        NewRun::from_request("replay-run", "replay-session", "primary", &create).expect("new run");
    let run = repository
        .create_run(&owner, &create, &new_run)
        .expect("create")
        .value;
    let writer = Arc::new(RunWriter::new(
        Arc::clone(&repository),
        Arc::new(crate::feed::RunFeeds::default()),
        owner.clone(),
        &run,
    ));
    writer
        .append(RunUpdateKind::State {
            status: RunStatus::Running,
        })
        .expect("start");
    let interactions = Arc::new(
        InteractionRouter::new(Arc::new(DenyApproval), None).with_timeout(Duration::from_secs(2)),
    );
    let service = RuntimeAgentRunApi::with_repository(
        runtime,
        Arc::clone(&repository),
        Arc::clone(&interactions),
        "primary",
        "interaction replay test",
    );
    service.attach_active_writer_for_test(run.id.clone(), Arc::clone(&writer));

    let waiter = {
        let interactions = Arc::clone(&interactions);
        let writer = Arc::clone(&writer);
        tokio::spawn(async move {
            interactions
                .scope(Arc::clone(&writer), async {
                    interactions
                        .prompt(UserPromptRequest {
                            question: "Continue?".into(),
                            choices: vec!["yes".into()],
                            allow_free_form: false,
                        })
                        .await
                })
                .await
        })
    };
    let pending = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let run = repository
                .get_run(&owner, "replay-run")
                .expect("read interaction")
                .expect("run");
            if let Some(interaction) = run.pending_interaction {
                return (run.etag, interaction);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pending interaction");
    let response = RespondInteractionRequest {
        run_id: "replay-run".into(),
        interaction_id: pending.1.id,
        etag: pending.0,
        idempotency_key: IdempotencyKey::new("replay-response-key").expect("response key"),
        response: InteractionResponse::Prompt {
            answer: "yes".into(),
            selected_index: Some(0),
        },
    };

    journal.fail_next_batch_after_commit();
    let first = service
        .respond_interaction(&owner, response.clone())
        .await
        .expect_err("first acknowledgement is outcome unknown");
    assert_eq!(first.reason, colossus_api::ApiErrorReason::OutcomeUnknown);

    let replay = service
        .respond_interaction(&owner, response)
        .await
        .expect("retry resolves the durable response");
    assert_eq!(replay.status, colossus_api::InteractionStatus::Responded);
    let answer = waiter.await.expect("waiter task").expect("prompt response");
    assert_eq!(answer.answer, "yes");
    assert_eq!(answer.selected_index, Some(0));

    let resumed = repository
        .get_run(&owner, "replay-run")
        .expect("read resumed run")
        .expect("run");
    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(resumed.last_sequence, 5);
}

#[test]
fn runtime_service_conformance() {
    const CHILD_MARKER: &str = "COLOSSUS_API_RUNTIME_TEST_CHILD";
    if env::var_os(CHILD_MARKER).is_none() {
        let status = Command::new(env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "service_tests::runtime_service_conformance",
                "--nocapture",
            ])
            .env(CHILD_MARKER, "1")
            .env("COLOSSUS_API_RUNTIME_TEST_JOURNAL", "11".repeat(32))
            .env("COLOSSUS_API_RUNTIME_TEST_SIGNING", "22".repeat(32))
            .status()
            .expect("spawn isolated runtime conformance process");
        assert!(status.success(), "runtime conformance child failed");
        return;
    }

    let fixture = runtime_fixture();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(async {
            concurrent_exact_create_key_executes_the_provider_once(Arc::clone(&fixture.runtime))
                .await;
            cancellation_before_the_spawned_task_runs_is_terminal_at_turn_zero(Arc::clone(
                &fixture.runtime,
            ))
            .await;
            execution_panic_is_durable_outcome_unknown_and_releases_admission(Arc::clone(
                &fixture.runtime,
            ))
            .await;
            failed_terminal_append_recovers_conservatively_and_wakes_watchers(Arc::clone(
                &fixture.runtime,
            ))
            .await;
            committed_interaction_retry_resynchronizes_and_delivers(Arc::clone(&fixture.runtime))
                .await;
            dropping_a_watch_releases_its_per_application_permit(Arc::clone(&fixture.runtime))
                .await;
            orphan_recovery_above_the_application_cap_drains_in_order(Arc::clone(&fixture.runtime))
                .await;
            caller_owned_text_artifacts_are_rendered_as_bounded_run_input(Arc::clone(
                &fixture.runtime,
            ))
            .await;
            plan_continuation_is_bound_to_owner_source_session_and_exact_revision(Arc::clone(
                &fixture.runtime,
            ))
            .await;
        });
}

#[tokio::test(flavor = "current_thread")]
async fn durable_response_beats_expiry_even_when_delivery_is_delayed() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(Arc::clone(&journal)));
    let owner = caller(
        &format!("app:response-{}", Uuid::now_v7().simple()),
        "response-owner",
    );
    let create = request(
        &format!("response-{}", Uuid::now_v7().simple()),
        "interaction fixture",
    );
    let new_run = NewRun::from_request("response-run", "response-session", "primary", &create)
        .expect("new run");
    let run = repository
        .create_run(&owner, &create, &new_run)
        .expect("create")
        .value;
    let writer = Arc::new(RunWriter::new(
        Arc::clone(&repository),
        Arc::new(crate::feed::RunFeeds::default()),
        owner.clone(),
        &run,
    ));
    writer
        .append(RunUpdateKind::State {
            status: RunStatus::Running,
        })
        .expect("start");
    let router = Arc::new(
        InteractionRouter::new(Arc::new(DenyApproval), None)
            .with_timeout(Duration::from_millis(100)),
    );

    let waiter = {
        let router = Arc::clone(&router);
        let writer = Arc::clone(&writer);
        tokio::spawn(async move {
            router
                .scope(Arc::clone(&writer), async {
                    router
                        .prompt(UserPromptRequest {
                            question: "Continue?".into(),
                            choices: vec!["yes".into()],
                            allow_free_form: false,
                        })
                        .await
                })
                .await
        })
    };
    let pending = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let run = repository
                .get_run(&owner, "response-run")
                .expect("read interaction")
                .expect("run");
            if let Some(interaction) = run.pending_interaction {
                return (run.etag, interaction);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pending interaction");
    let response = InteractionResponse::Prompt {
        answer: "yes".into(),
        selected_index: Some(0),
    };
    let resolved = writer
        .respond_interaction(
            &owner,
            &pending.1.id,
            &pending.0,
            &IdempotencyKey::new("response-race-key").expect("response key"),
            response,
        )
        .expect("durable response");

    // Delay the in-memory handoff until after the deadline. The expiry append must
    // lose against the response already serialized by RunWriter.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(router.deliver("response-run", &resolved));
    let answer = waiter.await.expect("waiter task").expect("prompt response");
    assert_eq!(answer.answer, "yes");
    assert_eq!(answer.selected_index, Some(0));

    let updates = repository
        .updates_after(&owner, "response-run", 0, 16)
        .expect("updates");
    assert!(!updates.iter().any(|update| {
        matches!(
            &update.kind,
            RunUpdateKind::Interaction { interaction }
                if interaction.status == colossus_api::InteractionStatus::Expired
        )
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn public_approval_interactions_persist_without_prompt_choices() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(Arc::clone(&journal)));
    let owner = caller(
        &format!("app:approval-{}", Uuid::now_v7().simple()),
        "approval-owner",
    );
    let create = request(
        &format!("approval-{}", Uuid::now_v7().simple()),
        "approval fixture",
    );
    let new_run = NewRun::from_request("approval-run", "approval-session", "primary", &create)
        .expect("new run");
    let run = repository
        .create_run(&owner, &create, &new_run)
        .expect("create")
        .value;
    let writer = Arc::new(RunWriter::new(
        Arc::clone(&repository),
        Arc::new(crate::feed::RunFeeds::default()),
        owner.clone(),
        &run,
    ));
    writer
        .append(RunUpdateKind::State {
            status: RunStatus::Running,
        })
        .expect("start");
    let router = Arc::new(
        InteractionRouter::new(Arc::new(DenyApproval), None).with_timeout(Duration::from_secs(2)),
    );
    let effect = effect_request(
        owner.actor(),
        "filesystem.write",
        "/private/customer-secret.txt",
        serde_json::json!({"private": true}),
    );
    let decision = PolicyDecision {
        decision_id: "approval-decision".into(),
        policy_revision: "approval-test-v1".into(),
        outcome: DecisionOutcome::RequireApproval,
        reason: "explicit approval required".into(),
        obligations: PolicyObligations::default(),
    };
    let waiter = {
        let router = Arc::clone(&router);
        let writer = Arc::clone(&writer);
        tokio::spawn(async move {
            router
                .scope(Arc::clone(&writer), async {
                    router
                        .request_approval(
                            &effect,
                            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            &decision,
                        )
                        .await
                })
                .await
        })
    };

    let pending = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let run = repository
                .get_run(&owner, "approval-run")
                .expect("read interaction")
                .expect("run");
            if let Some(interaction) = run.pending_interaction {
                return interaction;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("approval interaction persisted");
    assert_eq!(pending.kind, InteractionKind::Approval);
    assert!(pending.choices.is_empty());
    assert!(!pending.allow_free_form);

    router.cancel_run("approval-run");
    assert!(
        waiter
            .await
            .expect("approval task")
            .expect_err("cancelled approval")
            .to_string()
            .contains("public approval")
    );
}
