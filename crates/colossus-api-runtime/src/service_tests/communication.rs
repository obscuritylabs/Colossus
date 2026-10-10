use super::*;
use colossus_api::{
    AgentMessageReceipt, GetAgentMessageRequest, GetAgentTaskRequest, ListAgentMessagesRequest,
    ListAgentParticipantsRequest, ListAgentTasksRequest, SubmitAgentTaskMessageRequest,
};

fn input(id: &str, text: &str) -> SubmitAgentTaskMessageRequest {
    SubmitAgentTaskMessageRequest {
        message_id: id.into(),
        task_id: None,
        context_id: None,
        text: text.into(),
        role: "primary".into(),
        max_turns: 2,
    }
}
async fn terminal(api: &RuntimeAgentRunApi, caller: &CallerContext, id: &str) {
    let mut updates = api
        .watch_run(
            caller,
            WatchRunRequest {
                run_id: id.into(),
                after_sequence: 0,
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(update) = updates.next().await {
            if matches!(
                update.unwrap().kind,
                RunUpdateKind::Result { .. }
                    | RunUpdateKind::Failure { .. }
                    | RunUpdateKind::Cancellation { .. }
            ) {
                return;
            }
        }
    })
    .await
    .unwrap();
}

pub(super) async fn watch_survives_requeue_after_all_attempts_close(runtime: Arc<Runtime>) {
    use colossus_api::WatchAgentMessagesRequest;
    use colossus_contracts::{
        AgentMessageFailure, ExecutionContext, SendAgentMessage, SubagentJob, SubagentStatus,
    };
    use colossus_ports::{AgentInbox, ChildCommunication, RunControl};

    let api = service(
        runtime.clone(),
        RunAdmissionConfig::default()
            .with_watch_limits(1, 1)
            .unwrap(),
    );
    let comm = api.communication().unwrap();
    let owner = caller_with_exact_scopes(
        &format!("app:watch-requeue-{}", Uuid::now_v7().simple()),
        "watch-requeue",
        &[
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::AGENT_MESSAGES_READ,
        ],
    );
    let create = request("watch-requeue-root", "Inspect a requeued child");
    let new_run = NewRun::from_request(
        Uuid::now_v7().to_string(),
        Uuid::now_v7().to_string(),
        "primary",
        &create,
    )
    .unwrap();
    let context = ExecutionContext {
        run_id: Some(new_run.id().into()),
        session_id: Some(new_run.session_id().into()),
        ..Default::default()
    };
    let communication = runtime.communication();
    let events = communication.stage_root(&context, &owner.actor()).unwrap();
    let repository = Arc::new(EventSourcedRunRepository::new(runtime.journal()));
    let run = repository
        .create_run(&owner, &create, &new_run.with_transaction_events(events))
        .unwrap()
        .value;
    let writer = RunWriter::new(
        repository,
        Arc::new(crate::feed::RunFeeds::default()),
        owner.clone(),
        &run,
    );
    writer
        .append(RunUpdateKind::State {
            status: RunStatus::Running,
        })
        .unwrap();
    communication
        .begin_run(&context, &owner.actor(), RunControl::default())
        .unwrap();
    let mut job = SubagentJob {
        id: Uuid::now_v7().to_string(),
        session_id: context.session_id.clone().unwrap(),
        parent_run_id: context.run_id.clone().unwrap(),
        parent_call_id: "watch-delegate".into(),
        task: "Inspect a requeued child".into(),
        role: "primary".into(),
        allowed_tools: Some(Vec::new()),
        status: SubagentStatus::Queued,
        child_session_id: Uuid::now_v7().to_string(),
        child_run_id: None,
        final_output: String::new(),
        error: String::new(),
        created_at: "2026-10-09T12:00:00Z".into(),
        updated_at: "2026-10-09T12:00:00Z".into(),
        started_at: None,
        completed_at: None,
    };
    runtime
        .journal()
        .append_batch(communication.register_child(&job).unwrap())
        .unwrap();
    communication.child_committed(&job);
    let first = communication
        .list_participants(&owner.actor(), &job.parent_run_id)
        .unwrap()
        .pop()
        .unwrap();
    let message = communication
        .send_from_application(
            &owner.actor(),
            SendAgentMessage {
                recipient_id: first.id.clone(),
                text: "Before completion".into(),
                idempotency_key: "before-requeue".into(),
                reply_to: None,
            },
        )
        .unwrap();
    let watch = WatchAgentMessagesRequest {
        root_run_id: job.parent_run_id.clone(),
        after_sequence: 0,
    };
    let mut updates = comm.watch_messages(&owner, watch.clone()).await.unwrap();
    let accepted = updates.next().await.unwrap().unwrap();
    assert_eq!(accepted.message.id, message.id);
    assert_eq!(accepted.message.receipt, AgentMessageReceipt::Accepted);

    job.status = SubagentStatus::Completed;
    runtime
        .journal()
        .append_batch(
            communication
                .close_child(&job, AgentMessageFailure::Completed)
                .unwrap(),
        )
        .unwrap();
    communication.child_committed(&job);
    communication
        .close_run(&job.parent_run_id, AgentMessageFailure::Completed)
        .unwrap();
    writer
        .append(RunUpdateKind::Result {
            result: RunResult {
                output: "Parent completed".into(),
                plan_id: None,
                plan_revision: None,
                plan_status: None,
                goal_id: None,
                profile: "offline".into(),
                model_profile: "offline".into(),
                provider_profile: "offline".into(),
                model: "offline".into(),
                elapsed_seconds: 0.1,
            },
        })
        .unwrap();
    let closed = updates.next().await.unwrap().unwrap();
    assert_eq!(closed.sequence, accepted.sequence + 1);
    assert!(matches!(
        closed.message.receipt,
        AgentMessageReceipt::NotDelivered {
            reason: AgentMessageFailure::Completed
        }
    ));
    assert!(
        communication
            .list_participants(&owner.actor(), &job.parent_run_id)
            .unwrap()
            .iter()
            .all(|participant| !participant.open)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), updates.next())
            .await
            .is_err(),
        "closed attempts must not end a collaboration watch"
    );

    job.status = SubagentStatus::Queued;
    runtime
        .journal()
        .append_batch(communication.register_child(&job).unwrap())
        .unwrap();
    communication.child_committed(&job);
    let requeued = communication
        .list_participants(&owner.actor(), &job.parent_run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(requeued.generation, 2);
    assert_ne!(requeued.id, first.id);
    let message = communication
        .send_from_application(
            &owner.actor(),
            SendAgentMessage {
                recipient_id: requeued.id.clone(),
                text: "After requeue".into(),
                idempotency_key: "after-requeue".into(),
                reply_to: None,
            },
        )
        .unwrap();
    let accepted = tokio::time::timeout(Duration::from_secs(2), updates.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(accepted.sequence, closed.sequence + 1);
    assert_eq!(accepted.message.id, message.id);
    assert_eq!(accepted.message.recipient_id, requeued.id);
    assert_eq!(
        comm.watch_messages(&owner, watch.clone())
            .await
            .err()
            .unwrap()
            .reason,
        colossus_api::ApiErrorReason::CapacityExceeded
    );
    drop(updates);
    let resumed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match comm
                .watch_messages(
                    &owner,
                    WatchAgentMessagesRequest {
                        after_sequence: closed.sequence,
                        ..watch.clone()
                    },
                )
                .await
            {
                Ok(updates) => break updates,
                Err(error) if error.reason == colossus_api::ApiErrorReason::CapacityExceeded => {
                    tokio::task::yield_now().await
                }
                Err(error) => panic!("watch restart failed: {error:?}"),
            }
        }
    })
    .await
    .unwrap();
    let mut resumed = resumed;
    assert_eq!(
        resumed.next().await.unwrap().unwrap().message.id,
        message.id
    );
}

