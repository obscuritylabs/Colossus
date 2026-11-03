use super::*;

fn message(run: &Run) -> ReleasedSessionMessage {
    ReleasedSessionMessage {
        session_id: run.session_id.clone(),
        run_id: run.id.clone(),
        sequence: 7,
        role: ReleasedMessageRole::User,
        content: vec![ReleasedContentPart::Text {
            text: "accepted input".into(),
        }],
        created_at: "2026-11-01T00:00:00Z".into(),
    }
}

#[test]
fn user_message_write_is_execute_only_owner_bound_and_exactly_idempotent_after_later_updates() {
    let (_, repository, _) = fixture();
    let owner = caller_context("app:input-owner", "input-create", &[scopes::RUNS_EXECUTE]);
    let request = create_request("input-create", "accepted input");
    let run = create_run(&repository, &owner, &request, "input-run", "input-session");
    let value = message(&run);
    let written = repository
        .append_user_message_once(&owner, &run.id, run.last_sequence, value.clone())
        .unwrap();
    assert!(!written.replayed);
    assert!(repository.updates_after(&owner, &run.id, 0, 16).is_err());
    let mut changed_sequence = value.clone();
    changed_sequence.sequence += 1;
    let mut changed_time = value.clone();
    changed_time.created_at = "2026-11-01T00:00:01Z".into();
    for changed in [changed_sequence, changed_time] {
        let error = repository
            .append_user_message_once(&owner, &run.id, written.value.sequence, changed)
            .unwrap_err();
        assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    }
    assert_eq!(
        repository
            .recoverable_run(&owner, &run.id)
            .unwrap()
            .unwrap()
            .0
            .last_sequence,
        written.value.sequence,
        "a second initial identity must not append another user release"
    );
    let later = repository
        .append_update(
            &owner,
            &run.id,
            written.value.sequence,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    let terminal = repository
        .append_update(
            &owner,
            &run.id,
            later.sequence,
            RunUpdateKind::Failure {
                status: RunStatus::Failed,
                failure: RunFailure {
                    code: "fixture.stopped".into(),
                    message: "fixture stopped".into(),
                    outcome: OutcomeCertainty::Known,
                    recoverable: false,
                    http_status: None,
                    retry_after_ms: None,
                },
            },
        )
        .unwrap();
    let replay = repository
        .append_user_message_once(&owner, &run.id, run.last_sequence, value.clone())
        .unwrap();
    assert!(replay.replayed);
    assert!(
        repository
            .user_message_published(&owner, &run.id, &request.input)
            .unwrap()
    );
    assert!(
        repository
            .user_message_published(
                &owner,
                &run.id,
                &[ContentPart::Text {
                    text: "different".into()
                }]
            )
            .is_err()
    );
    assert_eq!(replay.value, written.value);
    let mut mismatch = value.clone();
    mismatch.content = vec![ReleasedContentPart::Text {
        text: "PRIVATE_MISMATCH".into(),
    }];
    let error = repository
        .append_user_message_once(&owner, &run.id, terminal.sequence, mismatch)
        .unwrap_err();
    assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    assert!(!error.message.contains("PRIVATE_MISMATCH"));
    let other = caller_context("app:other", "input-other", &[scopes::RUNS_EXECUTE]);
    assert!(
        repository
            .append_user_message_once(&other, &run.id, terminal.sequence, value)
            .is_err()
    );
    let second = create_run(
        &repository,
        &owner,
        &create_request("input-second", "second"),
        "second-run",
        "second-session",
    );
    let mut forged = message(&second);
    forged.session_id = "different-session".into();
    assert!(
        repository
            .append_user_message_once(&owner, &second.id, second.last_sequence, forged)
            .is_err()
    );
    let end = repository
        .append_update(
            &owner,
            &second.id,
            second.last_sequence,
            RunUpdateKind::Failure {
                status: RunStatus::Failed,
                failure: RunFailure {
                    code: "fixture.stopped".into(),
                    message: "fixture stopped".into(),
                    outcome: OutcomeCertainty::Known,
                    recoverable: false,
                    http_status: None,
                    retry_after_ms: None,
                },
            },
        )
        .unwrap();
    let mut second_message = message(&second);
    second_message.content = vec![ReleasedContentPart::Text {
        text: "second".into(),
    }];
    assert!(
        repository
            .append_user_message_once(&owner, &second.id, end.sequence, second_message)
            .is_err()
    );
}
