use super::*;
use colossus_testkit::InMemoryEventJournal;

#[tokio::test]
async fn released_logic_and_recorded_states_preserve_real_routes_without_private_outputs() {
    let yaml = include_str!("../../../../examples/workflows/01-control-flow-lab.yaml");
    let (_, service) = setup();
    let metadata = execute(
        &service,
        &WorkflowControlOperation::RegisterDefinition {
            yaml: yaml.into(),
            expected_hash: validate_definition(yaml).unwrap().content_hash,
            idempotency_key: "logic-import".into(),
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(metadata["logic"]["steps"][0]["kind"], "parallel");
    assert_eq!(
        metadata["logic"]["steps"][1]["branches"][0]["label"],
        "True"
    );
    assert_eq!(metadata["logic"]["steps"][2]["kind"], "foreach");
    assert!(metadata["logic"]["steps"][3].get("value").is_none());
    let catalog = execute(
        &service,
        &WorkflowControlOperation::ListWorkflows {
            after: None,
            limit: 100,
        },
        "app:a",
    )
    .unwrap();
    assert!(
        catalog["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["logic"].is_null())
    );
    let queued = execute(&service, &WorkflowControlOperation::StartRun {
        workflow_id: "control-flow-lab:1.0.0".into(), expected_hash: metadata["workflow_hash"].as_str().unwrap().into(),
        inputs: json!({"environment": "production", "components": ["api", "worker", "desktop"]}), idempotency_key: "logic-run".into(),
    }, "app:a").unwrap();
    let id = queued["run_id"].as_str().unwrap();
    assert_eq!(
        execute(
            &service,
            &WorkflowControlOperation::GetRun { run_id: id.into() },
            "app:a"
        )
        .unwrap()["step_states"],
        json!([])
    );
    service.run_queued(id).await.unwrap();
    let run = execute(
        &service,
        &WorkflowControlOperation::GetRun { run_id: id.into() },
        "app:a",
    )
    .unwrap();
    assert_eq!(run["status"], "completed");
    let states = run["step_states"].as_array().unwrap();
    assert!(
        !states
            .iter()
            .any(|state| state["step_id"] == "non-production-route")
    );
    let component = states
        .iter()
        .find(|state| state["step_id"] == "component-observed")
        .unwrap();
    assert_eq!(component["status"], "completed");
    assert_eq!(component["completed_executions"], 3);
    assert_eq!(
        states
            .iter()
            .find(|state| state["step_id"] == "result")
            .unwrap()["completed_executions"],
        1
    );
    assert!(run.get("outputs").is_none());
    assert!(run.get("inputs").is_none());
    assert!(matches!(
        execute(
            &service,
            &WorkflowControlOperation::GetRun { run_id: id.into() },
            "app:b"
        ),
        Err(WorkflowError::PermissionDenied)
    ));
}

#[test]
fn display_projection_withholds_prompts_arguments_inputs_and_emitted_values() {
    let mut definition = validate_definition(YAML).unwrap().definition;
    definition.steps = vec![
        WorkflowStep::Agent {
            id: "agent".into(),
            prompt: "PRIVATE-PROMPT".into(),
            options: Default::default(),
            idempotency: None,
        },
        WorkflowStep::Tool {
            id: "tool".into(),
            tool: "registered_tool".into(),
            arguments: json!({"secret": "PRIVATE-ARGUMENT"}),
            idempotency: None,
        },
        WorkflowStep::Workflow {
            id: "child".into(),
            workflow: "child".into(),
            version: "1.0.0".into(),
            inputs: json!({"secret": "PRIVATE-INPUT"}),
        },
        WorkflowStep::Approval {
            id: "approve".into(),
            prompt: "PRIVATE-APPROVAL".into(),
        },
        WorkflowStep::WaitForInput {
            id: "input".into(),
            prompt: "PRIVATE-OPERATOR-PROMPT".into(),
            schema: json!({"default": "PRIVATE-SCHEMA"}),
        },
        WorkflowStep::Emit {
            id: "output".into(),
            value: json!({"secret": "PRIVATE-OUTPUT"}),
        },
    ];
    let projection = serde_json::to_string(&view::logic(&definition).unwrap()).unwrap();
    assert!(!projection.contains("PRIVATE-"));
    assert!(projection.contains("registered_tool"));
    assert!(projection.contains("child:1.0.0"));
    definition.steps = (0..513)
        .map(|index| WorkflowStep::Emit {
            id: format!("step-{index}"),
            value: Value::Null,
        })
        .collect();
    assert!(view::logic(&definition).is_none());
}

const YAML: &str = "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata:\n  name: scheduled\n  version: 1.0.0\n  description: Deterministic scheduled work\ninputs:\n  type: object\n  additionalProperties: false\n  required: [message]\n  properties:\n    message: {type: string}\noutputs: {type: object}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 2\nsteps:\n  - id: result\n    type: emit\n    value: {ok: true}\n";

#[test]
fn deletion_is_owned_revision_bound_durable_and_preserves_allocated_runs() {
    let (journal, service) = setup();
    let allocated = execute(&service, &create(), "app:a").unwrap();
    service.tick_schedules_at("2026-10-03T12:00:00Z").unwrap();
    let current = execute(
        &service,
        &WorkflowControlOperation::GetSchedule {
            schedule_id: "daily".into(),
        },
        "app:a",
    )
    .unwrap();
    let delete = WorkflowControlOperation::DeleteSchedule {
        schedule_id: "daily".into(),
        etag: current["etag"].as_str().unwrap().into(),
    };
    let stale = WorkflowControlOperation::DeleteSchedule {
        schedule_id: "daily".into(),
        etag: allocated["etag"].as_str().unwrap().into(),
    };
    assert!(matches!(
        execute(&service, &stale, "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
    assert!(matches!(
        execute(&service, &delete, "app:b"),
        Err(WorkflowError::PermissionDenied)
    ));
    let before = journal
        .read_stream("workflow-schedule:daily")
        .unwrap()
        .len();
    let deleted = execute(&service, &delete, "app:a").unwrap();
    assert_eq!(deleted["schedule_id"], "daily");
    assert_eq!(execute(&service, &delete, "app:a").unwrap(), deleted);
    assert_eq!(
        journal
            .read_stream("workflow-schedule:daily")
            .unwrap()
            .len(),
        before + 1
    );
    let reopened = super::tests::service(journal.clone());
    assert_eq!(execute(&reopened, &delete, "app:a").unwrap(), deleted);
    assert!(matches!(
        reopened.get_schedule("daily"),
        Err(WorkflowError::NotFound(_))
    ));
    assert!(reopened.list_schedules(100).unwrap().is_empty());
    let catalog = execute(
        &reopened,
        &WorkflowControlOperation::ListSchedules {
            after: None,
            limit: 1,
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(catalog["items"], json!([]));
    assert!(
        reopened
            .tick_schedules_at("2026-10-04T12:00:00Z")
            .unwrap()
            .is_empty()
    );
    let run_id = current["record"]["last_run_id"].as_str().unwrap();
    let inspect = WorkflowControlOperation::GetRun {
        run_id: run_id.into(),
    };
    assert_eq!(
        execute(&reopened, &inspect, "app:a").unwrap()["run_id"],
        run_id
    );
    assert!(matches!(
        execute(&reopened, &inspect, "app:b"),
        Err(WorkflowError::PermissionDenied)
    ));
    assert!(matches!(
        execute(&reopened, &create(), "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
    let mut replacement = create();
    if let WorkflowControlOperation::CreateSchedule {
        idempotency_key, ..
    } = &mut replacement
    {
        *idempotency_key = "new-key".into();
    }
    assert!(matches!(
        execute(&reopened, &replacement, "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
    assert_eq!(reopened.repository.runs(100).unwrap().len(), 1);
}

#[test]
fn deleted_streams_do_not_consume_the_live_schedule_page() {
    let (_, service) = setup();
    let first = execute(&service, &create(), "app:a").unwrap();
    execute(
        &service,
        &WorkflowControlOperation::DeleteSchedule {
            schedule_id: "daily".into(),
            etag: first["etag"].as_str().unwrap().into(),
        },
        "app:a",
    )
    .unwrap();
    let mut next = create();
    if let WorkflowControlOperation::CreateSchedule {
        schedule_id,
        idempotency_key,
        ..
    } = &mut next
    {
        *schedule_id = "second".into();
        *idempotency_key = "second-key".into();
    }
    execute(&service, &next, "app:a").unwrap();
    assert_eq!(service.list_schedules(1).unwrap()[0].schedule_id, "second");
    let page = execute(
        &service,
        &WorkflowControlOperation::ListSchedules {
            after: None,
            limit: 1,
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(page["items"][0]["record"]["schedule_id"], "second");
    assert!(page["next_cursor"].is_null());
}

#[test]
fn simultaneous_tick_and_reviewed_delete_have_one_writer_order() {
    for _ in 0..12 {
        let (_, service) = setup();
        let created = execute(&service, &create(), "app:a").unwrap();
        let service = Arc::new(service);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let tick = {
            let service = service.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.tick_schedules_at("2026-10-03T12:00:00Z").unwrap();
            })
        };
        let delete = {
            let service = service.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                execute(
                    &service,
                    &WorkflowControlOperation::DeleteSchedule {
                        schedule_id: "daily".into(),
                        etag: created["etag"].as_str().unwrap().into(),
                    },
                    "app:a",
                )
            })
        };
        barrier.wait();
        tick.join().unwrap();
        match delete.join().unwrap() {
            Ok(_) => {
                assert!(matches!(
                    service.get_schedule("daily"),
                    Err(WorkflowError::NotFound(_))
                ));
                assert!(service.repository.runs(100).unwrap().is_empty());
            }
            Err(error) => {
                assert!(matches!(error, WorkflowError::Conflict(_)));
                assert!(service.get_schedule("daily").unwrap().last_run_id.is_some());
                assert_eq!(service.repository.runs(100).unwrap().len(), 1);
            }
        }
    }
}

fn owner(id: &str) -> WorkflowOrigin {
    WorkflowOrigin {
        owner: Actor {
            actor_type: ActorType::Application,
            id: id.into(),
        },
        session_id: None,
        run_id: None,
    }
}
fn service(journal: Arc<dyn EventJournal>) -> WorkflowService {
    WorkflowService::new(
        Arc::clone(&journal),
        Arc::new(EventSourcedWorkflowRepository::new(journal)),
        Arc::new(DenyWorkflowEffects),
    )
}
fn execute(
    service: &WorkflowService,
    operation: &WorkflowControlOperation,
    id: &str,
) -> Result<Value, WorkflowError> {
    let origin = owner(id);
    service.control(operation, origin.owner.clone(), origin)
}
fn create() -> WorkflowControlOperation {
    WorkflowControlOperation::CreateSchedule {
        schedule_id: "daily".into(),
        workflow_id: "scheduled:1.0.0".into(),
        expected_hash: validate_definition(YAML).unwrap().content_hash,
        inputs: json!({"message": "private input"}),
        cadence_seconds: 60,
        calendar: None,
        task: None,
        starts_at: "2026-10-03T12:00:00Z".into(),
        misfire_policy: WorkflowScheduleMisfirePolicy::FireOnce,
        enabled: true,
        idempotency_key: "create-1".into(),
    }
}
fn setup() -> (Arc<InMemoryEventJournal>, WorkflowService) {
    let journal = Arc::new(InMemoryEventJournal::default());
    let service = service(journal.clone());
    execute(
        &service,
        &WorkflowControlOperation::RegisterDefinition {
            yaml: YAML.into(),
            expected_hash: validate_definition(YAML).unwrap().content_hash,
            idempotency_key: "register-1".into(),
        },
        "app:a",
    )
    .unwrap();
    (journal, service)
}

#[test]
fn legacy_schedule_intent_preserves_its_receipt_fingerprint() {
    let operation = create();
    let value = serde_json::to_value(&operation).unwrap();
    assert!(value.get("calendar").is_none());
    assert!(value.get("task").is_none());
    let restored: WorkflowControlOperation = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), value);
}

#[test]
fn history_pages_by_allocation_order_across_manual_and_scheduled_run_ids() {
    let (_, service) = setup();
    let start = |key: &str| WorkflowControlOperation::StartRun {
        workflow_id: "scheduled:1.0.0".into(),
        expected_hash: validate_definition(YAML).unwrap().content_hash,
        inputs: json!({"message":"hello"}),
        idempotency_key: key.into(),
    };
    let first = execute(&service, &start("first"), "app:a").unwrap();
    execute(&service, &create(), "app:a").unwrap();
    service.tick_schedules_at("2026-10-03T12:00:00Z").unwrap();
    let schedule = execute(
        &service,
        &WorkflowControlOperation::GetSchedule {
            schedule_id: "daily".into(),
        },
        "app:a",
    )
    .unwrap();
    let scheduled = schedule["record"]["last_run_id"].as_str().unwrap();
    execute(&service, &start("other-owner"), "app:b").unwrap();
    let newest = execute(&service, &start("newest"), "app:a").unwrap();
    let list = |after| {
        execute(
            &service,
            &WorkflowControlOperation::ListRuns {
                workflow_id: "scheduled:1.0.0".into(),
                after,
                limit: 1,
            },
            "app:a",
        )
        .unwrap()
    };
    let page1 = list(None);
    assert_eq!(page1["items"][0]["run_id"], newest["run_id"]);
    let page2 = list(Some(page1["next_cursor"].as_str().unwrap().into()));
    assert_eq!(page2["items"][0]["run_id"], scheduled);
    let page3 = list(Some(page2["next_cursor"].as_str().unwrap().into()));
    assert_eq!(page3["items"][0]["run_id"], first["run_id"]);
    assert!(page3["next_cursor"].is_null());
}

#[test]
fn creation_receipt_survives_service_reopen_and_returns_original_after_ticks() {
    let (journal, service) = setup();
    let created = execute(&service, &create(), "app:a").unwrap();
    service.tick_schedules_at("2026-10-03T12:02:00Z").unwrap();
    let reopened = super::tests::service(journal.clone());
    assert_eq!(execute(&reopened, &create(), "app:a").unwrap(), created);
    assert_eq!(
        journal
            .read_stream("workflow-schedule:daily")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(service.repository.runs(100).unwrap().len(), 1);
}

#[test]
fn reviewed_hash_invalid_inputs_and_idempotency_conflict_do_not_mutate() {
    let (journal, service) = setup();
    let before = journal.head().unwrap();
    let mut changed = create();
    if let WorkflowControlOperation::CreateSchedule { expected_hash, .. } = &mut changed {
        *expected_hash = "0".repeat(64);
    }
    assert!(matches!(
        execute(&service, &changed, "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
    let mut invalid = create();
    if let WorkflowControlOperation::CreateSchedule { inputs, .. } = &mut invalid {
        *inputs = json!({"unexpected": true});
    }
    assert!(execute(&service, &invalid, "app:a").is_err());
    assert_eq!(journal.head().unwrap(), before);
    execute(&service, &create(), "app:a").unwrap();
    if let WorkflowControlOperation::CreateSchedule {
        cadence_seconds, ..
    } = &mut changed
    {
        *cadence_seconds = 120;
    }
    assert!(matches!(
        execute(&service, &changed, "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
    assert_eq!(
        journal
            .read_stream("workflow-schedule:daily")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn ownership_lists_and_tick_races_preserve_scope_and_preconditions() {
    let (_, service) = setup();
    let created = execute(&service, &create(), "app:a").unwrap();
    let page = WorkflowControlOperation::ListSchedules {
        after: None,
        limit: 100,
    };
    assert_eq!(
        execute(&service, &page, "app:b").unwrap()["items"],
        json!([])
    );
    assert!(execute(&service, &page, "app:a").unwrap()["items"][0]["record"]["inputs"].is_null());
    assert!(matches!(
        execute(
            &service,
            &WorkflowControlOperation::GetSchedule {
                schedule_id: "daily".into()
            },
            "app:b"
        ),
        Err(WorkflowError::PermissionDenied)
    ));
    service.tick_schedules_at("2026-10-03T12:00:00Z").unwrap();
    let run = service.get_schedule("daily").unwrap().last_run_id.unwrap();
    assert!(matches!(
        execute(
            &service,
            &WorkflowControlOperation::GetRun {
                run_id: run.clone()
            },
            "app:b"
        ),
        Err(WorkflowError::PermissionDenied)
    ));
    let public_run = execute(
        &service,
        &WorkflowControlOperation::GetRun { run_id: run },
        "app:a",
    )
    .unwrap();
    assert_eq!(public_run["status"], "queued");
    assert!(public_run.get("inputs").is_none());
    let pause = WorkflowControlOperation::SetScheduleEnabled {
        schedule_id: "daily".into(),
        enabled: false,
        etag: created["etag"].as_str().unwrap().into(),
    };
    assert!(matches!(
        execute(&service, &pause, "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
    assert!(service.get_schedule("daily").unwrap().enabled);
}

#[test]
fn legacy_schedules_have_metadata_without_claiming_origin_or_inputs() {
    let (_, service) = setup();
    service
        .create_schedule(
            "legacy",
            "scheduled",
            "1.0.0",
            json!({"message": "legacy secret"}),
            60,
            WorkflowScheduleMisfirePolicy::Skip,
            false,
            None,
        )
        .unwrap();
    let snapshot = execute(
        &service,
        &WorkflowControlOperation::GetSchedule {
            schedule_id: "legacy".into(),
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(snapshot["controllable"], false);
    assert!(snapshot["origin"].is_null());
    assert!(snapshot["record"]["inputs"].is_null());
    assert!(matches!(
        execute(
            &service,
            &WorkflowControlOperation::DeleteSchedule {
                schedule_id: "legacy".into(),
                etag: snapshot["etag"].as_str().unwrap().into(),
            },
            "app:a"
        ),
        Err(WorkflowError::PermissionDenied)
    ));
    assert!(matches!(
        execute(
            &service,
            &WorkflowControlOperation::SetScheduleEnabled {
                schedule_id: "legacy".into(),
                enabled: true,
                etag: snapshot["etag"].as_str().unwrap().into()
            },
            "app:a"
        ),
        Err(WorkflowError::PermissionDenied)
    ));
}

#[test]
fn equivalent_utc_start_replays_and_pause_preserves_next_boundary() {
    let (_, service) = setup();
    let first = execute(&service, &create(), "app:a").unwrap();
    let mut same = create();
    if let WorkflowControlOperation::CreateSchedule { starts_at, .. } = &mut same {
        *starts_at = "2026-10-03T08:00:00-04:00".into();
    }
    assert_eq!(execute(&service, &same, "app:a").unwrap(), first);
    let paused = execute(
        &service,
        &WorkflowControlOperation::SetScheduleEnabled {
            schedule_id: "daily".into(),
            enabled: false,
            etag: first["etag"].as_str().unwrap().into(),
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(
        paused["record"]["next_fire_at"],
        first["record"]["next_fire_at"]
    );
    let enabled = execute(
        &service,
        &WorkflowControlOperation::SetScheduleEnabled {
            schedule_id: "daily".into(),
            enabled: true,
            etag: paused["etag"].as_str().unwrap().into(),
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(
        enabled["record"]["next_fire_at"],
        first["record"]["next_fire_at"]
    );
}

#[test]
fn process_abort_after_allocation_commits_receipt_before_replay() {
    const ENV: &str = "COLOSSUS_CONTROL_RECEIPT_CRASH_ROOT";
    if let Ok(root) = std::env::var(ENV) {
        let service = service(crate::tests::control_crash_journal(std::path::Path::new(
            &root,
        )));
        execute(
            &service,
            &WorkflowControlOperation::RegisterDefinition {
                yaml: YAML.into(),
                expected_hash: validate_definition(YAML).unwrap().content_hash,
                idempotency_key: "register-1".into(),
            },
            "app:a",
        )
        .unwrap();
        execute(&service, &create(), "app:a").unwrap();
        panic!("fault injection should terminate after the atomic creation batch");
    }
    let root = tempfile::tempdir().unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "control::tests::process_abort_after_allocation_commits_receipt_before_replay",
            "--nocapture",
        ])
        .env(ENV, root.path())
        .status()
        .unwrap();
    assert!(!child.success());
    let original;
    {
        let journal = crate::tests::process_kill_journal(root.path());
        let recovered = service(journal.clone());
        original = execute(&recovered, &create(), "app:a").unwrap();
        assert_eq!(
            journal
                .read_stream("workflow-schedule:daily")
                .unwrap()
                .len(),
            1
        );
        recovered.tick_schedules_at("2026-10-03T12:02:00Z").unwrap();
    }
    let journal = crate::tests::process_kill_journal(root.path());
    let recovered = service(journal.clone());
    assert_eq!(execute(&recovered, &create(), "app:a").unwrap(), original);
    assert_eq!(recovered.repository.runs(100).unwrap().len(), 1);
    assert_eq!(
        execute(
            &recovered,
            &WorkflowControlOperation::GetSchedule {
                schedule_id: "daily".into()
            },
            "app:a"
        )
        .unwrap()["origin"],
        serde_json::to_value(owner("app:a")).unwrap()
    );
    assert!(journal.verify().unwrap().event_count > 0);
}

#[test]
fn simultaneous_tick_and_reviewed_pause_have_one_writer_order() {
    for _ in 0..12 {
        let (_, service) = setup();
        let created = execute(&service, &create(), "app:a").unwrap();
        let service = Arc::new(service);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let tick = {
            let service = service.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.tick_schedules_at("2026-10-03T12:00:00Z").unwrap();
            })
        };
        let pause = {
            let service = service.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                execute(
                    &service,
                    &WorkflowControlOperation::SetScheduleEnabled {
                        schedule_id: "daily".into(),
                        enabled: false,
                        etag: created["etag"].as_str().unwrap().into(),
                    },
                    "app:a",
                )
            })
        };
        barrier.wait();
        tick.join().unwrap();
        let result = pause.join().unwrap();
        let record = service.get_schedule("daily").unwrap();
        if result.is_ok() {
            assert!(!record.enabled);
            assert!(record.last_run_id.is_none());
        } else {
            assert!(matches!(result, Err(WorkflowError::Conflict(_))));
            assert!(record.enabled);
            assert!(record.last_run_id.is_some());
        }
    }
}

#[test]
fn calendar_ticks_catch_up_once_and_survive_restart_without_duplicate_runs() {
    use colossus_contracts::WorkflowCalendar;
    for policy in [
        WorkflowScheduleMisfirePolicy::FireOnce,
        WorkflowScheduleMisfirePolicy::Skip,
    ] {
        let (journal, service) = setup();
        let mut operation = create();
        if let WorkflowControlOperation::CreateSchedule {
            cadence_seconds,
            calendar,
            starts_at,
            misfire_policy,
            ..
        } = &mut operation
        {
            *cadence_seconds = 0;
            *calendar = Some(WorkflowCalendar {
                timezone: "America/New_York".into(),
                time: "09:00".into(),
                weekdays: Vec::new(),
            });
            *starts_at = "2026-03-07T14:00:00Z".into();
            *misfire_policy = policy;
        }
        execute(&service, &operation, "app:a").unwrap();
        let dispatch = service.tick_schedules_at("2026-03-09T13:00:00Z").unwrap();
        assert_eq!(
            dispatch[0].scheduled_at.as_deref(),
            Some("2026-03-09T13:00:00Z")
        );
        assert_eq!(dispatch[0].next_fire_at, "2026-03-10T13:00:00Z");
        assert_eq!(
            dispatch[0].run_id.is_some(),
            policy == WorkflowScheduleMisfirePolicy::FireOnce
        );
        drop(service);
        assert!(
            super::tests::service(journal)
                .tick_schedules_at("2026-03-09T13:00:00Z")
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn scheduled_task_and_definition_share_a_durable_receipt_and_stay_out_of_the_library() {
    use colossus_contracts::{WorkflowAgentOptions, WorkflowCalendar, WorkflowTask};
    let (journal, service) = setup();
    let mut operation = create();
    if let WorkflowControlOperation::CreateSchedule {
        workflow_id,
        expected_hash,
        inputs,
        cadence_seconds,
        calendar,
        task,
        starts_at,
        ..
    } = &mut operation
    {
        workflow_id.clear();
        expected_hash.clear();
        *inputs = json!({});
        *cadence_seconds = 0;
        *calendar = Some(WorkflowCalendar {
            timezone: "America/New_York".into(),
            time: "09:00".into(),
            weekdays: vec![1],
        });
        *starts_at = "2026-10-05T13:00:00Z".into();
        *task = Some(Box::new(WorkflowTask {
            name: "Monday briefing".into(),
            instructions: "PRIVATE task instructions $() {{ literal }} & * !!".into(),
            tools: vec!["web.search".into()],
            options: WorkflowAgentOptions {
                model_profile: Some("configured".into()),
                reasoning_effort: Some(colossus_contracts::ReasoningEffort::High),
            },
        }));
    }
    let snapshot = execute(&service, &operation, "app:a").unwrap();
    assert_eq!(
        snapshot["record"]["task"]["options"]["reasoning_effort"],
        "high"
    );
    let etag = snapshot["etag"].as_str().unwrap();
    assert_eq!(
        journal.read_stream("workflow-schedule:daily").unwrap()[0].record_hash,
        etag
    );
    let head = journal.head().unwrap();
    drop(service);
    let service = super::tests::service(journal.clone());
    assert_eq!(execute(&service, &operation, "app:a").unwrap(), snapshot);
    assert_eq!(journal.head().unwrap(), head);
    let catalog = execute(
        &service,
        &WorkflowControlOperation::ListWorkflows {
            after: None,
            limit: 100,
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(catalog["items"].as_array().unwrap().len(), 1);
    let list = execute(
        &service,
        &WorkflowControlOperation::ListSchedules {
            after: None,
            limit: 100,
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(list["items"][0]["record"]["task"]["instructions"], "");
    assert!(matches!(
        execute(
            &service,
            &WorkflowControlOperation::GetSchedule {
                schedule_id: "daily".into()
            },
            "app:b"
        ),
        Err(WorkflowError::PermissionDenied)
    ));
    let mut changed = operation;
    if let WorkflowControlOperation::CreateSchedule {
        task: Some(task), ..
    } = &mut changed
    {
        task.instructions.push('!');
    }
    assert!(matches!(
        execute(&service, &changed, "app:a"),
        Err(WorkflowError::Conflict(_))
    ));
}

#[tokio::test]
async fn run_history_returns_only_owned_definition_runs_and_detail_releases_final_result() {
    let (_, service) = setup();
    let request = |key: &str| WorkflowControlOperation::StartRun {
        workflow_id: "scheduled:1.0.0".into(),
        expected_hash: validate_definition(YAML).unwrap().content_hash,
        inputs: json!({"message":"hello"}),
        idempotency_key: key.into(),
    };
    let mine = execute(&service, &request("mine"), "app:a").unwrap();
    execute(&service, &request("other"), "app:b").unwrap();
    let id = mine["run_id"].as_str().unwrap();
    service.run_queued(id).await.unwrap();
    let history = execute(
        &service,
        &WorkflowControlOperation::ListRuns {
            workflow_id: "scheduled:1.0.0".into(),
            after: None,
            limit: 1,
        },
        "app:a",
    )
    .unwrap();
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    assert_eq!(history["items"][0]["run_id"], id);
    assert!(history["items"][0]["result"].is_null());
    assert_eq!(
        execute(
            &service,
            &WorkflowControlOperation::GetRun { run_id: id.into() },
            "app:a"
        )
        .unwrap()["result"],
        json!({"result":{"ok":true}})
    );
}
