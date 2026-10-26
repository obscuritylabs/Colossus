use super::*;

#[test]
fn terminal_authority_is_an_explicit_role_and_is_not_inherited_from_administration() {
    for role in [
        ProjectRole::Viewer,
        ProjectRole::Operator,
        ProjectRole::Approver,
        ProjectRole::ProjectAdmin,
    ] {
        assert!(!role.permissions().contains(&CloudPermission::Terminal));
    }
    let terminal = ProjectRole::TerminalOperator.permissions();
    assert!(terminal.contains(&CloudPermission::Terminal));
    assert!(terminal.contains(&CloudPermission::Read));
    assert!(!terminal.contains(&CloudPermission::Approve));
    assert!(!terminal.contains(&CloudPermission::Administer));
    let restored: ProjectRole = serde_json::from_str("\"terminal_operator\"").unwrap();
    assert_eq!(restored, ProjectRole::TerminalOperator);
    assert!(
        CloudCaller::new(
            "operator".into(),
            "project".into(),
            ProjectRole::Operator.permissions()
        )
        .unwrap()
        .require(CloudPermission::Terminal)
        .is_err()
    );
}
