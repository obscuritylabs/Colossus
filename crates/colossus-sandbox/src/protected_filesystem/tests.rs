use super::*;
use colossus_contracts::DecisionOutcome;
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, SandboxBoundaryGate, effect_request,
    system_actor,
};
use colossus_testkit::InMemoryEventJournal;

const SYNTHETIC_SECRET: &str = "synthetic-private-authority-needle";

fn fixture() -> (tempfile::TempDir, ConfinedRoot, ProtectedFilesystem) {
    #[cfg(windows)]
    let directory = tempfile::Builder::new()
        .prefix("colossus-protected-")
        .tempdir_in(std::env::var_os("USERPROFILE").expect("test user profile"))
        .expect("fixture below owner profile");
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().expect("fixture");
    let canonical = fs::canonicalize(directory.path()).expect("canonical fixture");
    let root =
        ConfinedRoot::bind(canonical.join("development-credentials")).expect("private authority");
    root.open_file(Path::new(".authority-key.env"))
        .expect("synthetic key file")
        .file()
        .write_all(SYNTHETIC_SECRET.as_bytes())
        .expect("synthetic contents");
    let protection = ProtectedFilesystem::new(vec![root.clone()]).expect("protected authority");
    (directory, root, protection)
}

#[tokio::test]
async fn acknowledged_full_access_permit_cannot_launch_with_development_custody() {
    let (directory, _root, protection) = fixture();
    let parent = fs::canonicalize(directory.path()).expect("cwd");
    let executable = std::env::current_exe().expect("test executable");
    let policy = BuiltInPolicy::offline_default()
        .with_action("process.spawn", DecisionOutcome::Allow)
        .with_sandbox("danger_full_access", "test", false)
        .with_resource_authority(ResourceAuthority::Ambient);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["process.spawn".into()]).with_sandbox_boundary_gate(Arc::new(
            SandboxBoundaryGate::new(Some(SandboxBoundaryMode::DangerFullAccess), true),
        )),
        [78_u8; 32],
    );
    let executor = SandboxProcessExecutor::new(
        SandboxExecutorConfig {
            helper_executable: parent.join("must-not-launch"),
            oci_runtime: None,
            oci_image: None,
            oci_proxy_image: None,
        },
        [77_u8; 32],
    )
    .with_protected_filesystem(protection);
    let mut request = effect_request(
        system_actor("full-access-custody"),
        "process.spawn",
        executable.display().to_string(),
        json!({"cwd": parent, "args": [], "environment": {}}),
    );
    request.capabilities = vec!["process.spawn".into()];
    let error = gateway
        .execute(request, &executor)
        .await
        .expect_err("native-owned custody restriction overrides acknowledged Full permit");
    assert!(
        error.to_string().contains("requires an isolating sandbox"),
        "{error}"
    );
    assert!(
        !error.to_string().contains("must-not-launch"),
        "rejection precedes helper acquisition"
    );
}

fn gateway(root: &Path, action: &str) -> EffectGateway {
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(action, DecisionOutcome::Allow)
                .with_filesystem_root(root.display().to_string(), "write"),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new([action.into()]),
        [73_u8; 32],
    )
}

#[tokio::test]
async fn private_authority_overrides_real_read_metadata_list_write_and_patch_permits() {
    let (directory, root, protection) = fixture();
    let parent = fs::canonicalize(directory.path()).expect("parent");
    let executor = FilesystemExecutor::new().with_protected_filesystem(protection);
    let key = root.path().join(".authority-key.env");
    for action in [
        "filesystem.read",
        "filesystem.metadata",
        "filesystem.list",
        "filesystem.write",
        "patch.preview",
        "audit.export.write",
    ] {
        let target = if action == "filesystem.list" {
            root.path()
        } else {
            &key
        };
        let mut request = effect_request(
            system_actor("protection-test"),
            action,
            target.display().to_string(),
            json!({"text": "replacement", "old": "synthetic", "new": "replacement"}),
        );
        request.capabilities = vec![action.into()];
        let error = gateway(&parent, action)
            .execute(request, &executor)
            .await
            .expect_err("native denial wins over permit");
        assert!(
            error.to_string().contains("native credential authority"),
            "{action}: {error}"
        );
        assert!(!error.to_string().contains(SYNTHETIC_SECRET));
    }
    assert_eq!(
        fs::read_to_string(&key).expect("fixture key unchanged"),
        SYNTHETIC_SECRET
    );
    let ordinary = parent.join("ordinary.txt");
    fs::write(&ordinary, "ordinary content").expect("ordinary file");
    let mut request = effect_request(
        system_actor("protection-test"),
        "filesystem.read",
        ordinary.display().to_string(),
        json!({}),
    );
    request.capabilities = vec!["filesystem.read".into()];
    let result = gateway(&parent, "filesystem.read")
        .execute(request, &executor)
        .await
        .expect("ordinary granted read");
    assert_eq!(result.bytes, b"ordinary content");
}

