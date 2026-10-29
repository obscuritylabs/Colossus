use super::*;
use crate::terminal_actor;
use colossus_testkit::InMemoryEventJournal;
use colossus_work::CreateSubagentRequest;

fn fixture() -> (
    Arc<NativeSessionLifecycle>,
    Arc<dyn SessionRepository>,
    Arc<dyn WorkRepository>,
) {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let work: Arc<dyn WorkRepository> =
        Arc::new(EventSourcedWorkRepository::new(Arc::clone(&journal)));
    sessions
        .create_session("chat", None, terminal_actor())
        .unwrap();
    (
        Arc::new(NativeSessionLifecycle::new(
            Arc::clone(&sessions),
            Arc::clone(&work),
            journal,
        )),
        sessions,
        work,
    )
}

fn context(run: &str, session: &str) -> ExecutionContext {
    ExecutionContext {
        run_id: Some(run.into()),
        session_id: Some(session.into()),
        ..Default::default()
    }
}

#[test]
fn concurrent_runs_keep_session_busy_until_every_owner_finishes() {
    let (lifecycle, _, _) = fixture();
    lifecycle.begin_run(&context("one", "chat")).unwrap();
    lifecycle.begin_run(&context("two", "chat")).unwrap();
    assert!(lifecycle.begin_run(&context("two", "chat")).is_err());
    assert!(
        lifecycle
            .set_archived("chat", true, terminal_actor())
            .is_err()
    );
    lifecycle.finish_run("one");
    assert!(
        lifecycle
            .set_archived("chat", true, terminal_actor())
            .is_err()
    );
    lifecycle.finish_run("two");
    lifecycle
        .set_archived("chat", true, terminal_actor())
        .unwrap();
    assert!(lifecycle.begin_run(&context("three", "chat")).is_err());
    lifecycle
        .set_archived("chat", false, terminal_actor())
        .unwrap();
    lifecycle.begin_run(&context("three", "chat")).unwrap();
}

#[test]
fn archive_and_start_race_has_exactly_one_winner() {
    let (lifecycle, sessions, _) = fixture();
    for iteration in 0..32 {
        let session = format!("race-{iteration}");
        sessions
            .create_session(&session, None, terminal_actor())
            .unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let archive = scope.spawn(|| {
                barrier.wait();
                lifecycle
                    .set_archived(&session, true, terminal_actor())
                    .is_ok()
            });
            barrier.wait();
            let admitted = lifecycle
                .begin_run(&context(&format!("run-{iteration}"), &session))
                .is_ok();
            assert_ne!(archive.join().unwrap(), admitted);
        });
    }
}

#[test]
fn queued_children_and_cancelled_children_still_cleaning_up_block_parent_archive() {
    let (lifecycle, sessions, work) = fixture();
    let service = WorkService::new(Arc::clone(&work), sessions);
    let child = service
        .create_subagent(
            CreateSubagentRequest {
                session_id: "chat".into(),
                parent_run_id: "parent".into(),
                parent_call_id: "delegate".into(),
                task: "read safely".into(),
                role: "default".into(),
                allowed_tools: None,
                instruction_snapshot_id: None,
            },
            terminal_actor(),
        )
        .unwrap();
    assert!(
        lifecycle
            .set_archived("chat", true, terminal_actor())
            .is_err()
    );
    let mut child_context = context("child-run", &child.child_session_id);
    child_context.subagent_id = Some(child.id.clone());
    lifecycle.begin_run(&child_context).unwrap();
    service
        .stop_subagent(
            &child.id,
            SubagentStatus::Cancelled,
            "operator stopped",
            terminal_actor(),
        )
        .unwrap();
    assert!(
        lifecycle
            .set_archived("chat", true, terminal_actor())
            .is_err()
    );
    lifecycle.finish_run("child-run");
    lifecycle
        .set_archived("chat", true, terminal_actor())
        .unwrap();
}

