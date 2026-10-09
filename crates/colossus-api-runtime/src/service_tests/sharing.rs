use super::*;
use colossus_api::{ApiErrorReason, ListRunsRequest, SetWorkspaceSharingRequest};

fn page(token: Option<String>) -> ListRunsRequest {
    ListRunsRequest {
        session_id: None,
        statuses: vec![],
        page_size: 1,
        page_token: token,
        include_archived: true,
    }
}
fn history(run_id: &str) -> ListSessionActivityRequest {
    ListSessionActivityRequest {
        source_run_id: run_id.into(),
        query: String::new(),
        lanes: vec![colossus_api::SessionActivityLane::Agent],
        kinds: vec![
            colossus_api::SessionActivityKind::User,
            colossus_api::SessionActivityKind::Assistant,
        ],
        statuses: vec![],
        page_size: 32,
        page_token: None,
    }
}

#[test]
fn explicit_workspace_sharing_preserves_source_and_recipient_authority() {
    const CHILD: &str = "COLOSSUS_API_SHARING_TEST_CHILD";
    if env::var_os(CHILD).is_none() {
        let status = Command::new(env::current_exe().expect("test executable"))
            .args(["--exact","service_tests::sharing::explicit_workspace_sharing_preserves_source_and_recipient_authority","--nocapture"])
            .env(CHILD,"1").env("COLOSSUS_API_RUNTIME_TEST_JOURNAL","11".repeat(32))
            .env("COLOSSUS_API_RUNTIME_TEST_SIGNING","22".repeat(32)).status().expect("isolated sharing test");
        assert!(status.success());
        return;
    }
    let fixture = runtime_fixture();
    let api = service(
        fixture.runtime.clone(),
        RunAdmissionConfig::default()
            .with_list_limits(4, 1, 8, 16, 2, 16)
            .expect("sharing fixture list admission"),
    );
    let owner = caller("app:desktop-sharing", "sharing-owner");
    let cloud = caller("app:cloud-sharing", "sharing-cloud");
    let other = caller("app:other-sharing", "sharing-other");
    let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
    let sharing_stream = format!(
        "api.workspace.sharing:{}",
        hex::encode(Sha256::digest(b"app:cloud-sharing"))
    );
    let sharing_events = || {
        fixture
            .runtime
            .journal()
            .read_stream(&sharing_stream)
            .expect("canonical sharing events")
            .len()
    };
    for number in 0..2 {
        let create = request(&format!("sharing-create-{number}"), "Local conversation");
        let run = NewRun::from_request(
            format!("sharing-run-{number}"),
            format!("sharing-session-{number}"),
            "primary",
            &create,
        )
        .expect("run");
        repository
            .create_run(&owner, &create, &run)
            .expect("source run");
        fixture
            .runtime
            .create_application_session(
                &format!("sharing-session-{number}"),
                Some("Local conversation"),
                owner.actor(),
            )
            .expect("session");
    }
    for (role, text) in [
        (
            ModelMessageRole::User,
            "Full local question survives history sharing.",
        ),
        (ModelMessageRole::Assistant, "Released local answer."),
        (
            ModelMessageRole::Tool,
            "Tool lane must not become cloud conversation.",
        ),
    ] {
        let tool_call_id =
            matches!(role, ModelMessageRole::Tool).then(|| "shared-history-tool".into());
        fixture
            .runtime
            .append_application_message(
                "sharing-session-0",
                "sharing-run-0",
                colossus_contracts::ModelMessage {
                    agent_message_origin: None,
                    role,
                    content: ModelContent::from(text),
                    tool_call_id,
                    tool_calls: Vec::new(),
                },
                owner.actor(),
            )
            .expect("canonical released fixture message");
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            assert!(
                api.list_visible_runs(&cloud, page(None))
                    .await
                    .expect("private page")
                    .runs
                    .is_empty()
            );
            assert!(
                api.get_run(
                    &cloud,
                    GetRunRequest {
                        run_id: "sharing-run-0".into()
                    }
                )
                .await
                .is_err()
            );
            let read_only =
                caller_with_exact_scopes("app:desktop-sharing", "read-only", &[scopes::RUNS_READ]);
            assert_eq!(
                api.set_workspace_sharing(
                    &read_only,
                    SetWorkspaceSharingRequest {
                        recipient_application_id: "app:cloud-sharing".into(),
                        enabled: false,
                        allow_continuation: false
                    }
                )
                .await
                .expect_err("an idempotent private request still requires source control")
                .reason,
                ApiErrorReason::ScopeDenied
            );
            for _ in 0..4097 {
                let private = api
                    .set_workspace_sharing(
                        &owner,
                        SetWorkspaceSharingRequest {
                            recipient_application_id: "app:cloud-sharing".into(),
                            enabled: false,
                            allow_continuation: false,
                        },
                    )
                    .await
                    .expect(
                        "private reconnect reconciliation cannot exhaust the 4096-event budget",
                    );
                assert!(!private.enabled && !private.allow_continuation);
            }
            assert_eq!(
                sharing_events(),
                0,
                "default private requests do not append permission changes"
            );
            assert_eq!(
                api.set_workspace_sharing(
                    &read_only,
                    SetWorkspaceSharingRequest {
                        recipient_application_id: "app:cloud-sharing".into(),
                        enabled: true,
                        allow_continuation: false,
                    }
                )
                .await
                .expect_err("source control scope")
                .reason,
                ApiErrorReason::ScopeDenied
            );
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:cloud-sharing".into(),
                    enabled: true,
                    allow_continuation: false,
                },
            )
            .await
            .expect("explicit share");
            assert_eq!(sharing_events(), 1, "an actual enable remains durable");
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:cloud-sharing".into(),
                    enabled: true,
                    allow_continuation: false,
                },
            )
            .await
            .expect("same enabled tuple");
            assert_eq!(sharing_events(), 1);
            let canonical = api
                .list_session_activity(&cloud, history("sharing-run-0"))
                .await
                .expect("shared canonical conversation");
            assert!(canonical.activities.iter().any(|activity| {
                activity.input.as_ref().is_some_and(|input| {
                    input.format == "text" && input.value.contains("Full local question")
                })
            }));
            assert!(canonical.activities.iter().all(|activity| activity.lane
                == colossus_api::SessionActivityLane::Agent
                && matches!(
                    activity.kind,
                    colossus_api::SessionActivityKind::User
                        | colossus_api::SessionActivityKind::Assistant
                )));
            assert!(!canonical.activities.iter().any(|activity| {
                activity
                    .input
                    .as_ref()
                    .is_some_and(|input| input.value.contains("Tool lane"))
                    || activity
                        .result
                        .as_ref()
                        .is_some_and(|result| result.value.contains("Tool lane"))
            }));
            let first = api
                .list_visible_runs(&cloud, page(None))
                .await
                .expect("shared page");
            assert_eq!(first.runs.len(), 1);
            assert!(!first.runs[0].controllable);
            assert!(!first.runs[0].continuable);
            let next = first.next_page_token.expect("shared cursor");
            assert_eq!(
                api.list_visible_runs(&cloud, page(Some(next.clone())))
                    .await
                    .expect("next page")
                    .runs
                    .len(),
                1
            );
            assert!(
                api.list_visible_runs(&other, page(None))
                    .await
                    .expect("other page")
                    .runs
                    .is_empty()
            );
            let shared = api
                .get_run(
                    &cloud,
                    GetRunRequest {
                        run_id: "sharing-run-0".into(),
                    },
                )
                .await
                .expect("shared read");
            assert_eq!(shared.id, "sharing-run-0");
            assert_eq!(
                api.cancel_run(
                    &cloud,
                    CancelRunRequest {
                        run_id: "sharing-run-0".into(),
                        idempotency_key: IdempotencyKey::new("shared-cancel").expect("key")
                    }
                )
                .await
                .expect_err("shared runs read only")
                .reason,
                ApiErrorReason::RunNotFound
            );
            let mut continuation = request("shared-continuation-denied", "Continue");
            continuation.session_id = Some("sharing-session-0".into());
            assert_eq!(
                api.create_run(&cloud, continuation)
                    .await
                    .expect_err("read-only share cannot continue")
                    .reason,
                ApiErrorReason::RunNotFound
            );
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:cloud-sharing".into(),
                    enabled: true,
                    allow_continuation: true,
                },
            )
            .await
            .expect("allow continuation");
            let visible = api
                .list_visible_runs(&cloud, page(None))
                .await
                .expect("continuable");
            assert!(visible.runs[0].continuable);
            let mut continuation = request("shared-continuation-allowed", "Continue");
            continuation.session_id = Some("sharing-session-0".into());
            let run = api
                .create_run(&cloud, continuation)
                .await
                .expect("cloud-owned continuation")
                .run;
            let execution = repository
                .execution_request(&cloud, &run.id)
                .expect("cloud captured grant")
                .expect("execution");
            assert_eq!(execution.application_id, "app:cloud-sharing");
            assert!(execution.allowed_tools.is_empty());
            assert!(
                repository
                    .get_run(&owner, &run.id)
                    .expect("owner read")
                    .is_none()
            );
            api.set_workspace_sharing(
                &owner,
                SetWorkspaceSharingRequest {
                    recipient_application_id: "app:cloud-sharing".into(),
                    enabled: false,
                    allow_continuation: false,
                },
            )
            .await
            .expect("revoke sharing");
            assert_eq!(
                sharing_events(),
                3,
                "enable, continuation change and disable each append once"
            );
            for _ in 0..4097 {
                api.set_workspace_sharing(
                    &owner,
                    SetWorkspaceSharingRequest {
                        recipient_application_id: "app:cloud-sharing".into(),
                        enabled: false,
                        allow_continuation: false,
                    },
                )
                .await
                .expect("already-disabled reconnect reconciliation cannot consume journal budget");
            }
            assert_eq!(
                sharing_events(),
                3,
                "same disabled tuple preserves audit history and its remaining budget"
            );
            assert!(
                api.list_session_activity(&cloud, history("sharing-run-0"))
                    .await
                    .is_err()
            );
            assert!(
                api.list_session_activity(&cloud, history(&run.id))
                    .await
                    .is_err(),
                "cloud-owned continuation cannot bypass source history revocation"
            );
            let retained = api
                .list_visible_runs(&cloud, page(None))
                .await
                .expect("cloud-owned run remains visible");
            assert_eq!(retained.runs[0].run.id, run.id);
            assert!(retained.runs[0].controllable);
            assert!(
                !retained.runs[0].continuable,
                "canonical source session sharing was revoked"
            );
            assert!(
                api.get_run(
                    &cloud,
                    GetRunRequest {
                        run_id: "sharing-run-0".into()
                    }
                )
                .await
                .is_err()
            );
            assert!(
                api.list_visible_runs(&cloud, page(Some(next)))
                    .await
                    .is_err(),
                "cursor invalidated on share change"
            );
            wait_inactive(&api).await;
        });
}
