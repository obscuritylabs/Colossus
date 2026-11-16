use super::*;
use crate::{RuntimeConfig, RuntimeOpenOptions, test_support::private_tempdir};
use colossus_policy::DenyApproval;
use std::{path::Path, sync::Arc};

fn open(workspace: &Path) -> Runtime {
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.memory.index_enabled = false;
    Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace).unwrap(),
    )
    .unwrap()
}

fn origin() -> OffsetDateTime {
    OffsetDateTime::parse("2026-11-16T00:00:00Z", &Rfc3339).unwrap()
}

#[test]
fn profile_requires_exact_session_owner_and_reviewed_revision() {
    let directory = private_tempdir();
    let runtime = open(directory.path());
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "app:fixture".into(),
    };
    let session = Uuid::now_v7().to_string();
    runtime
        .create_application_session(&session, Some("Owned"), owner.clone())
        .unwrap();
    let terminal = terminal_actor();

    assert!(matches!(
        runtime.enable_always_on(&terminal, &session, 0),
        Err(RuntimeError::Store(StoreError::NotFound(_)))
    ));
    let profile = runtime.enable_always_on(&owner, &session, 0).unwrap();
    assert!(matches!(
        runtime.always_on_profile(&terminal),
        Err(RuntimeError::Store(StoreError::NotFound(_)))
    ));
    assert!(matches!(
        runtime.pause_always_on(&owner, 0),
        Err(RuntimeError::Store(StoreError::Conflict { .. }))
    ));

    let pending = runtime
        .request_always_on_wake(&owner, profile.revision)
        .unwrap();
    assert_eq!(
        runtime
            .request_always_on_wake(&owner, pending.revision)
            .unwrap(),
        pending
    );
    let paused = runtime.pause_always_on(&owner, pending.revision).unwrap();
    assert!(!paused.enabled);
    assert!(paused.pending.is_none());
    assert_eq!(
        paused.last_wake.unwrap().status,
        AlwaysOnWakeStatus::Interrupted
    );
}

#[test]
fn paused_profile_can_rebind_only_after_its_claim_settles() {
    let directory = private_tempdir();
    let runtime = open(directory.path());
    let owner = terminal_actor();
    let first = runtime.create_session(Some("First chat")).unwrap().id;
    let next = runtime.create_session(Some("Second chat")).unwrap().id;
    let time = origin();
    let enabled = runtime
        .enable_always_on_at(&owner, &first, 0, time)
        .unwrap();
    assert!(
        runtime
            .enable_always_on(&owner, &next, enabled.revision)
            .is_err()
    );
    let due = time + INTERVAL;
    let claimed = runtime.claim_always_on_at(&owner, due).unwrap().unwrap();
    let current = runtime.always_on_profile(&owner).unwrap().unwrap();
    let paused = runtime.pause_always_on(&owner, current.revision).unwrap();
    assert!(paused.pending.is_some());
    assert!(
        runtime
            .enable_always_on(&owner, &next, paused.revision)
            .is_err()
    );
    assert!(runtime.set_session_archived(&first, true).is_err());

    let settled = runtime
        .finish_always_on_wake(&owner, &claimed.wake.id, AlwaysOnWakeStatus::Interrupted)
        .unwrap();
    runtime.set_session_archived(&first, true).unwrap();
    assert!(
        runtime
            .enable_always_on(&owner, &first, settled.revision)
            .is_err()
    );
    assert!(matches!(
        runtime.enable_always_on(&owner, &next, paused.revision),
        Err(RuntimeError::Store(StoreError::Conflict { .. }))
    ));
    let rebound = runtime
        .enable_always_on_at(&owner, &next, settled.revision, time)
        .unwrap();
    assert_eq!(rebound.session_id, next);
    assert_eq!(rebound.last_wake, settled.last_wake);

    let next_claim = runtime.claim_always_on_at(&owner, due).unwrap().unwrap();
    assert_eq!(next_claim.session_id, next);
    assert_eq!(next_claim.wake.requested_at, claimed.wake.requested_at);
    assert_ne!(next_claim.wake.id, claimed.wake.id);
    assert_eq!(
        runtime
            .finish_always_on_wake(&owner, &claimed.wake.id, AlwaysOnWakeStatus::Interrupted,)
            .unwrap()
            .pending,
        Some(next_claim.wake)
    );
}