pub(super) async fn owner_scopes_replay_and_frozen_task_queries(runtime: Arc<Runtime>) {
    let admission = RunAdmissionConfig::default()
        .with_list_limits(4, 1, 64, 64, 64, 64)
        .unwrap();
    let api = service(runtime.clone(), admission);
    let comm = api.communication().unwrap();
    let app = format!("app:peer-{}", Uuid::now_v7().simple());
    let owner = caller_with_exact_scopes(
        &app,
        "communication-owner",
        &[
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
            scopes::AGENT_MESSAGES_READ,
            scopes::AGENT_MESSAGES_SEND,
        ],
    );
    // Recover a queued allocation through the public API after admitting follow-ups.
    // This deterministically exercises the restart path without racing the Echo provider.
    let initial = input("peer-initial", "Review this task");
    let mut create = request("initial-peer-allocation", "Review this task");
    create.max_turns = 2;
    let run_id = Uuid::now_v7().to_string();
    let session_id = Uuid::now_v7().to_string();
    let new_run = NewRun::from_request(&run_id, &session_id, "primary", &create).unwrap();
    let fingerprint = crate::task_inputs::digest(
        &serde_json::to_vec(&(&initial.text, &initial.role, initial.max_turns)).unwrap(),
    );
    let mut events = crate::task_inputs::stage_initial(
        runtime.journal().as_ref(),
        &owner,
        &new_run,
        crate::task_inputs::InitialTaskInput {
            request: initial,
            fingerprint,
        },
    )
    .unwrap();
    let context = colossus_contracts::ExecutionContext {
        run_id: Some(run_id.clone()),
        session_id: Some(session_id),
        ..Default::default()
    };
    events.extend(
        runtime
            .communication()
            .stage_root_with_peer_input(&context, &owner.actor(), Some("peer-initial"))
            .unwrap(),
    );
    EventSourcedRunRepository::new(runtime.journal())
        .create_run(&owner, &create, &new_run.with_transaction_events(events))
        .unwrap();
    let first = comm
        .submit_task_message(&owner, input("peer-initial", "Review this task"))
        .await
        .unwrap();
    let participants = comm
        .list_participants(
            &owner,
            ListAgentParticipantsRequest {
                root_run_id: first.task_id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(participants.len(), 1);
    let root = participants[0].id.clone();
    let mut followup = input("peer-follow-up", "Use updated requirements");
    followup.task_id = Some(first.task_id.clone());
    followup.context_id = Some(first.context_id.clone());
    comm.submit_task_message(&owner, followup.clone())
        .await
        .unwrap();
    let same_followup = comm.submit_task_message(&owner, followup).await.unwrap();
    assert_eq!(same_followup.task_id, first.task_id);
    terminal(&api, &owner, &first.task_id).await;
    let page = comm
        .list_messages(
            &owner,
            ListAgentMessagesRequest {
                participant_id: root.clone(),
                after_sequence: 0,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert!(
        matches!(&page.messages[0].receipt, AgentMessageReceipt::IncludedInTurn { request_hash, .. } if request_hash.len() == 64)
    );
    let messages = runtime.session_messages(&first.context_id).unwrap();
    assert!(
        messages
            .iter()
            .filter(|record| record.message.role == ModelMessageRole::User)
            .all(|record| !record.message.begins_user_turn())
    );
    let task = comm
        .get_task(
            &owner,
            GetAgentTaskRequest {
                task_id: first.task_id.clone(),
                history_length: 16,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        task.history
            .iter()
            .map(|message| message.message_id.as_str())
            .collect::<Vec<_>>(),
        ["peer-initial", "peer-follow-up"]
    );
    assert!(task.status.is_terminal());
    let replay = comm
        .submit_task_message(&owner, input("peer-initial", "Review this task"))
        .await
        .unwrap();
    assert_eq!(replay.task_id, first.task_id);
    assert!(
        comm.submit_task_message(&owner, input("peer-initial", "Changed input"))
            .await
            .is_err()
    );
    let mut late = input("late-message", "New work for a terminal task");
    late.task_id = Some(first.task_id.clone());
    assert!(comm.submit_task_message(&owner, late).await.is_err());

    let other = caller_with_exact_scopes(
        "app:foreign-peer",
        "foreign-peer",
        &[scopes::RUNS_READ, scopes::AGENT_MESSAGES_READ],
    );
    assert!(
        comm.get_task(
            &other,
            GetAgentTaskRequest {
                task_id: first.task_id.clone(),
                history_length: 16
            }
        )
        .await
        .is_err()
    );
    assert!(
        comm.get_message(
            &other,
            GetAgentMessageRequest {
                message_id: page.messages[0].id.clone()
            }
        )
        .await
        .is_err()
    );
    let no_message_read = caller_with_exact_scopes(
        &app,
        "no-message-read",
        &[scopes::RUNS_READ, scopes::AGENT_MESSAGES_SEND],
    );
    assert!(
        comm.list_messages(
            &no_message_read,
            ListAgentMessagesRequest {
                participant_id: root,
                after_sequence: 0,
                limit: 16
            }
        )
        .await
        .is_err()
    );
    let reader = caller_with_exact_scopes(
        &app,
        "message-reader",
        &[scopes::RUNS_READ, scopes::AGENT_MESSAGES_READ],
    );
    assert!(
        comm.submit_task_message(&reader, input("read-cannot-write", "Denied"))
            .await
            .is_err()
    );

    let second = comm
        .submit_task_message(&owner, input("peer-second", "Second task"))
        .await
        .unwrap();
    terminal(&api, &owner, &second.task_id).await;
    let query = ListAgentTasksRequest {
        context_id: None,
        statuses: Vec::new(),
        status_updated_after: None,
        page_size: 1,
        page_token: None,
        include_output: false,
        history_length: 1,
    };
    let first_page = comm.list_tasks(&owner, query.clone()).await.unwrap();
    assert_eq!(first_page.total_size, 2);
    assert_eq!(first_page.tasks.len(), 1);
    assert_eq!(first_page.tasks[0].task_id, second.task_id);
    assert_eq!(first_page.tasks[0].history.len(), 1);
    assert!(first_page.tasks[0].output.is_none());
    let third = comm
        .submit_task_message(&owner, input("peer-third", "Later task"))
        .await
        .unwrap();
    terminal(&api, &owner, &third.task_id).await;
    let mut next = query.clone();
    next.page_token = first_page.next_page_token;
    let second_page = comm.list_tasks(&owner, next.clone()).await.unwrap();
    assert_eq!(second_page.total_size, 2);
    assert_eq!(second_page.tasks[0].task_id, first.task_id);
    assert!(second_page.next_page_token.is_none());
    next.include_output = true;
    assert!(comm.list_tasks(&owner, next).await.is_err());
    let all = comm
        .list_tasks(
            &owner,
            ListAgentTasksRequest {
                page_size: 20,
                ..query
            },
        )
        .await
        .unwrap();
    assert_eq!(all.total_size, 3);
}
