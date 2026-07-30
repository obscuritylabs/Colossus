use super::*;
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::{InMemoryEventJournal, assert_work_repository_conformance};

#[test]
fn event_sourced_work_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    assert_work_repository_conformance(|| {
        Box::new(EventSourcedWorkRepository::new(Arc::clone(&journal)))
    });
}

fn fixture() -> (Arc<dyn EventJournal>, Arc<dyn WorkRepository>, WorkService) {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    sessions
        .create_session("session-1", Some("work"), user_actor())
        .expect("session");
    let repository: Arc<dyn WorkRepository> =
        Arc::new(EventSourcedWorkRepository::new(Arc::clone(&journal)));
    let service = WorkService::new(Arc::clone(&repository), sessions);
    (journal, repository, service)
}

#[test]
fn tasks_reconstruct_after_updates_and_repository_restart() {
    let (journal, repository, service) = fixture();
    let created = service
        .create_task(
            "session-1",
            "Implement work state",
            "Use immutable events",
            TaskStatus::Pending,
            user_actor(),
        )
        .expect("create");
    let updated = service
        .update_task(
            &created.id,
            None,
            Some("Repository and projections"),
            Some(TaskStatus::InProgress),
            user_actor(),
        )
        .expect("update");
    assert_eq!(updated.status, TaskStatus::InProgress);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(
        repository
            .list_tasks(Some("session-1"), None, 10)
            .expect("list")
            .len(),
        1
    );

    let reopened = EventSourcedWorkRepository::new(journal);
    assert_eq!(
        reopened.get_task(&created.id).expect("get").expect("task"),
        updated
    );
}

#[test]
fn active_decisions_update_archive_and_filter_without_deletion() {
    let (_journal, repository, service) = fixture();
    let created = service
        .create_decision(
            "session-1",
            "Audit first",
            "All durable mutations use immutable events.",
            DecisionSource::User,
            DecisionPriority::Critical,
            "Preserve evidence",
            "When changing canonical state",
            "Auditability is foundational",
            "I want auditing from the ground up",
            None,
            None,
            None,
            user_actor(),
        )
        .expect("create");
    let updated = service
        .update_decision(
            &created.id,
            None,
            Some("All state changes use immutable canonical events."),
            Some(DecisionPriority::High),
            None,
            None,
            None,
            None,
            user_actor(),
        )
        .expect("update");
    assert_eq!(updated.priority, DecisionPriority::High);
    assert_eq!(
        repository
            .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
            .expect("active")
            .len(),
        1
    );
    let archived = service
        .archive_decision(&created.id, user_actor())
        .expect("archive");
    assert_eq!(archived.status, DecisionStatus::Archived);
    assert!(
        repository
            .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
            .expect("active")
            .is_empty()
    );
    assert_eq!(
        repository
            .get_decision(&created.id)
            .expect("get")
            .expect("decision")
            .status,
        DecisionStatus::Archived
    );
}

#[test]
fn supersession_is_atomic_and_preserves_lineage() {
    let (journal, repository, service) = fixture();
    let old = service
        .create_decision(
            "session-1",
            "Storage",
            "Use SQLite.",
            DecisionSource::User,
            DecisionPriority::Normal,
            "Keep state local",
            "During the Python implementation",
            "Legacy choice",
            "SQLite at first",
            None,
            None,
            None,
            user_actor(),
        )
        .expect("old");
    let (superseded, replacement) = service
        .supersede_decision(
            &old.id,
            "Storage",
            "Use replaceable repository ports with redb as the initial canonical adapter.",
            DecisionSource::User,
            DecisionPriority::Critical,
            "Keep storage replaceable",
            "For all Rust canonical state",
            "Supports redb, PostgreSQL, and indexes",
            "abstract layer like a Repo pattern",
            user_actor(),
        )
        .expect("supersede");
    assert_eq!(superseded.status, DecisionStatus::Superseded);
    assert_eq!(replacement.supersedes.as_deref(), Some(old.id.as_str()));
    assert_eq!(
        repository
            .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
            .expect("active"),
        vec![replacement.clone()]
    );
    let stream = journal
        .read_stream(&format!("decision:{}", old.id))
        .expect("old stream");
    assert_eq!(stream.last().expect("last").event_type, DECISION_SUPERSEDED);
    assert_eq!(
        journal
            .read_stream(&format!("decision:{}", replacement.id))
            .expect("replacement stream")
            .len(),
        1
    );
}

