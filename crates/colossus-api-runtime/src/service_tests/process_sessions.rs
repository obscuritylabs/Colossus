use super::*;
use colossus_api::{
    ApiErrorReason, ListProcessSessionsRequest, ReadProcessSessionRequest,
    StopProcessSessionRequest,
};

pub(super) async fn inspection_and_control_require_current_scopes(runtime: Arc<Runtime>) {
    let service = service(runtime, RunAdmissionConfig::default());
    let read_only =
        caller_with_exact_scopes("app:shell-reader", "shell-reader", &[scopes::RUNS_READ]);
    let no_read =
        caller_with_exact_scopes("app:shell-reader", "shell-control", &[scopes::RUNS_CONTROL]);
    assert!(
        service
            .list_process_sessions(&read_only, ListProcessSessionsRequest { after: None })
            .await
            .expect("authorized list")
            .sessions
            .is_empty()
    );
    let denied = service
        .list_process_sessions(&no_read, ListProcessSessionsRequest { after: None })
        .await
        .expect_err("read scope required");
    assert_eq!(denied.reason, ApiErrorReason::ScopeDenied);
    let denied = service
        .read_process_session(
            &no_read,
            ReadProcessSessionRequest {
                session_id: Uuid::now_v7().to_string(),
                after_sequence: 0,
                wait_ms: 0,
                max_output_bytes: 65536,
            },
        )
        .await
        .expect_err("logs require read scope");
    assert_eq!(denied.reason, ApiErrorReason::ScopeDenied);
    let denied = service
        .stop_process_session(
            &read_only,
            StopProcessSessionRequest {
                session_id: Uuid::now_v7().to_string(),
            },
        )
        .await
        .expect_err("read cannot stop");
    assert_eq!(denied.reason, ApiErrorReason::ScopeDenied);
    let denied = service
        .stop_process_session(
            &no_read,
            StopProcessSessionRequest {
                session_id: Uuid::now_v7().to_string(),
            },
        )
        .await
        .expect_err("control alone cannot read retained logs through Stop");
    assert_eq!(denied.reason, ApiErrorReason::ScopeDenied);
    for (wait_ms, max_output_bytes) in [(30001, 65536), (0, 65537), (0, 16383)] {
        let error = service
            .read_process_session(
                &read_only,
                ReadProcessSessionRequest {
                    session_id: Uuid::now_v7().to_string(),
                    after_sequence: 0,
                    wait_ms,
                    max_output_bytes,
                },
            )
            .await
            .expect_err("bounds rejected");
        assert_eq!(error.reason, ApiErrorReason::InvalidArgument);
    }
    let owner = caller("app:shell-owner", "shell-owner");
    let id = Uuid::now_v7().to_string();
    for _ in 0..2 {
        let error = service
            .read_process_session(
                &owner,
                ReadProcessSessionRequest {
                    session_id: id.clone(),
                    after_sequence: 0,
                    wait_ms: 0,
                    max_output_bytes: 65536,
                },
            )
            .await
            .expect_err("missing shell");
        assert_eq!(error.reason, ApiErrorReason::InvalidRunTransition);
    }
    let error = service
        .stop_process_session(&owner, StopProcessSessionRequest { session_id: id })
        .await
        .expect_err("missing shell");
    assert_eq!(
        error.reason,
        ApiErrorReason::InvalidRunTransition,
        "Stop admission must remain available after read capacity is consumed"
    );
}
