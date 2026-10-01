use super::*;
use colossus_testkit::InMemoryEventJournal;

fn summary(lifetime: ProcessLifetime) -> ProcessSessionSummary {
    ProcessSessionSummary {
        id: Uuid::now_v7().to_string(),
        session_id: "chat-1".into(),
        run_id: "run-1".into(),
        owner: Actor {
            actor_type: ActorType::Application,
            id: "app-1".into(),
        },
        subagent_id: None,
        lifetime,
        status: ProcessSessionStatus::Starting,
        command: "safe command".into(),
        cwd: ".".into(),
        created_at_ms: now_ms(),
        deadline_ms: None,
        exit_code: None,
        reason: None,
        truncated: false,
        output_sequence: 0,
    }
}
fn context(run: &str, chat: &str) -> ExecutionContext {
    ExecutionContext {
        run_id: Some(run.into()),
        session_id: Some(chat.into()),
        ..ExecutionContext::default()
    }
}
fn registry(journal: Arc<dyn EventJournal>, directory: &Path) -> ProcessSessions {
    let lease = Arc::new(
        workspace_lease::WorkspaceOwnershipLease::acquire_at(directory, &directory.join("leases"))
            .expect("lease"),
    );
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new(["shell.run".into()]),
        [3; 32],
    ));
    ProcessSessions::open(
        journal,
        gateway,
        Arc::new(SandboxProcessExecutor::new(
            SandboxExecutorConfig {
                helper_executable: directory.join("missing-helper"),
                oci_runtime: None,
                oci_image: None,
                oci_proxy_image: None,
            },
            [4; 32],
        )),
        lease,
    )
    .expect("registry")
}
fn add(
    registry: &ProcessSessions,
    lifetime: ProcessLifetime,
    status: ProcessSessionStatus,
) -> Arc<ManagedSession> {
    let session = Arc::new(ManagedSession::new(
        Arc::clone(&registry.journal),
        context("run-1", "chat-1"),
        summary(lifetime),
    ));
    state(&session).summary.status = status;
    persistence::insert(
        &mut registry.registry(),
        &session,
        registry.journal.as_ref(),
    )
    .expect("insert");
    session
}

#[test]
fn run_end_stops_run_jobs_and_pending_background_jobs_but_preserves_running_background() {
    let directory = tempfile::tempdir().expect("root");
    let registry = registry(Arc::new(InMemoryEventJournal::default()), directory.path());
    let ordinary = add(
        &registry,
        ProcessLifetime::Run,
        ProcessSessionStatus::Running,
    );
    let pending = add(
        &registry,
        ProcessLifetime::Workspace,
        ProcessSessionStatus::Starting,
    );
    let background = add(
        &registry,
        ProcessLifetime::Workspace,
        ProcessSessionStatus::Running,
    );
    registry.cancel_run("run-1");
    assert!(ordinary.control.is_cancelled());
    assert!(pending.control.is_cancelled());
    assert!(!background.control.is_cancelled());
    drop(registry);
    assert!(
        background.control.is_cancelled(),
        "runtime drop must signal background cleanup"
    );
}

#[test]
fn later_turn_handles_are_bound_to_application_conversation_and_exact_agent_lineage() {
    let directory = tempfile::tempdir().expect("root");
    let registry = registry(Arc::new(InMemoryEventJournal::default()), directory.path());
    let session = add(
        &registry,
        ProcessLifetime::Workspace,
        ProcessSessionStatus::Running,
    );
    let id = state(&session).summary.id.clone();
    let owner = state(&session).summary.owner.clone();
    let next = context("run-2", "chat-1");
    registry
        .begin_run(&next, &owner, RunControl::default())
        .expect("run");
    assert!(registry.authorized(&id, &next).is_ok());
    let wrong_chat = context("run-3", "chat-2");
    registry
        .begin_run(&wrong_chat, &owner, RunControl::default())
        .expect("run");
    assert!(registry.authorized(&id, &wrong_chat).is_err());
    let other_app = context("run-4", "chat-1");
    registry
        .begin_run(
            &other_app,
            &Actor {
                id: "app-2".into(),
                ..owner
            },
            RunControl::default(),
        )
        .expect("run");
    assert!(registry.authorized(&id, &other_app).is_err());
    let child = ExecutionContext {
        subagent_id: Some("child".into()),
        ..next.clone()
    };
    assert!(registry.authorized(&id, &child).is_err());
    state(&session).summary.lifetime = ProcessLifetime::Run;
    assert!(registry.authorized(&id, &next).is_err());
}

