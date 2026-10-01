use super::*;
use crate::desktop_settings::{ExecutionBoundarySetting, validate_workspace};

fn fixture() -> (
    tempfile::TempDir,
    DesktopSettings,
    String,
    ApprovalInteraction,
) {
    let root = tempfile::tempdir().unwrap();
    let mut settings = DesktopSettings::default();
    let id = settings
        .add_space(validate_workspace(root.path()).unwrap())
        .unwrap();
    let approval = ApprovalInteraction {
        command_context: Some(colossus_sdk::CommandApprovalContext {
            justification: "Check the build".into(),
            executable: "build-tool".into(),
            arguments: vec!["--check".into(), "two  spaces".into()],
            working_directory: root.path().to_string_lossy().into_owned(),
            redacted: false,
        }),
        action: "process.execute".into(),
        resource: "configured executable".into(),
        reason: "Approval required".into(),
        risk: None,
        request_hash: "one-use-binding".into(),
    };
    (root, settings, id, approval)
}

#[test]
fn remembers_only_exact_argv_cwd_and_workspace_identity() {
    let (_root, settings, id, original) = fixture();
    let rule = allowance(&settings, &id, &original).unwrap();
    for field in [
        "executable",
        "argument",
        "cwd",
        "redacted",
        "action",
        "missing",
        "whitespace",
    ] {
        let mut changed = original.clone();
        let context = changed.command_context.as_mut().unwrap();
        match field {
            "executable" => context.executable.push('x'),
            "argument" => context.arguments.push("--delete".into()),
            "cwd" => context.working_directory.push_str("/elsewhere"),
            "redacted" => context.redacted = true,
            "action" => changed.action = "mcp.call".into(),
            "missing" => changed.command_context = None,
            _ => context.arguments[1] = "two spaces".into(),
        }
        assert_ne!(
            allowance(&settings, &id, &changed).as_ref(),
            Some(&rule),
            "accepted {field}"
        );
    }
    assert!(allowance(&settings, "other-workspace", &original).is_none());
    let mut changed = settings.clone();
    changed.spaces[0]
        .workspace
        .identity
        .as_mut()
        .unwrap()
        .sha256 = "f".repeat(64);
    assert_ne!(allowance(&changed, &id, &original), Some(rule.clone()));
    changed = settings.clone();
    changed.spaces[0].execution_boundary = ExecutionBoundarySetting::OfflineIsolated;
    assert_ne!(allowance(&changed, &id, &original), Some(rule));
}

#[test]
fn one_use_binding_and_agent_explanation_do_not_change_exact_command_scope() {
    let (_root, settings, id, mut approval) = fixture();
    let original = allowance(&settings, &id, &approval).unwrap();
    approval.request_hash = "new-one-use-binding".into();
    approval.command_context.as_mut().unwrap().justification = "Recheck the same build".into();
    assert_eq!(allowance(&settings, &id, &approval), Some(original.clone()));
    let serialized = serde_json::to_string(&original).unwrap();
    assert!(!serialized.contains("build-tool"));
    assert!(!serialized.contains("two  spaces"));
    assert!(!serialized.contains("one-use-binding"));
    assert_eq!(
        serde_json::from_str::<CommandAllowance>(&serialized).unwrap(),
        original
    );
}

#[test]
fn bounded_saved_rules_and_old_settings_default_to_no_consent() {
    let (_root, settings, id, approval) = fixture();
    let rule = allowance(&settings, &id, &approval).unwrap();
    assert!(validate_saved(&vec![rule.clone(); MAX_ALLOWANCES]).is_ok());
    assert!(validate_saved(&vec![rule.clone(); MAX_ALLOWANCES + 1]).is_err());
    let mut malformed = rule;
    malformed.fingerprint = "not-a-digest".into();
    assert!(validate_saved(&[malformed]).is_err());
    let json = serde_json::to_value(settings).unwrap();
    assert!(json.get("commandAllowances").is_none());
}

#[test]
fn saved_consent_survives_reopening_and_can_be_cleared() {
    let (_workspace, mut settings, id, approval) = fixture();
    #[cfg(windows)]
    let parent = tempfile::Builder::new()
        .prefix("ColossusApprovalTest-")
        .tempdir_in(std::env::var_os("LOCALAPPDATA").unwrap())
        .unwrap();
    #[cfg(not(windows))]
    let parent = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(parent.path())
        .unwrap()
        .join("desktop");
    let store = SettingsStore::open(path.clone()).unwrap();
    let _ = crate::desktop_settings::normalized_settings_snapshot(&settings)
        .expect("normalize initial settings");
    store.save(&settings).expect("save initial settings");
    // Capture the normalized persisted workspace configuration, as production does.
    settings = store.load().unwrap();
    let rule = allowance(&settings, &id, &approval).unwrap();
    assert!(store.command_allowances().unwrap().is_empty());
    store
        .save_command_allowances(std::slice::from_ref(&rule))
        .unwrap();
    store.save(&settings).unwrap(); // An ordinary snapshot cannot drop newly saved consent.
    drop(store);
    let reopened = SettingsStore::open(path).unwrap();
    let stale_settings = reopened.load().unwrap();
    assert_eq!(
        reopened.command_allowances().unwrap().as_slice(),
        std::slice::from_ref(&rule)
    );
    assert_eq!(allowance(&stale_settings, &id, &approval), Some(rule));
    reopened.save_command_allowances(&[]).unwrap();
    // A concurrently captured settings snapshot must never resurrect cleared consent.
    reopened.save(&stale_settings).unwrap();
    assert!(reopened.command_allowances().unwrap().is_empty());
}

#[test]
fn settings_changed_during_native_review_cannot_receive_old_consent() {
    let (_root, settings, id, approval) = fixture();
    let reviewed = allowance(&settings, &id, &approval).unwrap();
    assert_eq!(
        revalidate_reviewed_allowance(&settings, &id, &approval, Some(&reviewed)).unwrap(),
        reviewed
    );
    for change in ["global", "boundary", "selection"] {
        let mut changed = settings.clone();
        match change {
            "global" => changed.global_configuration.revision += 1,
            "boundary" => {
                changed.spaces[0].execution_boundary = ExecutionBoundarySetting::OfflineIsolated;
            }
            _ => changed.selected_space_id = None,
        }
        assert!(
            revalidate_reviewed_allowance(&changed, &id, &approval, Some(&reviewed)).is_err(),
            "accepted changed {change}"
        );
    }
    assert!(revalidate_reviewed_allowance(&settings, &id, &approval, None).is_err());
}
