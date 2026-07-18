use super::*;

/// Shared creation, append-only ordering, validation, and reconstruction checks for
/// every canonical session repository adapter.
pub fn assert_session_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn SessionRepository>,
{
    let repository = factory();
    assert!(
        repository
            .get_session("session-conformance")
            .expect("missing session")
            .is_none()
    );
    let actor = conformance_actor("session-user");
    let created = repository
        .create_session(
            "session-conformance",
            Some("Conformance session"),
            actor.clone(),
        )
        .expect("create session");
    assert_eq!(created.message_count, 0);
    assert!(
        repository
            .create_session("session-conformance", Some("duplicate"), actor.clone())
            .is_err()
    );
    for (role, content) in [
        (ModelMessageRole::User, "Persist this Rust context."),
        (ModelMessageRole::Assistant, "Context persisted."),
    ] {
        repository
            .append_message(
                "session-conformance",
                "run-conformance",
                ModelMessage {
                    role,
                    content: content.into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                actor.clone(),
            )
            .expect("append message");
    }
    let reopened = factory();
    let messages = reopened
        .list_messages("session-conformance")
        .expect("reconstructed messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sequence, 1);
    assert_eq!(messages[1].sequence, 2);
    let summary = reopened
        .get_session("session-conformance")
        .expect("session")
        .expect("reconstructed session");
    assert_eq!(summary.message_count, 2);
    assert_eq!(summary.last_run_id.as_deref(), Some("run-conformance"));
    assert_eq!(
        summary.last_user_preview.as_deref(),
        Some("Persist this Rust context.")
    );
    assert_eq!(
        reopened.list_sessions(10).expect("session list")[0].id,
        "session-conformance"
    );
    assert!(
        reopened
            .append_message(
                "missing-session",
                "run-conformance",
                ModelMessage {
                    role: ModelMessageRole::User,
                    content: "invalid".into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                actor,
            )
            .is_err()
    );
}
