use super::*;
use crate::test_support::private_tempdir;

#[test]
fn branch_receipts_bind_exact_prefix_and_reject_missing_mismatched_or_duplicate_proof() {
    let workspace = private_tempdir();
    let mut config = RuntimeConfig::offline_template("unused.redb");
    config.use_ephemeral_storage();
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path()).unwrap(),
    )
    .unwrap();
    let actor = Actor {
        actor_type: ActorType::Application,
        id: "receipt-owner".into(),
    };
    runtime
        .create_application_session("source", None, actor.clone())
        .unwrap();
    runtime
        .append_application_message(
            "source",
            "source-run",
            ModelMessage {
                agent_message_origin: None,
                role: ModelMessageRole::User,
                content: "same accepted prompt".into(),
                tool_call_id: None,
                tool_calls: vec![],
            },
            actor.clone(),
        )
        .unwrap();
    assert_eq!(
        runtime
            .session_branch_input_boundary("source", "source-run")
            .unwrap(),
        None
    );
    runtime
        .ensure_session_branch(
            "source",
            1,
            RunBranchContextMode::Exact,
            "branch",
            "branch-run",
            actor.clone(),
        )
        .unwrap();
    assert_eq!(
        runtime
            .session_branch_input_boundary("branch", "branch-run")
            .unwrap(),
        Some(1)
    );
    assert!(
        runtime
            .session_branch_input_boundary("branch", "other-run")
            .is_err()
    );
    assert!(
        runtime
            .ensure_session_branch(
                "source",
                1,
                RunBranchContextMode::Conversation,
                "branch",
                "branch-run",
                actor.clone()
            )
            .is_err()
    );

    let event = runtime
        .journal
        .read_stream_from(&stream("branch"), 0, 2)
        .unwrap()
        .remove(0);
    let mut receipt: PreparedBranch =
        serde_json::from_value(runtime.journal.decrypt_payload(&event).unwrap()).unwrap();
    let duplicate = |receipt: &PreparedBranch, expected_stream_version| NewEvent {
        event_version: 1,
        stream_id: stream(&receipt.session_id),
        expected_stream_version,
        classification: EventClassification::Domain,
        event_type: PREPARED_EVENT.into(),
        actor: actor.clone(),
        context: ExecutionContext {
            correlation_id: receipt.run_id.clone(),
            session_id: Some(receipt.session_id.clone()),
            run_id: Some(receipt.run_id.clone()),
            ..Default::default()
        },
        payload: serde_json::to_value(receipt).unwrap(),
    };
    runtime.journal.append(duplicate(&receipt, 1)).unwrap();
    assert!(
        runtime
            .session_branch_input_boundary("branch", "branch-run")
            .is_err()
    );

    runtime
        .create_application_session("mismatch", None, actor.clone())
        .unwrap();
    let message = runtime
        .session_messages("source")
        .unwrap()
        .remove(0)
        .message;
    runtime
        .append_application_message("mismatch", "mismatch-run", message, actor.clone())
        .unwrap();
    receipt.session_id = "mismatch".into();
    receipt.run_id = "mismatch-run".into();
    receipt.prefix_sha256 = "0".repeat(64);
    runtime.journal.append(duplicate(&receipt, 0)).unwrap();
    assert!(
        runtime
            .session_branch_input_boundary("mismatch", "mismatch-run")
            .is_err()
    );

    runtime
        .ensure_session_branch(
            "source",
            0,
            RunBranchContextMode::Exact,
            "empty",
            "empty-run",
            actor,
        )
        .unwrap();
    assert_eq!(
        runtime
            .session_branch_input_boundary("empty", "empty-run")
            .unwrap(),
        Some(0)
    );
    assert!(runtime.session_messages("empty").unwrap().is_empty());
}
