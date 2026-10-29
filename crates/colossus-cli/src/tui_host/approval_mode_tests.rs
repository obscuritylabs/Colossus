use super::*;

#[test]
fn approval_mode_snapshots_advance_and_delayed_bootstrap_cannot_overwrite_them() {
    let modes = HostApprovalModes::new(None);
    assert_eq!(
        modes.snapshot().expect("initial"),
        ApprovalModeSnapshot {
            mode: None,
            revision: 0
        }
    );
    modes.initialize(ApprovalMode::Ask).expect("worker default");
    let initial = modes.snapshot().expect("initialized");
    assert_eq!(
        initial,
        ApprovalModeSnapshot {
            mode: Some(ApprovalMode::Ask),
            revision: 1
        }
    );
    modes.set(ApprovalMode::Deny).expect("explicit change");
    modes
        .initialize(ApprovalMode::FullAccess)
        .expect("delayed bootstrap ignored");
    assert_eq!(
        modes.snapshot().expect("current"),
        ApprovalModeSnapshot {
            mode: Some(ApprovalMode::Deny),
            revision: 2
        }
    );
    modes.set(ApprovalMode::RiskAuto).expect("second change");
    assert_eq!(
        modes.snapshot().expect("next"),
        ApprovalModeSnapshot {
            mode: Some(ApprovalMode::RiskAuto),
            revision: 3
        }
    );
    assert_eq!(
        initial.mode,
        Some(ApprovalMode::Ask),
        "previous snapshot stays immutable"
    );
}

#[test]
fn approval_mode_snapshot_never_pairs_a_mode_with_another_changes_revision() {
    let modes = Arc::new(HostApprovalModes::new(Some(ApprovalMode::Ask)));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let writer = {
        let modes = Arc::clone(&modes);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            for revision in 1..=2_000 {
                modes
                    .set(if revision % 2 == 1 {
                        ApprovalMode::Deny
                    } else {
                        ApprovalMode::RiskAuto
                    })
                    .expect("mode change");
            }
        })
    };
    barrier.wait();
    for _ in 0..2_000 {
        let snapshot = modes.snapshot().expect("snapshot");
        let expected = if snapshot.revision == 0 {
            ApprovalMode::Ask
        } else if snapshot.revision % 2 == 1 {
            ApprovalMode::Deny
        } else {
            ApprovalMode::RiskAuto
        };
        assert_eq!(snapshot.mode, Some(expected));
    }
    writer.join().expect("mode writer");
    assert_eq!(modes.snapshot().expect("final").revision, 2_000);
}

#[test]
fn approval_mode_revision_exhaustion_does_not_mutate_the_mode() {
    let original = ApprovalModeSnapshot {
        mode: Some(ApprovalMode::Deny),
        revision: u64::MAX,
    };
    let modes = HostApprovalModes(Mutex::new(original));
    assert!(modes.set(ApprovalMode::FullAccess).is_err());
    assert_eq!(modes.snapshot().expect("unchanged"), original);
}

#[test]
fn session_catalogue_remains_available_when_child_inventory_hits_the_payload_cap() {
    let session = SessionSummary {
        id: "session".into(),
        title: Some("Work".into()),
        created_at: "2026-10-28T12:00:00Z".into(),
        updated_at: "2026-10-28T12:00:00Z".into(),
        message_count: 1,
        last_run_id: Some("run".into()),
        last_user_preview: Some("task".into()),
    };
    let catalogue = root_session_catalogue(
        vec![session],
        Err("worker response exceeds byte limit".into()),
    );
    assert_eq!(catalogue.len(), 1);
    assert_eq!(catalogue[0].id, "session");
}
