use super::*;
use std::fs;

fn home() -> (tempfile::TempDir, ColossusHome) {
    #[cfg(windows)]
    let directory =
        tempfile::tempdir_in(std::env::var_os("LOCALAPPDATA").expect("Windows private parent"))
            .expect("test directory");
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().expect("test directory");
    let parent = fs::canonicalize(directory.path()).expect("canonical parent");
    let home = ColossusHome::ensure_at(parent.join("home")).expect("private test home");
    (directory, home)
}

fn status(state: ConnectorStatus) -> CloudStatus {
    CloudStatus {
        target_id: "owned-workspace".into(),
        status: state,
        node_id: Some("owned-node".into()),
        project_id: Some("owned-project".into()),
        endpoint: Some("https://control-plane.example".into()),
        host_id: Some("owned-host".into()),
        workspace_id: Some("owned-workspace".into()),
        shared_sessions: false,
        shared_continuation: false,
        sharing_supported: true,
    }
}

#[test]
fn saved_connections_survive_reopen_without_claiming_live_connectivity() {
    let (_guard, home) = home();
    access(
        Some((
            "owned-workspace".into(),
            Some(status(ConnectorStatus::Connected)),
        )),
        || Ok(home.clone()),
    )
    .expect("remember public fields");
    let reopened = ColossusHome::ensure_at(home.root()).expect("reopened home");
    let saved = access(None, || Ok(reopened)).expect("saved summary without credential store");
    assert_eq!(
        saved["owned-workspace"].status,
        ConnectorStatus::Disconnected
    );
    assert_eq!(
        saved["owned-workspace"].project_id.as_deref(),
        Some("owned-project")
    );
    let bytes =
        fs::read(home.root().join("desktop-control-plane/connections.json")).expect("public cache");
    let encoded = String::from_utf8(bytes).expect("UTF8 metadata");
    for forbidden in ["credential", "bearer", "privateKey", "certificate"] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn revocation_and_forgetting_update_the_public_summary() {
    let (_guard, home) = home();
    access(
        Some((
            "owned-workspace".into(),
            Some(status(ConnectorStatus::Revoked)),
        )),
        || Ok(home.clone()),
    )
    .expect("revoked summary");
    assert_eq!(
        access(None, || Ok(home.clone())).expect("revoked read")["owned-workspace"].status,
        ConnectorStatus::Revoked
    );
    access(Some(("owned-workspace".into(), None)), || Ok(home.clone())).expect("forget summary");
    assert!(access(None, || Ok(home)).expect("empty summary").is_empty());
}

#[test]
fn remote_revocation_updates_matching_existing_summary_and_survives_reopen() {
    let (_guard, home) = home();
    access(
        Some((
            "owned-workspace".into(),
            Some(status(ConnectorStatus::Disconnected)),
        )),
        || Ok(home.clone()),
    )
    .expect("retained enrollment");
    access_change(
        Some(Change::Revoked {
            target: "owned-workspace".into(),
            node: "owned-node".into(),
            alive: Arc::new(AtomicBool::new(true)),
        }),
        || Ok(home.clone()),
    )
    .expect("remote terminal status");
    let reopened = ColossusHome::ensure_at(home.root()).expect("reopened home");
    assert_eq!(
        access(None, || Ok(reopened)).expect("retained terminal state")["owned-workspace"].status,
        ConnectorStatus::Revoked
    );
    let saved = access(None, || Ok(home.clone())).expect("read after restart");
    let preserved = disconnected_status(&saved, "owned-workspace", "owned-node");
    assert_eq!(
        preserved,
        ConnectorStatus::Revoked,
        "read-only settings refresh retains terminal observation"
    );
    let mut disconnected = status(ConnectorStatus::Disconnected);
    disconnected.status = preserved;
    access(Some(("owned-workspace".into(), Some(disconnected))), || {
        Ok(home.clone())
    })
    .expect("later disconnect persists retained terminal state");
    assert_eq!(
        access(None, || Ok(home.clone())).expect("later reopen")["owned-workspace"].status,
        ConnectorStatus::Revoked
    );
    assert_eq!(
        disconnected_status(&saved, "owned-workspace", "replacement-node"),
        ConnectorStatus::Disconnected,
        "different enrollment never inherits old revocation"
    );
}

#[test]
fn terminal_callback_cannot_resurrect_forget_or_overwrite_a_replacement() {
    let (_guard, home) = home();
    let live = Arc::new(AtomicBool::new(true));
    access(
        Some((
            "owned-workspace".into(),
            Some(status(ConnectorStatus::Disconnected)),
        )),
        || Ok(home.clone()),
    )
    .expect("retained enrollment");
    access(Some(("owned-workspace".into(), None)), || Ok(home.clone())).expect("forget");
    access_change(
        Some(Change::Revoked {
            target: "owned-workspace".into(),
            node: "owned-node".into(),
            alive: Arc::clone(&live),
        }),
        || Ok(home.clone()),
    )
    .expect("absent enrollment never inserted even before token invalidation");
    assert!(
        access(None, || Ok(home.clone()))
            .expect("forgotten cache")
            .is_empty()
    );

    let mut replacement = status(ConnectorStatus::Disconnected);
    replacement.node_id = Some("replacement-node".into());
    access(Some(("owned-workspace".into(), Some(replacement))), || {
        Ok(home.clone())
    })
    .expect("replacement enrollment");
    access_change(
        Some(Change::Revoked {
            target: "owned-workspace".into(),
            node: "owned-node".into(),
            alive: Arc::clone(&live),
        }),
        || Ok(home.clone()),
    )
    .expect("different node not overwritten");
    assert_eq!(
        access(None, || Ok(home.clone())).expect("replacement state")["owned-workspace"].status,
        ConnectorStatus::Disconnected
    );

    // Reconnect may retain the same node. The old session token still distinguishes it.
    live.store(false, Ordering::Release);
    access(
        Some((
            "owned-workspace".into(),
            Some(status(ConnectorStatus::Disconnected)),
        )),
        || Ok(home.clone()),
    )
    .expect("same-node replacement session");
    access_change(
        Some(Change::Revoked {
            target: "owned-workspace".into(),
            node: "owned-node".into(),
            alive: live,
        }),
        || Ok(home.clone()),
    )
    .expect("dropped old session cannot mark replacement revoked");
    assert_eq!(
        access(None, || Ok(home)).expect("same-node replacement state")["owned-workspace"].status,
        ConnectorStatus::Disconnected
    );
}

#[test]
fn terminal_callback_checks_liveness_after_waiting_for_cache_lock() {
    let (_guard, home) = home();
    access(
        Some((
            "owned-workspace".into(),
            Some(status(ConnectorStatus::Disconnected)),
        )),
        || Ok(home.clone()),
    )
    .expect("retained enrollment");
    let root = ConfinedRoot::bind(home.root().join("desktop-control-plane")).expect("cache root");
    let lock = root
        .open_existing_file_read_write(Path::new("connections.lock"))
        .expect("lock");
    lock.file().lock_exclusive().expect("hold cache lock");
    let live = Arc::new(AtomicBool::new(true));
    let (entered, waiting) = std::sync::mpsc::channel();
    let worker_home = home.clone();
    let worker_live = Arc::clone(&live);
    let task = std::thread::spawn(move || {
        access_change(
            Some(Change::Revoked {
                target: "owned-workspace".into(),
                node: "owned-node".into(),
                alive: worker_live,
            }),
            || {
                entered.send(()).expect("notify before cache lock");
                Ok(worker_home)
            },
        )
    });
    waiting
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("terminal callback started");
    live.store(false, Ordering::Release);
    FileExt::unlock(lock.file()).expect("release cache lock");
    task.join()
        .expect("callback thread")
        .expect("invalidated callback is a no-op");
    assert_eq!(
        access(None, || Ok(home.clone())).expect("no stale terminal write")["owned-workspace"]
            .status,
        ConnectorStatus::Disconnected
    );
    access(Some(("owned-workspace".into(), None)), || Ok(home.clone())).expect("serialized forget");
    assert!(
        access(None, || Ok(home))
            .expect("forgotten summary")
            .is_empty()
    );
}

#[test]
fn forged_connected_states_and_secret_bearing_fields_are_rejected() {
    let (_guard, home) = home();
    access(None, || Ok(home.clone())).expect("prepare public cache");
    let path = home.root().join("desktop-control-plane/connections.json");
    let forged = BTreeMap::from([("owned-workspace", status(ConnectorStatus::Connected))]);
    fs::write(&path, serde_json::to_vec(&forged).expect("forged JSON"))
        .expect("write forged cache");
    assert!(access(None, || Ok(home.clone())).is_err());
    let mut forged = serde_json::to_value(BTreeMap::from([(
        "owned-workspace",
        status(ConnectorStatus::Disconnected),
    )]))
    .expect("summary JSON");
    forged["owned-workspace"]["bearer"] = serde_json::json!("synthetic-forbidden-field");
    fs::write(&path, serde_json::to_vec(&forged).expect("forged JSON"))
        .expect("write forbidden field");
    assert!(access(None, || Ok(home.clone())).is_err());
    fs::write(&path, vec![b' '; MAX_BYTES as usize + 1]).expect("oversized cache");
    assert!(access(None, || Ok(home)).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_cache_directory_cannot_redirect_metadata_writes() {
    let (_guard, home) = home();
    let outside = tempfile::tempdir().expect("outside test directory");
    std::os::unix::fs::symlink(outside.path(), home.root().join("desktop-control-plane"))
        .expect("directory symlink");
    assert!(
        access(
            Some((
                "owned-workspace".into(),
                Some(status(ConnectorStatus::Disconnected))
            )),
            || Ok(home)
        )
        .is_err()
    );
    assert!(
        fs::read_dir(outside.path())
            .expect("outside contents")
            .next()
            .is_none()
    );
}