#[tokio::test]
async fn claim_survives_restart_without_replaying_unconfirmed_execution() {
    let directory = private_tempdir();
    let runtime = open(directory.path());
    let owner = terminal_actor();
    let session = runtime.create_session(Some("Standing chat")).unwrap().id;
    let profile = runtime
        .enable_always_on_at(&owner, &session, 0, origin())
        .unwrap();
    assert_eq!(profile.next_wake_at, "2026-11-16T00:30:00Z");
    let late = origin() + Duration::minutes(96);
    let claim = runtime.claim_always_on_at(&owner, late).unwrap().unwrap();
    assert!(!claim.resumed);
    assert_eq!(claim.session_id, session);
    assert_eq!(claim.wake.trigger, AlwaysOnTrigger::Scheduled);
    assert_eq!(claim.wake.requested_at, "2026-11-16T01:30:00Z");
    assert_eq!(
        runtime
            .always_on_profile(&owner)
            .unwrap()
            .unwrap()
            .next_wake_at,
        "2026-11-16T02:00:00Z"
    );
    drop(runtime);

    let runtime = open(directory.path());
    let resumed = runtime.claim_always_on_at(&owner, late).unwrap().unwrap();
    assert!(resumed.resumed);
    assert_eq!(resumed.wake, claim.wake);
    assert_eq!(resumed.prompt, claim.prompt);
    let status = runtime.run_always_on_local_wake(&resumed).await.unwrap();
    assert_eq!(status, AlwaysOnWakeStatus::Interrupted);
    let settled = runtime
        .finish_always_on_wake(&owner, &claim.wake.id, status)
        .unwrap();
    assert!(settled.pending.is_none());
    assert_eq!(
        runtime
            .finish_always_on_wake(&owner, &claim.wake.id, status)
            .unwrap(),
        settled
    );
    assert!(runtime.claim_always_on_at(&owner, late).unwrap().is_none());
    assert!(runtime.session_messages(&session).unwrap().is_empty());
}

#[tokio::test]
async fn same_session_echo_wake_runs_once_and_releases_only_status() {
    let directory = private_tempdir();
    let runtime = open(directory.path());
    let owner = terminal_actor();
    let session = runtime.create_session(Some("Standing chat")).unwrap().id;
    runtime
        .create_task(
            &session,
            "private title",
            "private body",
            TaskStatus::Blocked,
        )
        .await
        .unwrap();
    let profile = runtime.enable_always_on(&owner, &session, 0).unwrap();
    runtime
        .request_always_on_wake(&owner, profile.revision)
        .unwrap();
    let claim = runtime.claim_always_on_wake(&owner).unwrap().unwrap();
    assert!(!claim.prompt.contains("private"));
    assert!(
        claim
            .prompt
            .contains("Tasks sampled: 1. Open: 1. Blocked: 1.")
    );
    let status = runtime.run_always_on_local_wake(&claim).await.unwrap();
    assert_eq!(status, AlwaysOnWakeStatus::Completed);
    let messages = runtime.session_messages(&session).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        runtime.run_always_on_local_wake(&claim).await.unwrap(),
        AlwaysOnWakeStatus::Completed
    );
    assert_eq!(runtime.session_messages(&session).unwrap(), messages);
    runtime
        .finish_always_on_wake(&owner, &claim.wake.id, status)
        .unwrap();
}

#[test]
fn enrolled_session_fences_overlapping_wakes_and_blocks_when_archived() {
    let directory = private_tempdir();
    let runtime = open(directory.path());
    let owner = terminal_actor();
    let session = runtime.create_session(Some("Standing chat")).unwrap().id;
    let profile = runtime.enable_always_on(&owner, &session, 0).unwrap();
    runtime
        .request_always_on_wake(&owner, profile.revision)
        .unwrap();
    let admission = || ExecutionContext {
        run_id: Some(Uuid::now_v7().to_string()),
        session_id: Some(session.clone()),
        ..Default::default()
    };
    let active = admission();
    runtime.native_session_lifecycle.begin_run(&active).unwrap();
    assert!(
        runtime
            .native_session_lifecycle
            .begin_run(&admission())
            .is_err()
    );
    assert!(runtime.claim_always_on_wake(&owner).unwrap().is_none());
    runtime
        .native_session_lifecycle
        .finish_run(active.run_id.as_deref().unwrap());
    runtime.set_session_archived(&session, true).unwrap();

    assert!(runtime.claim_always_on_wake(&owner).unwrap().is_none());
    let profile = runtime.always_on_profile(&owner).unwrap().unwrap();
    assert!(profile.pending.is_none());
    assert!(!profile.enabled);
    assert_eq!(
        profile.last_wake.unwrap().status,
        AlwaysOnWakeStatus::Blocked
    );
}