#[test]
fn completed_child_background_process_blocks_exact_parent_and_known_ancestor_archive() {
    use crate::process_sessions::ActiveProcessOwner;
    let (lifecycle, sessions, work) = fixture();
    let service = WorkService::new(Arc::clone(&work), sessions);
    let completed = |session: &str, parent_run: &str, run: &str| {
        let child = service
            .create_subagent(
                CreateSubagentRequest {
                    session_id: session.into(),
                    parent_run_id: parent_run.into(),
                    parent_call_id: format!("call-{run}"),
                    task: "bounded background fixture".into(),
                    role: "default".into(),
                    allowed_tools: None,
                    instruction_snapshot_id: None,
                },
                terminal_actor(),
            )
            .unwrap();
        service.start_subagent(&child.id, terminal_actor()).unwrap();
        service
            .complete_subagent(&child.id, run, "model finished", terminal_actor())
            .unwrap()
    };
    let child = completed("chat", "parent-run", "child-run");
    let owner = ActiveProcessOwner {
        session_id: child.child_session_id.clone(),
        subagent_id: Some(child.id.clone()),
    };
    assert_eq!(child.status, SubagentStatus::Completed);
    assert!(
        lifecycle
            .ensure_processes_idle("chat", std::slice::from_ref(&owner))
            .is_err()
    );
    assert!(
        lifecycle
            .ensure_processes_idle("independent-chat", std::slice::from_ref(&owner))
            .is_ok()
    );
    assert!(lifecycle.ensure_processes_idle("chat", &[]).is_ok());

    // Imported canonical lineage can include an ancestor even though new nested
    // model delegation is disabled. Resolve its exact parent run, without scans.
    let grandchild = completed(&child.child_session_id, "child-run", "grandchild-run");
    lifecycle
        .journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "run:child-run".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "model.request.prepared.v1".into(),
            actor: terminal_actor(),
            context: ExecutionContext {
                run_id: Some("child-run".into()),
                session_id: Some(child.child_session_id),
                subagent_id: Some(child.id),
                ..Default::default()
            },
            payload: json!({}),
        })
        .unwrap();
    let owner = ActiveProcessOwner {
        session_id: grandchild.child_session_id,
        subagent_id: Some(grandchild.id),
    };
    assert!(
        lifecycle
            .ensure_processes_idle("chat", std::slice::from_ref(&owner))
            .is_err()
    );
    assert!(
        lifecycle
            .ensure_processes_idle("independent-chat", std::slice::from_ref(&owner))
            .is_ok()
    );
    assert!(
        lifecycle
            .ensure_processes_idle(
                "chat",
                &[ActiveProcessOwner {
                    session_id: "wrong-session".into(),
                    subagent_id: owner.subagent_id.clone()
                }]
            )
            .is_err()
    );
    assert!(
        lifecycle
            .ensure_processes_idle(
                "chat",
                &[ActiveProcessOwner {
                    session_id: "unknown-session".into(),
                    subagent_id: Some("unknown-job".into())
                }]
            )
            .is_err()
    );
}

#[tokio::test]
async fn normal_operation_errors_release_but_dropped_unconfirmed_work_stays_fenced() {
    let (lifecycle, _, _) = fixture();
    assert!(
        lifecycle
            .during("chat", async { Err::<(), _>("ordinary error") })
            .await
            .unwrap()
            .is_err()
    );
    lifecycle
        .set_archived("chat", true, terminal_actor())
        .unwrap();
    lifecycle
        .set_archived("chat", false, terminal_actor())
        .unwrap();
    let worker = Arc::clone(&lifecycle);
    let (ready, started) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        worker
            .during("chat", async {
                ready.send(()).unwrap();
                std::future::pending::<()>().await
            })
            .await
    });
    started.await.unwrap();
    assert!(
        lifecycle
            .set_archived("chat", true, terminal_actor())
            .is_err()
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        lifecycle
            .set_archived("chat", true, terminal_actor())
            .is_err()
    );
}

#[tokio::test]
async fn runtime_archived_history_is_readable_but_cannot_start_model_shell_goal_or_plan() {
    let directory = crate::test_support::private_tempdir();
    let mut config = crate::RuntimeConfig::offline_template(directory.path().join("state.redb"));
    config.use_ephemeral_storage();
    let runtime = crate::Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        crate::RuntimeOpenOptions::for_workspace(directory.path()).unwrap(),
    )
    .unwrap();
    let session = runtime.create_session(Some("retain work")).unwrap();
    let task = runtime
        .create_task(&session.id, "Keep task", "saved work", TaskStatus::Pending)
        .await
        .unwrap();
    let archive = runtime.set_session_archived(&session.id, true).unwrap();
    assert_eq!(archive.revision, 1);
    assert!(runtime.list_sessions(100).unwrap().is_empty());
    assert_eq!(
        runtime.list_archived_sessions(100).unwrap()[0].id,
        session.id
    );
    assert!(
        runtime
            .run_model_in_session("default", "", "must not append", Some(1), &session.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("archived")
    );
    assert!(
        runtime
            .run_user_shell(&session.id, "echo blocked", &RunControl::default())
            .await
            .unwrap_err()
            .to_string()
            .contains("archived")
    );
    assert!(
        runtime
            .run_goal("default", "must not create", &session.id, 1, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("archived")
    );
    assert!(
        runtime
            .execute_plan_stream_controlled(
                "default",
                &session.id,
                "absent-plan",
                1,
                PlanExecutionStrategy::Direct,
                Some(1),
                &mut IgnoreEvents,
                &RunControl::default()
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("archived")
    );
    assert!(runtime.session_messages(&session.id).unwrap().is_empty());
    assert_eq!(runtime.work.get_task(&task.id).unwrap(), Some(task));
    assert!(
        runtime
            .work
            .list_goals(Some(&session.id), None, 10)
            .unwrap()
            .is_empty()
    );
    runtime.set_session_archived(&session.id, false).unwrap();
    runtime
        .run_model_in_session("default", "", "hello again", Some(1), &session.id)
        .await
        .unwrap();
    assert!(!runtime.session_messages(&session.id).unwrap().is_empty());
    assert_eq!(
        runtime
            .session_lifecycle(&session.id)
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    runtime.set_session_archived(&session.id, true).unwrap();
}

struct IgnoreEvents;
#[async_trait]
impl RunEventObserver for IgnoreEvents {
    async fn observe(&mut self, _: RunEventEnvelope) -> Result<(), ModelProviderError> {
        Ok(())
    }
}
