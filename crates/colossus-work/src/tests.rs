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
    let edited = service
        .update_draft_plan(&draft.id, None, Some("# Updated plan"), None, user_actor())
        .expect("edit");
    let approved = service
        .approve_plan(&draft.id, user_actor())
        .expect("approve");
    assert_eq!(approved.status, PlanStatus::Approved);
    assert!(approved.approved_at.is_some());
    assert!(
        service
            .update_draft_plan(&draft.id, Some("changed"), None, None, user_actor())
            .is_err()
    );
    let mut forged = approved.clone();
    forged.status = PlanStatus::Executed;
    forged.approved_at = Some("forged".into());
    forged.executed_run_id = Some("run-forged".into());
    assert!(repository.update_plan(forged, user_actor()).is_err());
    let executed = service
        .execute_plan(&draft.id, "run-1", user_actor())
        .expect("execute");
    assert_eq!(executed.status, PlanStatus::Executed);
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
    service
        .approve_plan(&plan.id, user_actor())
        .expect("approve");
    let goal = service
        .create_goal(
            "session-1",
            "Execute approved plan",
            5,
            Some(plan.id.clone()),
            user_actor(),
        )
        .expect("goal");
    let consumed = repository
        .get_plan(&plan.id)
        .expect("plan")
        .expect("record");
    assert_eq!(consumed.status, PlanStatus::Executed);
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
            "session-1",
            "run-1",
            "call-1",
            "Review the tests",
            "subagent_default",
            user_actor(),
        )
        .expect("queue");
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
            "session-1",
            "run-2",
            "call-2",
            "Long child task",
            "subagent_default",
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