#[tokio::test]
async fn hard_link_alias_fails_closed_before_direct_read_or_process_launch() {
    let (directory, root, protection) = fixture();
    let parent = fs::canonicalize(directory.path()).expect("parent");
    let alias = parent.join("apparently-ordinary.txt");
    fs::hard_link(root.path().join(".authority-key.env"), &alias)
        .expect("hard-link attack fixture");
    let mut request = effect_request(
        system_actor("alias-test"),
        "filesystem.read",
        alias.display().to_string(),
        json!({}),
    );
    request.capabilities = vec!["filesystem.read".into()];
    let error = gateway(&parent, "filesystem.read")
        .execute(
            request,
            &FilesystemExecutor::new().with_protected_filesystem(protection.clone()),
        )
        .await
        .expect_err("unsafe alias rejected before contents");
    assert!(error.to_string().contains("confinement is invalid"));
    assert!(!error.to_string().contains(SYNTHETIC_SECRET));
    let mut obligations = PolicyObligations {
        sandbox_backend: if cfg!(windows) {
            "windows_job"
        } else {
            "native"
        }
        .into(),
        ..PolicyObligations::default()
    };
    assert!(
        protection.restrict_process(&mut obligations).is_err(),
        "alias blocks isolated job before helper launch"
    );
    fs::remove_file(&alias).expect("remove test alias");
    protection.snapshot().expect("single-link custody restored");
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_parent_alias_and_open_descriptor_cannot_reveal_authority() {
    use std::os::unix::fs::symlink;
    let (directory, root, protection) = fixture();
    let parent = fs::canonicalize(directory.path()).expect("parent");
    let alias = parent.join("linked-parent");
    symlink(root.path(), &alias).expect("directory alias");
    let mut request = effect_request(
        system_actor("alias-test"),
        "filesystem.read",
        alias.join(".authority-key.env").display().to_string(),
        json!({}),
    );
    request.capabilities = vec!["filesystem.read".into()];
    let error = gateway(&parent, "filesystem.read")
        .execute(
            request,
            &FilesystemExecutor::new().with_protected_filesystem(protection.clone()),
        )
        .await
        .expect_err("canonical private parent rejected");
    assert!(error.to_string().contains("native credential authority"));
    let snapshot = protection.snapshot().expect("snapshot");
    let swapped = parent.join("swap-target");
    fs::write(&swapped, "ordinary").expect("ordinary leaf");
    let original = fs::File::open(&swapped).expect("ordinary descriptor");
    snapshot
        .check_file(&original)
        .expect("ordinary descriptor allowed");
    fs::remove_file(&swapped).expect("remove leaf");
    symlink(root.path().join(".authority-key.env"), &swapped)
        .expect("swap leaf after pathname check");
    let opened = fs::File::open(&swapped).expect("simulated raced descriptor");
    assert!(
        snapshot.check_file(&opened).is_err(),
        "descriptor identity rejected before bytes are read"
    );
    snapshot
        .check_file(&original)
        .expect("retained ordinary descriptor stays ordinary");
}

#[test]
fn private_authority_is_hidden_in_all_search_modes_and_isolated_jobs_only_add_denials() {
    let (directory, root, protection) = fixture();
    let parent = fs::canonicalize(directory.path()).expect("parent");
    fs::write(parent.join("visible.txt"), "public needle").expect("ordinary searchable file");
    let snapshot = protection.snapshot().expect("snapshot");
    for ambient in [false, true] {
        for scoped in [false, true] {
            let result = search_files(
                &parent,
                &json!({"pattern": "needle", "workspace_scoped": scoped}),
                64 * 1024,
                ambient,
                &[],
                &snapshot,
            )
            .expect("search");
            let value: Value = serde_json::from_slice(&result.bytes).expect("search JSON");
            assert_eq!(value["matches"].as_array().expect("matches").len(), 1);
            assert_eq!(value["matches"][0]["path"], "visible.txt");
            assert!(
                !String::from_utf8(result.bytes)
                    .expect("JSON text")
                    .contains(SYNTHETIC_SECRET)
            );
        }
    }
    let mut obligations = PolicyObligations {
        sandbox_backend: if cfg!(windows) {
            "windows_job"
        } else {
            "native"
        }
        .into(),
        filesystem: vec![FilesystemGrant {
            root: parent.join("workspace").display().to_string(),
            mode: "write".into(),
        }],
        protected_filesystem: vec!["existing-policy-denial".into()],
        ..PolicyObligations::default()
    };
    let grants = obligations.filesystem.clone();
    protection
        .restrict_process(&mut obligations)
        .expect("supported isolated job");
    assert_eq!(
        obligations.filesystem, grants,
        "deny root never expands a grant"
    );
    assert_eq!(
        obligations.protected_filesystem,
        vec![
            "existing-policy-denial".to_owned(),
            root.path().display().to_string()
        ]
    );
    obligations.allow_sandbox_downgrade = true;
    assert!(
        protection.restrict_process(&mut obligations).is_err(),
        "fallback cannot bypass native-owned deny root"
    );
    obligations.allow_sandbox_downgrade = false;
    for backend in ["external", "danger_full_access", "broker", "unknown"] {
        obligations.sandbox_backend = backend.into();
        assert!(
            protection.restrict_process(&mut obligations).is_err(),
            "reject {backend}"
        );
    }
}

#[test]
fn managed_process_clone_preserves_native_owned_denials() {
    let (_directory, root, protection) = fixture();
    let executor = SandboxProcessExecutor::new(
        SandboxExecutorConfig {
            helper_executable: std::env::current_exe().expect("helper path"),
            oci_runtime: None,
            oci_image: None,
            oci_proxy_image: None,
        },
        [79_u8; 32],
    )
    .with_protected_filesystem(protection);
    let controlled = executor.controlled(ProcessControl::default());
    let mut obligations = PolicyObligations {
        sandbox_backend: if cfg!(windows) {
            "windows_job"
        } else {
            "native"
        }
        .into(),
        ..PolicyObligations::default()
    };
    controlled
        .protected
        .restrict_process(&mut obligations)
        .expect("managed adapter retains protection");
    assert_eq!(
        obligations.protected_filesystem,
        vec![root.path().display().to_string()]
    );
}

#[test]
fn credential_control_variables_never_reach_ambient_or_explicit_process_environment() {
    let directory = tempfile::tempdir().expect("cwd");
    let cwd = fs::canonicalize(directory.path()).expect("cwd");
    let executable = std::env::current_exe().expect("fixture executable");
    let mut obligations = PolicyObligations {
        sandbox_backend: "danger_full_access".into(),
        resource_authority: ResourceAuthority::Ambient,
        max_output_bytes: 64 * 1024,
        ..PolicyObligations::default()
    };
    let mut spec = ProcessSpec {
        lifetime: None,
        cwd,
        args: vec![],
        environment: BTreeMap::new(),
        stdin_base64: None,
        stdin_completion: None,
        timeout_ms: None,
        max_output_bytes: None,
    };
    for name in [
        "COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY",
        "colossus_development_wrapping_key",
        "COLOSSUS_JOURNAL_KEY",
        "COLOSSUS_SIGNING_KEY",
        "COLOSSUS_DEV_JOURNAL_KEY",
        "colossus_dev_signing_key",
    ] {
        spec.environment = [(name.into(), SYNTHETIC_SECRET.into())].into();
        obligations.allowed_environment = vec![name.into()];
        let error = validate_process_spec(&spec, &executable.display().to_string(), &obligations)
            .expect_err("explicit reserved overlay rejected even under Full access");
        assert!(!error.to_string().contains(SYNTHETIC_SECRET));
        let mut environment = spec.environment.clone();
        inherit_ambient_environment(
            &mut environment,
            [
                (name.into(), SYNTHETIC_SECRET.into()),
                ("PATH".into(), "safe-path".into()),
            ],
        );
        assert!(
            !environment.contains_key(name),
            "reserved explicit/ambient {name} stripped"
        );
        assert_eq!(environment["PATH"], "safe-path");
    }
}

#[test]
fn replacing_retained_private_root_invalidates_protection() {
    let (directory, root, protection) = fixture();
    let replacement = fs::canonicalize(directory.path())
        .expect("parent")
        .join("old-authority");
    #[cfg(unix)]
    {
        fs::rename(root.path(), replacement).expect("replace private root");
        ConfinedRoot::bind(root.path()).expect("new empty private directory");
        assert!(
            protection.snapshot().is_err(),
            "retained authority cannot retarget"
        );
    }
    #[cfg(not(unix))]
    {
        let _ = (replacement, root);
        protection
            .snapshot()
            .expect("retained root revalidates on platform");
    }
}
