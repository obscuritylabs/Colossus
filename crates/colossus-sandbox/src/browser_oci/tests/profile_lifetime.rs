use super::*;
use crate::{BrowserProfileEngine, BrowserProfileError, BrowserProfileStore};
use colossus_contracts::BrowserProfileSelection;

fn store(path: &Path) -> BrowserProfileStore {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    BrowserProfileStore::bind(
        ConfinedRoot::bind(path).unwrap(),
        BrowserProfileEngine {
            cef_version: "accepted".into(),
            chromium_version: "accepted".into(),
            protocol_version: 1,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn failed_before_create_releases_exact_profile_without_dirtying_it() {
    let installation_root = tempfile::tempdir().unwrap();
    let profile_root = tempfile::tempdir().unwrap();
    let store = store(profile_root.path());
    let mut resources = resources(Arc::new(installation(installation_root.path())));
    let profile = store
        .create(&resources.binding, "Selected profile")
        .unwrap();
    resources.request.options.profile = BrowserProfileSelection::Workspace {
        id: profile.id.clone(),
    };
    resources.profile = Some(store.acquire(&resources.binding, &profile.id).unwrap());
    assert!(matches!(
        store.acquire(&resources.binding, &profile.id),
        Err(BrowserProfileError::Busy)
    ));
    resources.cleanup().await.unwrap();
    assert!(resources.closed);
    assert_eq!(
        resources.shutdown_receipt,
        OciBrowserShutdownReceipt::NoProcess
    );
    let mut subsequent = store.acquire(&resources.binding, &profile.id).unwrap();
    subsequent.retire(true).unwrap();
}

#[tokio::test]
async fn unknown_create_keeps_profile_exclusive_and_drop_does_not_prove_reuse() {
    let installation_root = tempfile::tempdir().unwrap();
    let profile_root = tempfile::tempdir().unwrap();
    let store = store(profile_root.path());
    let mut resources = resources(Arc::new(installation(installation_root.path())));
    let binding = resources.binding.clone();
    let profile = store.create(&binding, "Retained obligation").unwrap();
    resources.profile = Some(store.acquire(&binding, &profile.id).unwrap());
    // This fixture's unverified engine can never spawn. The retained uncertain
    // create intent nevertheless forbids turning an engine failure into reuse.
    resources.create_attempted = true;
    assert!(resources.cleanup().await.is_err());
    assert!(!resources.closed);
    assert!(matches!(
        store.acquire(&binding, &profile.id),
        Err(BrowserProfileError::Busy)
    ));
    drop(resources);
    assert!(matches!(
        store.acquire(&binding, &profile.id),
        Err(BrowserProfileError::OutcomeUnknown)
    ));
}

#[test]
fn persistent_mount_uses_bound_cache_and_keeps_control_and_component_read_only() {
    let installation_root = tempfile::tempdir().unwrap();
    let profile_root = tempfile::tempdir().unwrap();
    let store = store(profile_root.path());
    let binding = request().binding;
    let profile = store.create(&binding, "User label is not a path").unwrap();
    let mut lease = store.acquire(&binding, &profile.id).unwrap();
    let args = engine::arguments(
        &installation(installation_root.path()),
        "colossus-browser-fixed",
        "fixed",
        &installation_root.path().join("control"),
        Some(&lease),
    )
    .unwrap();
    let mounts: Vec<_> = args
        .windows(2)
        .filter(|pair| pair[0] == "--mount")
        .collect();
    assert_eq!(mounts.len(), 3);
    assert!(mounts[0][1].contains("dst=/opt/colossus-browser,readonly"));
    assert!(mounts[1][1].contains("dst=/run/colossus-browser-control,readonly"));
    assert_eq!(
        mounts[2][1],
        format!(
            "type=bind,src={},dst=/var/colossus-browser/profile,bind-propagation=rprivate",
            lease.cache_path().display()
        )
    );
    assert!(!args.iter().any(|arg| arg.contains("User label")));
    lease.retire(true).unwrap();
}
