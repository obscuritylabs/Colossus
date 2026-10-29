use super::*;

#[test]
fn archive_restore_survives_restart_without_changing_conversation() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
    repository
        .create_session("history", Some("Keep this"), actor())
        .unwrap();
    repository
        .append_message(
            "history",
            "run-one",
            message(ModelMessageRole::User, "preserved"),
            actor(),
        )
        .unwrap();
    let original = repository.get_session("history").unwrap().unwrap();
    let messages = repository.list_messages("history").unwrap();
    assert_eq!(
        repository
            .session_lifecycle("history")
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    let archived = repository
        .set_session_archived("history", true, 0, actor())
        .unwrap();
    assert_eq!(archived.revision, 1);
    assert_eq!(
        repository
            .set_session_archived("history", true, 1, actor())
            .unwrap(),
        archived
    );

    let reopened = EventSourcedSessionRepository::new(Arc::clone(&journal));
    assert_eq!(
        reopened.session_lifecycle("history").unwrap(),
        Some(archived)
    );
    assert!(reopened.list_sessions(100).unwrap().is_empty());
    assert_eq!(
        reopened.list_archived_sessions(100).unwrap(),
        vec![original.clone()]
    );
    assert_eq!(reopened.list_messages("history").unwrap(), messages);
    let restored = reopened
        .set_session_archived("history", false, 1, actor())
        .unwrap();
    assert!(!restored.archived);
    assert_eq!(restored.revision, 2);
    assert!(reopened.list_archived_sessions(100).unwrap().is_empty());
    assert_eq!(reopened.list_sessions(100).unwrap(), vec![original]);
    assert_eq!(reopened.list_messages("history").unwrap(), messages);
    assert_eq!(
        journal
            .read_stream("session-lifecycle:history")
            .unwrap()
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        [ARCHIVED_EVENT, RESTORED_EVENT]
    );
}

#[test]
fn archive_filter_precedes_recent_session_limit_and_cas_rejects_stale_state() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    let repository = EventSourcedSessionRepository::new(journal);
    repository
        .create_session("a-active", None, actor())
        .unwrap();
    for id in ["y-archived", "z-archived"] {
        repository.create_session(id, None, actor()).unwrap();
        repository
            .set_session_archived(id, true, 0, actor())
            .unwrap();
    }
    assert_eq!(repository.list_sessions(1).unwrap()[0].id, "a-active");
    assert_eq!(repository.list_archived_sessions(1).unwrap().len(), 1);
    assert!(matches!(
        repository.set_session_archived("z-archived", false, 0, actor()),
        Err(StoreError::Conflict {
            expected: 0,
            actual: 1,
            ..
        })
    ));
    assert!(repository.session_lifecycle("missing").unwrap().is_none());
    assert!(matches!(
        repository.set_session_archived("missing", true, 0, actor()),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn unsettled_tool_turn_and_malformed_lifecycle_fail_closed() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedSessionRepository::new(Arc::clone(&journal));
    repository.create_session("pending", None, actor()).unwrap();
    repository
        .begin_tool_turn(
            "pending",
            PendingSessionToolTurn {
                run_id: "unfinished".into(),
                turn: 1,
                call_ids: vec!["call".into()],
            },
            actor(),
        )
        .unwrap();
    assert!(
        repository
            .set_session_archived("pending", true, 0, actor())
            .is_err()
    );
    assert_eq!(
        repository
            .session_lifecycle("pending")
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "session-lifecycle:pending".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: ARCHIVED_EVENT.into(),
            actor: actor(),
            context: ExecutionContext {
                session_id: Some("pending".into()),
                ..Default::default()
            },
            payload: json!({"session_id": "another-session", "archived": true, "revision": 1}),
        })
        .unwrap();
    assert!(matches!(
        repository.session_lifecycle("pending"),
        Err(StoreError::Verification(_))
    ));
    assert!(repository.list_sessions(100).is_err());
}
