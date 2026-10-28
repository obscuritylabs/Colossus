use super::*;
use colossus_connector::ConnectorCommand;

#[test]
fn url_and_token_flow_uses_ordinary_workspace_selection_without_connection_json() {
    let cli = Cli::try_parse_from([
        "colossus",
        "--workspace",
        "project",
        "--config",
        "runtime.yaml",
        "cloud",
        "enroll",
        "https://cloud.example.test/api/enroll",
        "--name",
        "engineering",
        "--enrollment-ca-file",
        "private-ca.pem",
    ])
    .expect("simple enrollment");
    assert_eq!(cli.workspace, PathBuf::from("project"));
    assert_eq!(cli.config, Some(PathBuf::from("runtime.yaml")));
    let Command::Cloud(command) = cli.command else {
        panic!("cloud command");
    };
    assert!(command.requires_managed_runtime());
    let ConnectorCommand::Enroll(arguments) = command else {
        panic!("enroll");
    };
    assert_eq!(
        arguments.selected_url().unwrap(),
        "https://cloud.example.test/api/enroll"
    );
    assert_eq!(arguments.local.storage.name, "engineering");
    assert!(arguments.local.local_config.is_none());
    assert!(!arguments.local.allow_approvals);
    assert!(!arguments.local.enable_human_terminals);
    assert_eq!(
        arguments.enrollment_ca_file,
        Some(PathBuf::from("private-ca.pem"))
    );
}

#[test]
fn advanced_external_daemon_and_legacy_url_flag_remain_available() {
    let cli = Cli::try_parse_from([
        "colossus",
        "control-plane",
        "enroll",
        "--enrollment-url",
        "https://cloud.example.test/api/enroll",
        "--local-config",
        "connector.json",
    ])
    .expect("advanced external enrollment");
    let Command::Cloud(command) = cli.command else {
        panic!("cloud command");
    };
    assert!(!command.requires_managed_runtime());
    let ConnectorCommand::Enroll(arguments) = command else {
        panic!("enroll");
    };
    assert_eq!(
        arguments.selected_url().unwrap(),
        "https://cloud.example.test/api/enroll"
    );
    assert_eq!(
        arguments.local.local_config,
        Some(PathBuf::from("connector.json"))
    );
}

#[test]
fn enrollment_rejects_missing_or_ambiguous_url_and_invitation_arguments() {
    for arguments in [
        vec!["colossus", "cloud", "enroll"],
        vec![
            "colossus",
            "cloud",
            "enroll",
            "https://one.test/api/enroll",
            "--enrollment-url",
            "https://two.test/api/enroll",
        ],
        vec![
            "colossus",
            "cloud",
            "enroll",
            "https://one.test/api/enroll",
            "--token",
            "synthetic-invitation",
        ],
    ] {
        assert!(Cli::try_parse_from(arguments).is_err());
    }
}

#[test]
fn approval_and_shell_grants_require_explicit_managed_flags() {
    let cli = Cli::try_parse_from([
        "colossus",
        "cloud",
        "run",
        "--allow-approvals",
        "--enable-human-terminals",
    ])
    .expect("explicit managed authority");
    let Command::Cloud(ConnectorCommand::Run(arguments)) = cli.command else {
        panic!("run");
    };
    assert!(arguments.allow_approvals && arguments.enable_human_terminals);
    for flag in ["--allow-approvals", "--enable-human-terminals"] {
        assert!(
            Cli::try_parse_from([
                "colossus",
                "cloud",
                "run",
                "--local-config",
                "connector.json",
                flag,
            ])
            .is_err(),
            "external-daemon authority cannot be changed by a managed flag"
        );
    }
}

#[test]
fn saved_enrollment_lifecycle_commands_do_not_open_a_managed_runtime() {
    for operation in ["status", "renew", "disconnect", "revoke", "forget"] {
        let cli = Cli::try_parse_from(["colossus", "cloud", operation, "--name", "engineering"])
            .expect("saved enrollment command");
        let Command::Cloud(command) = cli.command else {
            panic!("cloud command");
        };
        assert!(!command.requires_managed_runtime());
    }
    let cli = Cli::try_parse_from(["colossus", "cloud", "run", "--name", "engineering"])
        .expect("managed reconnect");
    let Command::Cloud(command) = cli.command else {
        panic!("cloud command");
    };
    assert!(command.requires_managed_runtime());
}
