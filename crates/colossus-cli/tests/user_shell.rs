//! Explicit user commands use real helpers without a configured model provider.
#![cfg(unix)]

use colossus_contracts::{ActorType, MemoryScope};
use colossus_policy::DenyApproval;
use colossus_ports::RunControl;
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, SandboxConfig};
use std::{path::Path, sync::Arc, time::Duration};

fn runtime(root: &Path, require_approval: bool) -> Runtime {
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.use_ephemeral_storage();
    config.memory.index_enabled = false;
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("user-workflows");
    config.sandbox = SandboxConfig {
        helper_path: Some(env!("CARGO_BIN_EXE_colossus").into()),
        timeout_ms: 10_000,
        ..SandboxConfig::default()
    };
    if require_approval {
        config
            .access
            .actions
            .require_approval
            .push("shell.run".into());
    }
    Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(root).expect("workspace"),
    )
    .expect("offline runtime")
}

#[tokio::test]
async fn sidebar_memory_count_uses_retained_identities_without_read_effects() {
    let directory = tempfile::tempdir().expect("workspace");
    let runtime = runtime(directory.path(), false);
    assert_eq!(
        runtime.memory_record_count().expect("empty count"),
        (0, false)
    );
    let memory = runtime
        .create_memory(
            MemoryScope::Global,
            "note",
            1.0,
            "Memory content stays out of status metadata.",
            "status fixture",
            None,
        )
        .await
        .expect("memory");
    runtime
        .archive_memory(&memory.id)
        .await
        .expect("archive memory");
    let before = runtime
        .journal()
        .read_global(1, 100)
        .expect("journal")
        .len();
    assert_eq!(
        runtime.memory_record_count().expect("retained count"),
        (1, false)
    );
    let after = runtime
        .journal()
        .read_global(1, 100)
        .expect("journal")
        .len();
    assert_eq!(
        before, after,
        "a metadata count must not issue memory read effects"
    );
}

#[tokio::test]
async fn user_shell_sources_scripts_and_preserves_output_exit_and_user_audit() {
    let directory = tempfile::tempdir().expect("workspace");
    std::fs::write(
        directory.path().join("environment.sh"),
        "export SHELL_FIXTURE='two  spaces'\n",
    )
    .expect("source script");
    let runtime = runtime(directory.path(), false);
    let session = runtime.create_session(None).expect("session");
    let result = runtime.run_user_shell(&session.id,
        ". ./environment.sh && printf '%s' \"$SHELL_FIXTURE\"; printf 'failure detail' >&2; exit 7",
        &RunControl::default()).await.expect("shell result");
    assert_eq!(result["stdout"], "two  spaces");
    assert_eq!(result["stderr"], "failure detail");
    assert_eq!(result["exit_code"], 7);
    assert_eq!(result["stopped"], false);
    let events = runtime.journal().read_global(1, 100).expect("audit");
    let request = events
        .iter()
        .find(|event| event.event_type == "effect.requested.v1")
        .expect("effect requested");
    assert_eq!(request.actor.actor_type, ActorType::User);
    assert_eq!(
        request.context.session_id.as_deref(),
        Some(session.id.as_str())
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type.starts_with("agent.run."))
    );
    let fresh = runtime
        .run_user_shell(
            &session.id,
            "printf '%s' \"${SHELL_FIXTURE-unset}\"",
            &RunControl::default(),
        )
        .await
        .expect("fresh shell");
    assert_eq!(fresh["stdout"], "unset");
}

#[tokio::test]
async fn user_shell_cancellation_reaps_the_process_before_returning() {
    let directory = tempfile::tempdir().expect("workspace");
    let runtime = runtime(directory.path(), false);
    let session = runtime.create_session(None).expect("session");
    let control = RunControl::default();
    let cancel = control.clone();
    let started = directory.path().join("started");
    let observed = started.clone();
    let task = tokio::spawn(async move {
        for _ in 0..500 {
            if observed.exists() {
                cancel.cancel();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("shell did not start");
    });
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        runtime.run_user_shell(
            &session.id,
            "printf started > started; sleep 2; printf leaked > after-cancel",
            &control,
        ),
    )
    .await
    .expect("bounded cancellation")
    .expect("confirmed stop");
    task.await.expect("cancel task");
    assert_eq!(result["stopped"], true);
    assert!(started.exists());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(
        !directory.path().join("after-cancel").exists(),
        "cancelled descendants kept running"
    );
}

#[tokio::test]
async fn user_shell_cannot_skip_approval_or_run_for_an_unknown_session() {
    let directory = tempfile::tempdir().expect("workspace");
    let runtime = runtime(directory.path(), true);
    let session = runtime.create_session(None).expect("session");
    assert!(
        runtime
            .run_user_shell(
                &session.id,
                "printf bypass > bypass",
                &RunControl::default()
            )
            .await
            .is_err()
    );
    assert!(!directory.path().join("bypass").exists());
    assert!(
        runtime
            .run_user_shell(
                "missing-session",
                "printf bypass > bypass",
                &RunControl::default()
            )
            .await
            .is_err()
    );
    assert!(!directory.path().join("bypass").exists());
}
