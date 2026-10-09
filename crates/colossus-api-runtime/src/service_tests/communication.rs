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