#[tokio::test]
async fn cursors_report_retention_gaps_and_waits_do_not_change_the_deadline() {
    let session = ManagedSession::new(
        Arc::new(InMemoryEventJournal::default()),
        context("run-1", "chat-1"),
        summary(ProcessLifetime::Workspace),
    );
    {
        let mut current = state(&session);
        current.summary.deadline_ms = Some(123456);
        for _ in 0..300 {
            current.push(&vec![b'x'; 1024], b"stderr\n", false);
        }
    }
    let first = session.wait(0, 0, 16384).await.expect("snapshot");
    assert!(first.gap && first.session.truncated);
    assert!(!first.chunks.is_empty());
    let second = session
        .wait(first.next_sequence, 0, 65536)
        .await
        .expect("next");
    assert!(
        second
            .chunks
            .iter()
            .all(|chunk| chunk.sequence > first.next_sequence)
    );
    assert_eq!(second.session.deadline_ms, Some(123456));
    let start = std::time::Instant::now();
    let empty = session
        .wait(second.next_sequence, 20, 65536)
        .await
        .expect("yield");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(empty.chunks.is_empty());
    assert_eq!(empty.session.deadline_ms, Some(123456));
    assert!(session.wait(u64::MAX, 0, 65536).await.is_err());
}

#[test]
fn restart_records_interruption_without_adopting_a_process_or_replaying_a_command() {
    let directory = tempfile::tempdir().expect("root");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let registry = registry(Arc::clone(&journal), directory.path());
    let session = add(
        &registry,
        ProcessLifetime::Workspace,
        ProcessSessionStatus::Running,
    );
    let id = state(&session).summary.id.clone();
    drop(registry);
    let restored = persistence::recover(&journal).expect("recover");
    let recovered = restored.sessions.get(&id).expect("recovered");
    assert_eq!(
        state(recovered).summary.status,
        ProcessSessionStatus::Interrupted
    );
    assert!(state(recovered).logs_unavailable);
    assert!(recovered.done.load(Ordering::Acquire));
    assert!(!recovered.launched.load(Ordering::Acquire));
    assert_eq!(
        journal
            .read_stream(&format!("process-session:{id}"))
            .expect("events")
            .len(),
        2
    );
    persistence::recover(&journal).expect("idempotent recovery");
    assert_eq!(
        journal
            .read_stream(&format!("process-session:{id}"))
            .expect("events")
            .len(),
        2
    );
}

#[test]
fn known_startup_failure_and_unconfirmed_execution_have_distinct_states() {
    for (error, expected) in [
        (
            GatewayError::Execution("invalid process spec".into()),
            ProcessSessionStatus::Failed,
        ),
        (
            GatewayError::OutcomeUnknown("helper channel lost".into()),
            ProcessSessionStatus::OutcomeUnknown,
        ),
    ] {
        let session = ManagedSession::new(
            Arc::new(InMemoryEventJournal::default()),
            context("run-1", "chat-1"),
            summary(ProcessLifetime::Run),
        );
        session.executing.store(true, Ordering::Release);
        session.complete(Err(error));
        assert_eq!(state(&session).summary.status, expected);
    }
}

#[tokio::test]
async fn launch_evidence_precedes_policy_release_and_preserves_uncertainty() {
    struct Reject;
    #[async_trait]
    impl QuarantinedEffectObserver for Reject {
        async fn observe(&mut self, _: QuarantinedEffectResult) -> Result<(), ExecutionError> {
            Err(ExecutionError::Failed("release denied".into()))
        }
    }
    let session = ManagedSession::new(
        Arc::new(InMemoryEventJournal::default()),
        context("run-1", "chat-1"),
        summary(ProcessLifetime::Run),
    );
    let mut reject = Reject;
    let mut observer = LaunchObserver {
        session: &session,
        inner: &mut reject,
    };
    assert!(
        observer
            .observe(QuarantinedEffectResult {
                bytes: br#"{"kind":"started","deadline_ms":1234}"#.to_vec(),
                media_type: "application/json".into(),
                effect_succeeded: true,
            })
            .await
            .is_err()
    );
    assert!(session.launched.load(Ordering::Acquire));
    session.complete(Err(GatewayError::Denied("release denied".into())));
    assert_eq!(
        state(&session).summary.status,
        ProcessSessionStatus::OutcomeUnknown
    );
}
