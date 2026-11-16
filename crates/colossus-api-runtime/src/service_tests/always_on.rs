//! Durable restart boundaries for managed Always On chat wakes.

use super::*;
use colossus_api::{ApiErrorCode, ApiErrorReason};
use colossus_contracts::always_on::AlwaysOnWakeStatus;

fn managed_caller(application_id: &str, request_id: &str, scopes: &[&str]) -> CallerContext {
    CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            application_id,
            format!("credential-{request_id}"),
            ApplicationKind::Sidecar,
            scopes.iter().map(|scope| ApiScope::new(*scope).unwrap()),
            ["primary".to_owned()],
            std::iter::empty(),
        )
        .expect("managed principal"),
        RequestId::new(request_id).expect("request identity"),
    )
}

pub(super) async fn managed_orphan_wakes_settle_without_restarting(runtime: Arc<Runtime>) {
    let application_id = "app:colossus-desktop-managed";
    let scopes = [scopes::RUNS_EXECUTE, scopes::RUNS_READ];
    let original = managed_caller(application_id, "before-restart", &scopes);
    let restarted = managed_caller(application_id, "after-restart", &scopes);
    let foreign = managed_caller("app:foreign-managed", "foreign", &scopes);
    let without_read = managed_caller(application_id, "without-read", &[scopes::RUNS_EXECUTE]);
    let owner = original.actor();
    let session_id = Uuid::now_v7().to_string();
    runtime
        .create_application_session(&session_id, None, owner.clone())
        .expect("canonical managed conversation");
    let mut profile = runtime
        .enable_always_on(&owner, &session_id, 0)
        .expect("owned enrollment");
    let journal = runtime.journal();
    let repository: Arc<dyn RunRepository> =
        Arc::new(EventSourcedRunRepository::new(Arc::clone(&journal)));

    for (started, ordinary_get) in [(false, false), (false, true), (true, false)] {
        runtime
            .request_always_on_wake(&owner, profile.revision)
            .expect("explicit wake");
        let claim = runtime
            .claim_always_on_wake(&owner)
            .expect("claim read")
            .expect("durable wake claim");
        let mut input = request(&claim.wake.idempotency_key(), &claim.prompt);
        input.session_id = Some(session_id.clone());

        let api = service(Arc::clone(&runtime), RunAdmissionConfig::default());
        let existing = journal.list_stream_ids("api-run:", None, 128).unwrap();
        assert_eq!(api.resolve_host_wake(&restarted, &input).unwrap(), None);
        assert_eq!(
            journal.list_stream_ids("api-run:", None, 128).unwrap(),
            existing
        );

        // Persist the public allocation without registering a live execution.
        // The new service sees the durable run with an empty ownership registry.
        let run_id = Uuid::now_v7().to_string();
        let allocation = NewRun::from_request(&run_id, &session_id, "primary", &input).unwrap();
        let orphan = repository
            .create_run(&original, &input, &allocation)
            .expect("allocate the exact wake")
            .value;
        if started {
            repository
                .append_update(
                    &original,
                    &run_id,
                    orphan.last_sequence,
                    RunUpdateKind::State {
                        status: RunStatus::Running,
                    },
                )
                .expect("persist execution start before process loss");
        }

        let api = service(Arc::clone(&runtime), RunAdmissionConfig::default());
        let allocated = journal.list_stream_ids("api-run:", None, 128).unwrap();
        assert_eq!(allocated.len(), existing.len() + 1);
        assert_eq!(api.resolve_host_wake(&foreign, &input).unwrap(), None);
        assert_eq!(
            api.resolve_host_wake(&without_read, &input)
                .unwrap_err()
                .code,
            ApiErrorCode::PermissionDenied
        );
        let mut different_prompt = input.clone();
        different_prompt.input = vec![ContentPart::Text {
            text: "Different intent with the same wake key".into(),
        }];
        assert_eq!(
            api.resolve_host_wake(&restarted, &different_prompt)
                .unwrap_err()
                .reason,
            ApiErrorReason::IdempotencyKeyReused
        );

        let expected = if started {
            RunStatus::OutcomeUnknown
        } else {
            RunStatus::Interrupted
        };
        if ordinary_get {
            let recovered = api
                .get_run(
                    &restarted,
                    GetRunRequest {
                        run_id: run_id.clone(),
                    },
                )
                .await
                .expect("ordinary read of a claimed host wake");
            assert_eq!(recovered.id, run_id);
            assert_eq!(recovered.status, expected);
        } else {
            assert_eq!(
                api.resolve_host_wake(&restarted, &input).unwrap(),
                Some(expected)
            );
        }
        let terminal = repository.get_run(&restarted, &run_id).unwrap().unwrap();
        assert_eq!(terminal.status, expected);
        assert_eq!(terminal.started_at.is_some(), started);
        let (outcome, wake_status) = if started {
            (
                OutcomeCertainty::Unknown,
                AlwaysOnWakeStatus::OutcomeUnknown,
            )
        } else {
            (OutcomeCertainty::Known, AlwaysOnWakeStatus::Interrupted)
        };
        assert_eq!(terminal.failure.unwrap().outcome, outcome);
        assert_eq!(
            api.resolve_host_wake(&restarted, &input).unwrap(),
            Some(expected)
        );
        assert_eq!(
            repository
                .get_run(&restarted, &run_id)
                .unwrap()
                .unwrap()
                .last_sequence,
            terminal.last_sequence
        );
        assert_eq!(api.active_execution_count(), 0);
        assert_eq!(api.pending_recovery_count(), 0);
        assert!(runtime.session_messages(&session_id).unwrap().is_empty());
        assert!(
            journal
                .read_stream(&format!("run:{run_id}"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            journal.list_stream_ids("api-run:", None, 128).unwrap(),
            allocated
        );

        profile = runtime
            .finish_always_on_wake(&owner, &claim.wake.id, wake_status)
            .expect("durable wake settlement");
        assert!(profile.pending.is_none());
        assert_eq!(profile.enabled, !started);
    }
}
