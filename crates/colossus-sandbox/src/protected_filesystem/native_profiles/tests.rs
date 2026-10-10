use super::*;
use colossus_contracts::DecisionOutcome;
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, effect_request, system_actor,
};
use colossus_testkit::InMemoryEventJournal;
use std::os::unix::{fs::symlink, net::UnixListener};

const SYNTHETIC_COOKIE: &str = "synthetic-profile-cookie-fixture";

fn fixture() -> (
    tempfile::TempDir,
    ConfinedRoot,
    ProtectedFilesystem,
    UnixListener,
) {
    let directory = tempfile::tempdir().expect("fixture");
    let parent = fs::canonicalize(directory.path()).expect("canonical");
    let root = ConfinedRoot::bind(parent.join("native-cache")).expect("native cache");
    fs::create_dir(root.path().join("context")).expect("Chromium context");
    fs::write(root.path().join("context/Cookies"), SYNTHETIC_COOKIE).expect("cookie");
    symlink(
        "unreachable-process-lock",
        root.path().join("SingletonLock"),
    )
    .expect("Chrome lock");
    symlink(
        "/nonexistent/chromium/socket",
        root.path().join("SingletonSocket"),
    )
    .expect("Chrome socket link");
    let listener = UnixListener::bind(root.path().join("owned.sock")).expect("Chrome socket");
    let protection = ProtectedFilesystem::new(Vec::new())
        .expect("no credentials")
        .with_native_profile_roots(vec![root.clone()])
        .expect("opaque native metadata");
    (directory, root, protection, listener)
}

fn gateway(parent: &Path, action: &str) -> EffectGateway {
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(action, DecisionOutcome::Allow)
                .with_filesystem_root(parent.display().to_string(), "write"),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new([action.into()]),
        [93; 32],
    )
}

async fn granted_effect(
    parent: &Path,
    target: &Path,
    action: &str,
    protection: ProtectedFilesystem,
) -> Result<colossus_policy::ReleasedEffectResult, colossus_policy::GatewayError> {
    let mut request = effect_request(
        system_actor("native-cache-protection"),
        action,
        target.display().to_string(),
        json!({"text": "replacement"}),
    );
    request.capabilities = vec![action.into()];
    gateway(parent, action)
        .execute(
            request,
            &FilesystemExecutor::new().with_protected_filesystem(protection),
        )
        .await
}

#[tokio::test]
async fn chrome_shaped_cache_remains_opaque_to_real_granted_filesystem_effects() {
    let (directory, root, protection, _listener) = fixture();
    assert!(
        ProtectedFilesystem::new(vec![root.clone()]).is_err(),
        "credential confinement remains strict despite opaque browser-cache handling"
    );
    let parent = fs::canonicalize(directory.path()).expect("parent");
    let alias = parent.join("workspace-cache-alias");
    symlink(root.path(), &alias).expect("directory alias");
    for target in [
        root.path().join("context/Cookies"),
        alias.join("context/Cookies"),
    ] {
        for action in ["filesystem.read", "filesystem.metadata", "filesystem.write"] {
            let error = granted_effect(&parent, &target, action, protection.clone())
                .await
                .expect_err("native cache overrides broad filesystem permit");
            assert!(!error.to_string().contains(SYNTHETIC_COOKIE));
        }
    }
    assert_eq!(
        fs::read_to_string(root.path().join("context/Cookies")).expect("cookie"),
        SYNTHETIC_COOKIE
    );
    let ordinary = parent.join("ordinary.txt");
    fs::write(&ordinary, "ordinary").expect("ordinary fixture");
    let result = granted_effect(&parent, &ordinary, "filesystem.read", protection)
        .await
        .expect("ordinary permit");
    assert_eq!(result.bytes, b"ordinary");
}

#[tokio::test]
async fn hardlink_alias_is_denied_and_invalidates_preexisting_snapshot() {
    let (directory, root, protection, _listener) = fixture();
    let snapshot = protection.snapshot().expect("snapshot");
    let parent = fs::canonicalize(directory.path()).expect("parent");
    let alias = parent.join("apparently-ordinary.txt");
    fs::hard_link(root.path().join("context/Cookies"), &alias).expect("hardlink fixture");
    assert!(
        snapshot.revalidate().is_err(),
        "link-count mutation invalidates retained cache identity"
    );
    assert!(
        snapshot
            .check_file(&File::open(&alias).expect("alias descriptor"))
            .is_err()
    );
    for action in ["filesystem.read", "filesystem.metadata", "filesystem.write"] {
        assert!(
            granted_effect(&parent, &alias, action, protection.clone())
                .await
                .is_err()
        );
    }
    assert_eq!(
        fs::read_to_string(&alias).expect("unchanged"),
        SYNTHETIC_COOKIE
    );
}

#[test]
fn replaced_cache_leaf_or_root_cannot_reuse_snapshot() {
    let (directory, root, protection, _listener) = fixture();
    let snapshot = protection.snapshot().expect("snapshot");
    fs::write(root.path().join("new-cookie"), "added after snapshot").expect("new native entry");
    assert!(
        snapshot.revalidate().is_err(),
        "new aliases cannot escape the captured entry set"
    );
    fs::remove_file(root.path().join("new-cookie")).expect("remove synthetic new entry");
    let snapshot = protection.snapshot().expect("stable snapshot");
    let cookie = root.path().join("context/Cookies");
    fs::rename(&cookie, root.path().join("old-cookie")).expect("retained old inode");
    fs::write(&cookie, "replacement").expect("foreign leaf");
    assert!(snapshot.revalidate().is_err());
    let fresh = protection.snapshot().expect("fresh native metadata");
    fs::rename(root.path(), directory.path().join("retained-root")).expect("move exact root");
    fs::create_dir(root.path()).expect("replacement root");
    assert!(fresh.revalidate().is_err());
    assert!(protection.snapshot().is_err());
}

#[test]
fn native_cache_masks_process_ancestor_grants_and_rejects_ambient_launch() {
    let (_directory, root, protection, _listener) = fixture();
    let mut obligations = PolicyObligations {
        sandbox_backend: "native".into(),
        resource_authority: ResourceAuthority::Declared,
        ..PolicyObligations::default()
    };
    protection
        .restrict_process(&mut obligations)
        .expect("native deny-root obligation");
    assert!(
        obligations
            .protected_filesystem
            .contains(&root.path().display().to_string())
    );
    obligations.resource_authority = ResourceAuthority::Ambient;
    assert!(protection.restrict_process(&mut obligations).is_err());
    obligations.resource_authority = ResourceAuthority::Declared;
    obligations.sandbox_backend = "danger_full_access".into();
    assert!(protection.restrict_process(&mut obligations).is_err());
}
