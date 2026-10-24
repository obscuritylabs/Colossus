use super::*;
use colossus_ports::WorkRepository;
use colossus_work::{EventSourcedWorkRepository, WorkService};

struct CanonicalPlanTools {
    work: WorkService,
    writes: AtomicUsize,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ToolExecutor for CanonicalPlanTools {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        self.calls.lock().unwrap().push(call.name.clone());
        let steps = vec![colossus_contracts::PlanStep {
            index: 1,
            title: "Verify".into(),
            detail: "Run the focused gate.".into(),
            requires_mutation: false,
        }];
        let output = match call.name.as_str() {
            "plan.create" => {
                self.writes.fetch_add(1, Ordering::AcqRel);
                let plan = self
                    .work
                    .create_plan(
                        context.session_id.as_deref().unwrap(),
                        "Implement Plan Mode",
                        "Initial plan",
                        steps,
                        test_actor(),
                    )
                    .unwrap();
                serde_json::to_string(&plan).unwrap()
            }
            "plan.update" => {
                self.writes.fetch_add(1, Ordering::AcqRel);
                let plan = self
                    .work
                    .update_draft_plan(
                        context.draft_plan_id.as_deref().unwrap(),
                        context.draft_plan_revision.unwrap(),
                        "Revised plan",
                        steps,
                        test_actor(),
                    )
                    .unwrap();
                serde_json::to_string(&plan).unwrap()
            }
            "echo" => "Read completed".into(),
            _ => panic!("unexpected tool"),
        };
        Ok(ToolResult {
            images: vec![],
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code: 0,
        })
    }
}

struct Fixture {
    service: AgentService,
    provider: Arc<ScriptedProvider>,
    sessions: Arc<dyn SessionRepository>,
    work: Arc<dyn WorkRepository>,
    executor: Arc<CanonicalPlanTools>,
}

fn fixture(turns: Vec<Result<ProviderTurn, ModelProviderError>>) -> Fixture {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(journal.clone()));
    sessions
        .create_session("session-recovery", Some("Plan recovery"), test_actor())
        .unwrap();
    let work: Arc<dyn WorkRepository> = Arc::new(EventSourcedWorkRepository::new(journal.clone()));
    let executor = Arc::new(CanonicalPlanTools {
        work: WorkService::new(work.clone(), sessions.clone()),
        writes: AtomicUsize::new(0),
        calls: Mutex::new(vec![]),
    });
    let provider = Arc::new(ScriptedProvider::new(turns));
    let service = AgentService::new(
        journal,
        provider.clone(),
        Arc::new(
            StaticToolRegistry::builtins(&[
                "plan.create".into(),
                "plan.update".into(),
                "echo".into(),
            ])
            .unwrap(),
        ),
        executor.clone(),
        sessions.clone(),
    )
    .with_work_repository(work.clone());
    Fixture {
        service,
        provider,
        sessions,
        work,
        executor,
    }
}

fn provider_failure() -> Result<ProviderTurn, ModelProviderError> {
    Err(ModelProviderError::HttpStatus {
        status: 404,
        message: "Unavailable model".into(),
    })
}

fn final_turn() -> Result<ProviderTurn, ModelProviderError> {
    turn(vec![ProviderEvent::FinalOutput {
        text: "Plan is ready".into(),
    }])
}

async fn run(fixture: &Fixture, target: PlanDraftTarget) -> Result<AgentRunResult, AgentError> {
    fixture
        .service
        .run_plan_target_in_session_with_skills(
            "primary",
            "Plan only",
            "Continue",
            6,
            Some("session-recovery"),
            &[],
            target,
        )
        .await
}

fn only_plan(fixture: &Fixture) -> PlanRecord {
    let plans = fixture
        .work
        .list_plans(Some("session-recovery"), None, 10)
        .unwrap();
    assert_eq!(plans.len(), 1);
    plans[0].clone()
}

fn update(plan: &PlanRecord) -> PlanDraftTarget {
    PlanDraftTarget::Update {
        plan_id: plan.id.clone(),
        revision: plan.revision,
    }
}