#[test]
fn plans_reconstruct_and_enforce_single_execution_lifecycle() {
    let (journal, repository, service) = fixture();
    let steps = vec![PlanStep {
        index: 1,
        title: "Implement".into(),
        detail: "Make the scoped Rust change.".into(),
        requires_mutation: true,
    }];
    let draft = service
        .create_plan(
            "session-1",
            "Finish the Rust transition",
            "# Plan",
            steps,
            user_actor(),
        )
        .expect("create");
    assert_eq!(draft.revision, 1);
    let edited = service
        .update_draft_plan(
            &draft.id,
            draft.revision,
            "# Updated plan",
            draft.steps.clone(),
            user_actor(),
        )
        .expect("edit");
    assert_eq!(edited.revision, 2);
    assert_eq!(edited.prompt, draft.prompt);
    let mut stale_repository_edit = draft.clone();
    stale_repository_edit.content = "# Stale repository write".into();
    let error = repository
        .update_plan(stale_repository_edit, user_actor())
        .expect_err("repository CAS");
    assert!(matches!(
        error,
        StoreError::Conflict {
            expected: 1,
            actual: 2,
            ..
        }
    ));
    let approved = service
        .approve_plan(&draft.id, user_actor())
        .expect("approve");
    assert_eq!(approved.status, PlanStatus::Approved);
    assert_eq!(approved.revision, 3);
    assert!(approved.approved_at.is_some());
    assert!(
        service
            .update_draft_plan(
                &draft.id,
                approved.revision,
                "changed",
                approved.steps.clone(),
                user_actor(),
            )
            .is_err()
    );
    let mut forged = approved.clone();
    forged.status = PlanStatus::Executed;
    forged.approved_at = Some("forged".into());
    forged.executed_run_id = Some("run-forged".into());
    assert!(repository.update_plan(forged, user_actor()).is_err());
    let stale_execution = service
        .execute_plan_at_revision(&draft.id, edited.revision, "run-stale", user_actor())
        .expect_err("stale execution");
    assert!(matches!(
        stale_execution,
        StoreError::Conflict {
            expected: 2,
            actual: 3,
            ..
        }
    ));
    assert_eq!(
        repository
            .get_plan(&draft.id)
            .expect("plan after stale execution")
            .expect("record")
            .status,
        PlanStatus::Approved
    );
    let executed = service
        .execute_plan(&draft.id, "run-1", user_actor())
        .expect("execute");
    assert_eq!(executed.status, PlanStatus::Executed);
    assert_eq!(executed.revision, 4);
    assert_eq!(executed.executed_run_id.as_deref(), Some("run-1"));
    assert!(
        service
            .execute_plan(&draft.id, "run-2", user_actor())
            .is_err()
    );
    assert_eq!(
        repository
            .list_plans(Some("session-1"), Some(PlanStatus::Executed), 10)
            .expect("list"),
        vec![executed.clone()]
    );
    let reopened = EventSourcedWorkRepository::new(journal);
    assert_eq!(reopened.get_plan(&draft.id).expect("get"), Some(executed));
    assert_eq!(edited.content, "# Updated plan");
}

#[test]
fn plan_updates_and_discards_reject_stale_revisions() {
    let (_journal, repository, service) = fixture();
    let draft = service
        .create_plan(
            "session-1",
            "Preserve the objective",
            "# First",
            vec![PlanStep {
                index: 1,
                title: "Inspect".into(),
                detail: String::new(),
                requires_mutation: false,
            }],
            user_actor(),
        )
        .expect("create");
    let replacement_steps = vec![PlanStep {
        index: 1,
        title: "Verify".into(),
        detail: "Run focused tests.".into(),
        requires_mutation: false,
    }];
    let updated = service
        .update_draft_plan(
            &draft.id,
            draft.revision,
            "# Refined",
            replacement_steps,
            user_actor(),
        )
        .expect("update");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.prompt, draft.prompt);

    let stale_update = service
        .update_draft_plan(
            &draft.id,
            draft.revision,
            "# Stale",
            draft.steps.clone(),
            user_actor(),
        )
        .expect_err("stale update");
    assert!(matches!(
        stale_update,
        StoreError::Conflict {
            expected: 1,
            actual: 2,
            ..
        }
    ));
    let stale_discard = service
        .discard_plan_at_revision(&draft.id, draft.revision, user_actor())
        .expect_err("stale discard");
    assert!(matches!(
        stale_discard,
        StoreError::Conflict {
            expected: 1,
            actual: 2,
            ..
        }
    ));

    let discarded = service
        .discard_plan_at_revision(&draft.id, updated.revision, user_actor())
        .expect("discard");
    assert_eq!(discarded.status, PlanStatus::Discarded);
    assert_eq!(discarded.revision, 3);
    assert_eq!(
        repository.get_plan(&draft.id).expect("get"),
        Some(discarded)
    );
}

