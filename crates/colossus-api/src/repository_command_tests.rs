use super::*;

#[test]
fn released_context_is_command_only_and_immutable() {
    let context = colossus_contracts::CommandApprovalContext {
        justification: "Check dependency versions.".into(),
        executable: "/bin/sh".into(),
        arguments: vec!["-c".into(), "cargo --version".into()],
        working_directory: "/work".into(),
        redacted: false,
    };
    assert!(validate_public_command_context(Some("process.execute"), Some(&context)).is_ok());
    assert!(validate_public_command_context(Some("workspace.modify"), Some(&context)).is_err());
    assert!(validate_public_command_context(None, Some(&context)).is_err());
    let pending = Interaction {
        id: "approval".into(),
        kind: InteractionKind::Approval,
        status: InteractionStatus::Pending,
        application_id: "app:owner".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        prompt: "Approval required".into(),
        choices: vec![],
        allow_free_form: false,
        request_hash: Some("a".repeat(64)),
        action: Some("process.execute".into()),
        resource: Some("configured executable".into()),
        risk: None,
        command_context: Some(context),
        expires_at: "2999-01-01T00:00:00Z".into(),
        response: None,
        responded_at: None,
    };
    let mut changed = pending.clone();
    changed.command_context.as_mut().unwrap().justification = "Changed task.".into();
    assert!(!same_interaction_challenge(&pending, &changed));
    changed = pending.clone();
    changed
        .command_context
        .as_mut()
        .unwrap()
        .arguments
        .push("different".into());
    assert!(!same_interaction_challenge(&pending, &changed));
    let mut historical = serde_json::to_value(&pending).unwrap();
    historical
        .as_object_mut()
        .unwrap()
        .remove("command_context");
    assert!(
        serde_json::from_value::<Interaction>(historical)
            .unwrap()
            .command_context
            .is_none()
    );
}
