use super::*;
use colossus_contracts::{ActorType, ModelToolCall};
use colossus_testkit::{InMemoryEventJournal, assert_session_repository_conformance};

fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "test".into(),
    }
}

#[test]
fn event_sourced_session_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    assert_session_repository_conformance(|| {
        Box::new(EventSourcedSessionRepository::new(Arc::clone(&journal)))
    });
}

fn message(role: ModelMessageRole, content: &str) -> ModelMessage {
    ModelMessage {
        role,
        content: content.into(),
        tool_call_id: None,
        tool_calls: Vec::new(),
    }
}

#[test]
fn sessions_and_messages_reconstruct_after_repository_restart() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
    repository
        .create_session("session-1", Some("Test session"), actor())
        .expect("create");
    repository
        .append_message(
            "session-1",
            "run-1",
            message(ModelMessageRole::User, "hello"),
            actor(),
        )
        .expect("user message");
    repository
        .append_message(
            "session-1",
            "run-1",
            message(ModelMessageRole::Assistant, "hi"),
            actor(),
        )
        .expect("assistant message");

    let reopened = EventSourcedSessionRepository::new(journal);
    let summary = reopened
        .get_session("session-1")
        .expect("summary")
        .expect("session");
    assert_eq!(summary.message_count, 2);
    assert_eq!(summary.last_user_preview.as_deref(), Some("hello"));
    assert_eq!(summary.last_run_id.as_deref(), Some("run-1"));
    let messages = reopened.list_messages("session-1").expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sequence, 1);
    assert_eq!(messages[1].message.content, "hi");
}

#[test]
fn message_batches_commit_all_or_none_in_order() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
    repository
        .create_session("batched", None, actor())
        .expect("create");
    let assistant = ModelMessage {
        role: ModelMessageRole::Assistant,
        content: String::new(),
        tool_call_id: None,
        tool_calls: vec![ModelToolCall {
            call_id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({}),
        }],
    };
    let tool = ModelMessage {
        role: ModelMessageRole::Tool,
        content: "done".into(),
        tool_call_id: Some("call-1".into()),
        tool_calls: Vec::new(),
    };
    let appended = repository
        .append_messages(
            "batched",
            "run-1",
            vec![
                SessionMessageAppend {
                    message: assistant,
                    actor: actor(),
                },
                SessionMessageAppend {
                    message: tool,
                    actor: actor(),
                },
            ],
        )
        .expect("atomic batch");
    assert_eq!(
        appended
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );

    let invalid = ModelMessage {
        role: ModelMessageRole::User,
        content: "bad".into(),
        tool_call_id: None,
        tool_calls: vec![ModelToolCall {
            call_id: "invalid".into(),
            name: "echo".into(),
            arguments: json!({}),
        }],
    };
    repository
        .append_messages(
            "batched",
            "run-2",
            vec![
                SessionMessageAppend {
                    message: message(ModelMessageRole::User, "would be partial"),
                    actor: actor(),
                },
                SessionMessageAppend {
                    message: invalid,
                    actor: actor(),
                },
            ],
        )
        .expect_err("invalid batch");
    assert_eq!(
        repository.list_messages("batched").expect("messages").len(),
        2,
        "validation must happen before the journal transaction"
    );
}

#[test]
fn list_is_recent_first_bounded_and_missing_session_rejects_messages() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(journal);
    repository
        .create_session("one", None, actor())
        .expect("one");
    repository
        .create_session("two", None, actor())
        .expect("two");
    assert_eq!(repository.list_sessions(1).expect("list")[0].id, "two");
    let error = repository
        .append_message(
            "missing",
            "run-1",
            message(ModelMessageRole::User, "no"),
            actor(),
        )
        .expect_err("missing session");
    assert!(matches!(error, StoreError::NotFound(_)));
}

#[test]
fn message_pages_are_chronological_bounded_and_cursor_safe() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(journal);
    repository
        .create_session("paged", None, actor())
        .expect("session");
    for index in 1..=7 {
        repository
            .append_message(
                "paged",
                "run-1",
                message(ModelMessageRole::User, &format!("message-{index}")),
                actor(),
            )
            .expect("message");
    }

    let newest = repository
        .list_messages_page("paged", None, 3, 2 * 1024 * 1024)
        .expect("newest page");
    assert_eq!(
        newest
            .messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![5, 6, 7]
    );
    assert_eq!(newest.before_sequence, Some(5));
    assert!(newest.has_more);

    let older = repository
        .list_messages_page("paged", newest.before_sequence, 3, 2 * 1024 * 1024)
        .expect("older page");
    assert_eq!(
        older
            .messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert_eq!(older.before_sequence, Some(2));
    assert!(older.has_more);

    assert!(matches!(
        repository.list_messages_page("paged", None, 100, 1),
        Err(StoreError::Adapter(message)) if message.contains("bounded page size")
    ));
}

#[test]
fn invalid_message_shapes_fail_before_append() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
    repository
        .create_session("one", None, actor())
        .expect("create");
    let invalid = ModelMessage {
        role: ModelMessageRole::User,
        content: "bad".into(),
        tool_call_id: None,
        tool_calls: vec![ModelToolCall {
            call_id: "call-1".into(),
            name: "echo".into(),
            arguments: json!({}),
        }],
    };
    assert!(
        repository
            .append_message("one", "run-1", invalid, actor())
            .is_err()
    );
    assert_eq!(journal.read_stream("session:one").expect("stream").len(), 1);
}