#[test]
fn goals_reconstruct_enforce_budget_and_preserve_terminal_evidence() {
    let (journal, repository, service) = fixture();
    let goal = service
        .create_goal(
            "session-1",
            "Complete the Rust transition",
            2,
            None,
            user_actor(),
        )
        .expect("create");
    let first = service
        .record_goal_iteration(&goal.id, user_actor())
        .expect("iteration");
    assert_eq!(first.iterations_completed, 1);
    let complete = service
        .update_goal_status(
            &goal.id,
            GoalStatus::Complete,
            "Transition verified.",
            "",
            user_actor(),
        )
        .expect("complete");
    let final_goal = service
        .record_goal_iteration(&goal.id, user_actor())
        .expect("terminal iteration");
    assert_eq!(final_goal.status, GoalStatus::Complete);
    assert_eq!(final_goal.iterations_completed, 2);
    assert_eq!(final_goal.summary, "Transition verified.");
    assert!(
        service
            .record_goal_iteration(&goal.id, user_actor())
            .is_err()
    );
    assert!(
        service
            .update_goal_status(&goal.id, GoalStatus::Blocked, "", "late", user_actor(),)
            .is_err()
    );
    assert_eq!(
        repository
            .list_goals(Some("session-1"), Some(GoalStatus::Complete), 10)
            .expect("list"),
        vec![final_goal.clone()]
    );
    let reopened = EventSourcedWorkRepository::new(journal);
    assert_eq!(reopened.get_goal(&goal.id).expect("get"), Some(final_goal));
    assert_eq!(complete.iterations_completed, 1);
}

#[test]
fn started_goal_iterations_remain_consumed_when_a_run_does_not_complete() {
    let (_, repository, service) = fixture();
    let goal = service
        .create_goal(
            "session-1",
            "Use a bounded retry budget",
            2,
            None,
            user_actor(),
        )
        .expect("create");

    service
        .record_goal_iteration(&goal.id, user_actor())
        .expect("reserve failed attempt");
    let resumable = repository
        .get_goal(&goal.id)
        .expect("goal")
        .expect("record");
    assert_eq!(resumable.status, GoalStatus::Active);
    assert_eq!(resumable.iterations_completed, 1);

    let exhausted = service
        .record_goal_iteration(&goal.id, user_actor())
        .expect("reserve remaining attempt");
    assert_eq!(exhausted.iterations_completed, 2);
    assert!(
        service
            .record_goal_iteration(&goal.id, user_actor())
            .is_err(),
        "a failed run must not make the same bounded slot reusable"
    );
}

#[test]
fn concurrent_stale_goal_iteration_reservations_commit_only_one_budget_slot() {
    let (journal, repository, service) = fixture();
    let goal = service
        .create_goal(
            "session-1",
            "Reserve each concurrent iteration once",
            2,
            None,
            user_actor(),
        )
        .expect("create");
    let mut reservation = goal.clone();
    reservation.iterations_completed = 1;
    reservation.updated_at = "2026-01-01T00:00:01Z".into();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let repository = Arc::clone(&repository);
        let reservation = reservation.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            repository.record_goal_iteration(reservation, 0, user_actor())
        }));
    }
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("reservation thread"))
        .collect::<Vec<_>>();
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::Conflict { .. })))
            .count(),
        1,
        "{results:?}"
    );
    let current = repository
        .get_goal(&goal.id)
        .expect("goal")
        .expect("record");
    assert_eq!(current.iterations_completed, 1);
    assert_eq!(
        journal
            .read_stream(&format!("goal:{}", goal.id))
            .expect("goal events")
            .len(),
        2
    );
}

#[test]
fn approved_plan_is_atomically_consumed_by_only_one_goal() {
    let (journal, repository, service) = fixture();
    let plan = service
        .create_plan(
            "session-1",
            "Ship Rust",
            "# Approved",
            vec![PlanStep {
                index: 1,
                title: "Verify".into(),
                detail: String::new(),
                requires_mutation: false,
            }],
            user_actor(),
        )
        .expect("plan");
    let approved = service
        .approve_plan(&plan.id, user_actor())
        .expect("approve");
    let stale_handoff = service
        .create_goal_at_plan_revision(
            "session-1",
            "Execute stale approved plan",
            5,
            Some(plan.id.clone()),
            Some(plan.revision),
            user_actor(),
        )
        .expect_err("stale plan handoff");
    assert!(matches!(
        stale_handoff,
        StoreError::Conflict {
            expected: 1,
            actual: 2,
            ..
        }
    ));
    assert!(
        repository
            .list_goals(Some("session-1"), None, 10)
            .expect("goals after stale handoff")
            .is_empty()
    );
    assert_eq!(
        repository
            .get_plan(&plan.id)
            .expect("plan after stale handoff")
            .expect("record")
            .status,
        PlanStatus::Approved
    );
    let (goal, committed_plan) = service
        .create_goal_with_plan_at_revision(
            "session-1",
            "Execute approved plan",
            5,
            Some(plan.id.clone()),
            Some(approved.revision),
            user_actor(),
        )
        .expect("goal");
    let consumed = repository
        .get_plan(&plan.id)
        .expect("plan")
        .expect("record");
    assert_eq!(committed_plan.as_ref(), Some(&consumed));
    assert_eq!(consumed.status, PlanStatus::Executed);
    assert_eq!(consumed.revision, 3);
    assert_eq!(consumed.executed_run_id.as_deref(), Some(goal.id.as_str()));
    assert_eq!(goal.source_plan_id.as_deref(), Some(plan.id.as_str()));
    assert!(
        service
            .create_goal(
                "session-1",
                "Duplicate",
                5,
                Some(plan.id.clone()),
                user_actor(),
            )
            .is_err()
    );
    let plan_events = journal
        .read_stream(&format!("plan:{}", plan.id))
        .expect("plan events");
    assert_eq!(plan_events.last().expect("event").event_type, PLAN_EXECUTED);
}

