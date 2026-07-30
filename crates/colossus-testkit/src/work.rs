use super::*;

/// Shared lifecycle, filtering, immutable-identity, and reconstruction checks for every
/// canonical work repository adapter.
pub fn assert_work_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn WorkRepository>,
{
    const AT: &str = "2026-07-12T00:00:00Z";
    const LATER: &str = "2026-07-12T00:01:00Z";
    let actor = conformance_actor("work-user");
    let repository = factory();

    let task = TaskRecord {
        id: "task-conformance".into(),
        session_id: "session-conformance".into(),
        title: "Prove repository behavior".into(),
        description: "Shared adapter contract".into(),
        status: TaskStatus::Pending,
        created_at: AT.into(),
        updated_at: AT.into(),
    };
    repository
        .create_task(task.clone(), actor.clone())
        .expect("create task");
    assert!(repository.create_task(task.clone(), actor.clone()).is_err());
    let mut updated_task = task.clone();
    updated_task.status = TaskStatus::InProgress;
    updated_task.updated_at = LATER.into();
    repository
        .update_task(updated_task.clone(), actor.clone())
        .expect("update task");

    let decision = KeyDecision {
        id: "decision-conformance".into(),
        session_id: "session-conformance".into(),
        goal_id: None,
        plan_id: None,
        source: DecisionSource::User,
        status: DecisionStatus::Active,
        priority: DecisionPriority::Critical,
        title: "Rust cutover".into(),
        decision: "Use canonical Rust repositories.".into(),
        intent: "Complete the transition.".into(),
        applies_when: "Persisting state.".into(),
        rationale: "One auditable runtime.".into(),
        source_excerpt: "Transition to Rust.".into(),
        supersedes: None,
        created_at: AT.into(),
        updated_at: AT.into(),
    };
    repository
        .create_decision(decision.clone(), actor.clone())
        .expect("create decision");
    let archived = repository
        .archive_decision(&decision.id, actor.clone())
        .expect("archive decision");
    assert_eq!(archived.status, DecisionStatus::Archived);

    let plan = PlanRecord {
        id: "plan-conformance".into(),
        session_id: "session-conformance".into(),
        prompt: "Complete the Rust transition.".into(),
        status: PlanStatus::Draft,
        revision: 1,
        content: "Execute the shared contract.".into(),
        steps: vec![PlanStep {
            index: 1,
            title: "Verify".into(),
            detail: "Run conformance.".into(),
            requires_mutation: false,
        }],
        created_at: AT.into(),
        updated_at: AT.into(),
        approved_at: None,
        executed_run_id: None,
    };
    repository
        .create_plan(plan.clone(), actor.clone())
        .expect("create plan");
    let mut approved = plan.clone();
    approved.status = PlanStatus::Approved;
    approved.updated_at = LATER.into();
    approved.approved_at = Some(LATER.into());
    let approved = repository
        .update_plan(approved.clone(), actor.clone())
        .expect("approve plan");
    assert_eq!(approved.revision, 2);

    let goal = GoalRecord {
        id: "goal-conformance".into(),
        session_id: "session-conformance".into(),
        objective: "Finish the conformance milestone.".into(),
        source_plan_id: None,
        status: GoalStatus::Active,
        summary: String::new(),
        blocked_reason: String::new(),
        iteration_budget: 3,
        iterations_completed: 0,
        created_at: AT.into(),
        updated_at: AT.into(),
    };
    repository
        .create_goal(goal.clone(), actor.clone())
        .expect("create goal");
    let mut iterated_goal = goal.clone();
    iterated_goal.iterations_completed = 1;
    iterated_goal.updated_at = LATER.into();
    repository
        .record_goal_iteration(iterated_goal.clone(), 0, actor.clone())
        .expect("record goal iteration");
    let mut completed_goal = iterated_goal;
    completed_goal.status = GoalStatus::Complete;
    completed_goal.summary = "Conformance verified.".into();
    completed_goal.updated_at = LATER.into();
    repository
        .update_goal(completed_goal.clone(), actor.clone())
        .expect("complete goal");

    let job = SubagentJob {
        id: "agent-conformance".into(),
        session_id: "session-conformance".into(),
        parent_run_id: "run-conformance".into(),
        parent_call_id: "call-conformance".into(),
        task: "Verify one bounded adapter.".into(),
        role: "subagent_default".into(),
        allowed_tools: None,
        status: SubagentStatus::Queued,
        child_session_id: "child-session-conformance".into(),
        child_run_id: None,
        final_output: String::new(),
        error: String::new(),
        created_at: AT.into(),
        updated_at: AT.into(),
        started_at: None,
        completed_at: None,
    };
    repository
        .create_subagent(job.clone(), actor.clone())
        .expect("create subagent");
    let mut running = job.clone();
    running.status = SubagentStatus::Running;
    running.started_at = Some(LATER.into());
    running.updated_at = LATER.into();
    repository
        .update_subagent(running.clone(), actor)
        .expect("start subagent");

    let reopened = factory();
    assert_eq!(
        reopened.get_task(&task.id).expect("task"),
        Some(updated_task)
    );
    assert_eq!(
        reopened.get_decision(&decision.id).expect("decision"),
        Some(archived)
    );
    assert_eq!(reopened.get_plan(&plan.id).expect("plan"), Some(approved));
    assert_eq!(
        reopened.get_goal(&goal.id).expect("goal"),
        Some(completed_goal)
    );
    assert_eq!(
        reopened.get_subagent(&job.id).expect("subagent"),
        Some(running)
    );
    assert_eq!(
        reopened
            .list_tasks(
                Some("session-conformance"),
                Some(TaskStatus::InProgress),
                10
            )
            .expect("tasks")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .list_decisions(
                Some("session-conformance"),
                Some(DecisionStatus::Archived),
                10
            )
            .expect("decisions")
            .len(),
        1
    );
}