#[tokio::test]
async fn interrupted_plan_recovers_one_committed_write_across_repeated_failures_then_allows_a_new_revision()
 {
    let fixture = fixture(vec![
        turn(vec![plan_create_call("create-once")]),
        provider_failure(),
        turn(vec![plan_create_call("duplicate-create")]),
        turn(vec![ProviderEvent::ToolCallRequested {
            call_id: "recovery-read".into(),
            name: "echo".into(),
            arguments: json!({"text":"read"}),
        }]),
        provider_failure(),
        final_turn(),
        turn(vec![plan_update_call("next-revision")]),
        final_turn(),
    ]);
    assert!(run(&fixture, PlanDraftTarget::Create).await.is_err());
    let original = only_plan(&fixture);
    assert_eq!(original.revision, 1);
    assert!(run(&fixture, update(&original)).await.is_err());
    assert_eq!(fixture.executor.writes.load(Ordering::Acquire), 1);
    let recovered = run(&fixture, update(&original)).await.unwrap();
    assert_eq!(recovered.plan.as_ref(), Some(&original));
    assert_eq!(only_plan(&fixture), original);
    assert_eq!(fixture.executor.writes.load(Ordering::Acquire), 1);
    assert_eq!(
        *fixture.executor.calls.lock().unwrap(),
        ["plan.create", "echo"]
    );
    let revised = run(&fixture, update(&original))
        .await
        .unwrap()
        .plan
        .unwrap();
    assert_eq!(revised.id, original.id);
    assert_eq!(revised.revision, 2);
    assert_eq!(fixture.executor.writes.load(Ordering::Acquire), 2);
    let requests = fixture.provider.requests.lock().unwrap();
    assert_eq!(requests[0].tools, requests[2].tools);
    assert_eq!(requests[0].tools, requests[5].tools);
    assert!(requests[3].messages.iter().any(|message| {
        message
            .content
            .contains("already completed its one required plan.create write")
    }));
    assert!(
        requests[6]
            .tools
            .iter()
            .any(|tool| tool.name == "plan.update")
    );
    assert!(
        !requests[6]
            .tools
            .iter()
            .any(|tool| tool.name == "plan.create")
    );
}

#[tokio::test]
async fn generic_plan_recovery_supersedes_new_update_instructions_after_the_original_write() {
    let fixture = fixture(vec![
        turn(vec![plan_create_call("create-once")]),
        provider_failure(),
        final_turn(),
    ]);
    assert!(run(&fixture, PlanDraftTarget::Create).await.is_err());
    let original = only_plan(&fixture);
    let instructions = "Refine the selected draft. You MUST call plan.update exactly once before your final response.";
    let recovered = fixture
        .service
        .run_plan_target_in_session_with_skills(
            "primary",
            instructions,
            "Continue",
            6,
            Some("session-recovery"),
            &[],
            update(&original),
        )
        .await
        .unwrap();

    assert_eq!(recovered.plan.as_ref(), Some(&original));
    assert_eq!(fixture.executor.writes.load(Ordering::Acquire), 1);
    assert_eq!(only_plan(&fixture), original);
    let requests = fixture.provider.requests.lock().unwrap();
    let recovered_request = &requests[2];
    assert!(recovered_request.instructions.starts_with(instructions));
    assert!(recovered_request.instructions.contains(&format!(
        "Canonical Draft plan {} at revision {} already satisfies that operation's one required write",
        original.id, original.revision,
    )));
    assert!(recovered_request.instructions.contains(
        "supersedes any earlier mandatory plan.create or plan.update requirement for this run"
    ));
    assert!(
        recovered_request
            .instructions
            .contains("Do not create or update a plan again in this run")
    );
    assert!(
        !recovered_request
            .tools
            .iter()
            .any(|tool| tool.name == "plan.update")
    );
}

#[tokio::test]
async fn interrupted_plan_rejects_changed_draft_target_or_mode_before_appending_input() {
    for incompatible in [
        "updated",
        "approved",
        "other_plan",
        "other_revision",
        "execute",
        "pending",
    ] {
        let fixture = fixture(vec![
            turn(vec![plan_create_call("create-once")]),
            provider_failure(),
        ]);
        assert!(run(&fixture, PlanDraftTarget::Create).await.is_err());
        let plan = only_plan(&fixture);
        let target = match incompatible {
            "other_plan" => PlanDraftTarget::Update {
                plan_id: "another-plan".into(),
                revision: 1,
            },
            "other_revision" => PlanDraftTarget::Update {
                plan_id: plan.id.clone(),
                revision: 2,
            },
            _ => update(&plan),
        };
        if matches!(incompatible, "updated" | "approved") {
            let mut changed = plan.clone();
            if incompatible == "approved" {
                changed.status = colossus_contracts::PlanStatus::Approved;
                changed.approved_at = Some("2026-10-23T00:00:00Z".into());
            } else {
                changed.content = "Changed elsewhere".into();
            }
            fixture.work.update_plan(changed, test_actor()).unwrap();
        }
        if incompatible == "pending" {
            fixture
                .sessions
                .begin_tool_turn(
                    "session-recovery",
                    PendingSessionToolTurn {
                        run_id: "uncertain-run".into(),
                        turn: 1,
                        call_ids: vec!["pending-call".into()],
                    },
                    test_actor(),
                )
                .unwrap();
        }
        let before = fixture.sessions.list_messages("session-recovery").unwrap();
        let result = if incompatible == "execute" {
            fixture
                .service
                .run_in_session(
                    "primary",
                    "Execute",
                    "Continue",
                    1,
                    Some("session-recovery"),
                )
                .await
        } else {
            run(&fixture, target).await
        };
        assert!(result.is_err(), "{incompatible}");
        assert_eq!(
            fixture.sessions.list_messages("session-recovery").unwrap(),
            before,
            "{incompatible}"
        );
        assert_eq!(
            fixture.provider.requests.lock().unwrap().len(),
            2,
            "{incompatible}"
        );
        assert_eq!(
            fixture.executor.writes.load(Ordering::Acquire),
            1,
            "{incompatible}"
        );
    }
}