#[test]
fn subagents_reconstruct_and_enforce_terminal_requeue_transitions() {
    let (journal, repository, service) = fixture();
    let queued = service
        .create_subagent(
            CreateSubagentRequest {
                session_id: "session-1".into(),
                parent_run_id: "run-1".into(),
                parent_call_id: "call-1".into(),
                task: "Review the tests".into(),
                role: "subagent_default".into(),
                allowed_tools: None,
            },
            user_actor(),
        )
        .expect("queue");
    let bounded = service
        .create_subagent(
            CreateSubagentRequest {
                session_id: "session-1".into(),
                parent_run_id: "run-bounded".into(),
                parent_call_id: "call-bounded".into(),
                task: "Review one file".into(),
                role: "subagent_default".into(),
                allowed_tools: Some(vec!["filesystem.read".into(), "git.diff".into()]),
            },
            user_actor(),
        )
        .expect("queue bounded subagent");
    assert_eq!(
        bounded.allowed_tools,
        Some(vec!["filesystem.read".into(), "git.diff".into()])
    );
    assert!(
        service
            .create_subagent(
                CreateSubagentRequest {
                    session_id: "session-1".into(),
                    parent_run_id: "run-duplicate".into(),
                    parent_call_id: "call-duplicate".into(),
                    task: "Review one file".into(),
                    role: "subagent_default".into(),
                    allowed_tools: Some(vec!["filesystem.read".into(), "filesystem.read".into(),]),
                },
                user_actor(),
            )
            .is_err()
    );
    let running = service
        .start_subagent(&queued.id, user_actor())
        .expect("start");
    assert_eq!(running.status, SubagentStatus::Running);
    let failed = service
        .stop_subagent(
            &queued.id,
            SubagentStatus::Failed,
            "provider failed",
            user_actor(),
        )
        .expect("fail");
    assert_eq!(failed.status, SubagentStatus::Failed);
    let requeued = service
        .requeue_subagent(&queued.id, user_actor())
        .expect("requeue");
    assert_eq!(requeued.status, SubagentStatus::Queued);
    assert!(requeued.error.is_empty());
    service
        .start_subagent(&queued.id, user_actor())
        .expect("restart");
    let completed = service
        .complete_subagent(&queued.id, "child-run", "done", user_actor())
        .expect("complete");
    assert_eq!(completed.status, SubagentStatus::Completed);
    assert!(service.requeue_subagent(&queued.id, user_actor()).is_err());
    assert_eq!(
        repository
            .list_subagents(Some("session-1"), Some(SubagentStatus::Completed), 10)
            .expect("list"),
        vec![completed.clone()]
    );
    let reopened = EventSourcedWorkRepository::new(journal);
    assert_eq!(
        reopened.get_subagent(&queued.id).expect("get"),
        Some(completed)
    );
    let cancellable = service
        .create_subagent(
            CreateSubagentRequest {
                session_id: "session-1".into(),
                parent_run_id: "run-2".into(),
                parent_call_id: "call-2".into(),
                task: "Long child task".into(),
                role: "subagent_default".into(),
                allowed_tools: None,
            },
            user_actor(),
        )
        .expect("queue cancellable");
    service
        .start_subagent(&cancellable.id, user_actor())
        .expect("start cancellable");
    let cancelled = service
        .stop_subagent(
            &cancellable.id,
            SubagentStatus::Cancelled,
            "operator cancelled",
            user_actor(),
        )
        .expect("cancel");
    assert_eq!(cancelled.status, SubagentStatus::Cancelled);
    assert!(
        service
            .complete_subagent(&cancellable.id, "late-run", "late output", user_actor())
            .is_err()
    );
}
