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
            branch_snapshots_require_current_source_session_authority().await;
        });
}

async fn branch_snapshots_require_current_source_session_authority() {
    use colossus_api::ArtifactApi as _;
    let fixture = runtime_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_additional_scopes(
        "app:fork-owner",
        "fork-owner",
        &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
    );
    let recipient = caller_with_additional_scopes(
        "app:fork-recipient",
        "fork-recipient",
        &[scopes::ARTIFACTS_READ],
    );
    let artifacts = EventSourcedArtifactApi::new(fixture.runtime.journal());
    let document = super::document_inputs::upload(
        &artifacts,
        &owner,
        "shared.pdf",
        "application/pdf",
        b"%PDF-1.7\nprivate shared input\n%%EOF\n".to_vec(),
    )
    .await;
    let ContentPart::Artifact { artifact_id } = document else {
        unreachable!()
    };
    let source_session = Uuid::now_v7().to_string();
    fixture
        .runtime
        .create_application_session(&source_session, None, owner.actor())
        .unwrap();
    let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
    let source_run_id = Uuid::now_v7().to_string();
    let file = fixture
        .runtime
        .run_input_file_reference(owner.principal().application_id(), &artifact_id)
        .unwrap();
    fixture
        .runtime
        .append_application_message(
            &source_session,
            &source_run_id,
            colossus_contracts::ModelMessage {
                agent_message_origin: None,
                role: ModelMessageRole::User,
                content: ModelContent::Parts(vec![ModelContentPart::File { file: file.clone() }]),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            owner.actor(),
        )
        .unwrap();
    api.set_workspace_sharing(
        &owner,
        SetWorkspaceSharingRequest {
            recipient_application_id: recipient.principal().application_id().into(),
            enabled: true,
            allow_continuation: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        artifacts
            .download(&recipient, &artifact_id, 0)
            .await
            .unwrap_err()
            .reason,
        ApiErrorReason::ArtifactNotFound
    );
    let mut continuation = request("fork-intermediate", "Continue the shared context");
    continuation.session_id = Some(source_session.clone());
    let intermediate = api.create_run(&recipient, continuation).await.unwrap().run;
    wait_terminal(&api, &recipient, &intermediate.id).await;
    wait_inactive(&api).await;
    assert_eq!(intermediate.session_id, source_session);
    assert!(
        repository
            .get_run(&recipient, &intermediate.id)
            .unwrap()
            .is_some()
    );
    assert!(
        repository
            .get_run(&owner, &intermediate.id)
            .unwrap()
            .is_none()
    );
    let mut branch = request("fork-active", "Snapshot the authorized context");
    branch.branch = Some(RunBranch {
        source_run_id: intermediate.id.clone(),
        source_message_count: fixture
            .runtime
            .get_session(&source_session)
            .unwrap()
            .unwrap()
            .message_count,
        context_mode: RunBranchContextMode::Exact,
    });
    let snapshot = api
        .create_run(&recipient, branch.clone())
        .await
        .unwrap()
        .run;
    wait_terminal(&api, &recipient, &snapshot.id).await;
    wait_inactive(&api).await;
    let messages = repository
        .updates_after(&recipient, &snapshot.id, 0, 64)
        .unwrap()
        .into_iter()
        .filter_map(|update| match update.kind {
            RunUpdateKind::Message { message } => Some(message),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    let copied_count = fixture
        .runtime
        .session_branch_input_boundary(&snapshot.session_id, &snapshot.id)
        .unwrap()
        .unwrap();
    assert!(messages[0].sequence > copied_count);
    assert_eq!(
        messages[0].content,
        vec![colossus_api::ReleasedContentPart::Text {
            text: "Snapshot the authorized context".into(),
        }]
    );

    // The copied tail deliberately equals the new accepted prompt. Neither a
    // live pre-input panic nor recovery may turn that prefix into a new Message.
    let mut matching = request("fork-before-input-panic", "Continue the shared context");
    matching.branch = branch.branch.clone();
    let effects_before = fixture
        .runtime
        .journal()
        .read_global(1, 10_000)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "effect.started.v1")
        .map(|event| event.global_sequence)
        .collect::<Vec<_>>();
    api.inject_next_execution_fault(ExecutionTestFault::Panic);
    let interrupted = api
        .create_run(&recipient, matching.clone())
        .await
        .unwrap()
        .run;
    assert_eq!(
        wait_terminal(&api, &recipient, &interrupted.id)
            .await
            .status,
        RunStatus::OutcomeUnknown
    );
    wait_inactive(&api).await;
    assert!(
        !repository
            .updates_after(&recipient, &interrupted.id, 0, 64)
            .unwrap()
            .iter()
            .any(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
    );

    matching.idempotency_key = IdempotencyKey::new("fork-recover-before-input").unwrap();
    let orphan = NewRun::from_request(
        Uuid::now_v7().to_string(),
        Uuid::now_v7().to_string(),
        "primary",
        &matching,
    )
    .unwrap();
    let orphan = repository
        .create_run(&recipient, &matching, &orphan)
        .unwrap()
        .value;
    fixture
        .runtime
        .ensure_session_branch(
            &source_session,
            matching.branch.as_ref().unwrap().source_message_count,
            RunBranchContextMode::Exact,
            &orphan.session_id,
            &orphan.id,
            recipient.actor(),
        )
        .unwrap();
    repository
        .append_update(
            &recipient,
            &orphan.id,
            orphan.last_sequence,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    let recovered = api
        .get_run(
            &recipient,
            GetRunRequest {
                run_id: orphan.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(recovered.status, RunStatus::OutcomeUnknown);
    assert!(
        !repository
            .updates_after(&recipient, &orphan.id, 0, 64)
            .unwrap()
            .iter()
            .any(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
    );

    // An old/incomplete branch without a receipt remains uncertain. Its copied
    // matching User is not sufficient evidence of a newly consumed input.
    matching.idempotency_key = IdempotencyKey::new("fork-missing-receipt").unwrap();
    let missing = NewRun::from_request(
        Uuid::now_v7().to_string(),
        Uuid::now_v7().to_string(),
        "primary",
        &matching,
    )
    .unwrap();
    let missing = repository
        .create_run(&recipient, &matching, &missing)
        .unwrap()
        .value;
    fixture
        .runtime
        .create_application_session(&missing.session_id, None, recipient.actor())
        .unwrap();
    fixture
        .runtime
        .append_application_message(
            &missing.session_id,
            &missing.id,
            colossus_contracts::ModelMessage {
                agent_message_origin: None,
                role: ModelMessageRole::User,
                content: "Continue the shared context".into(),
                tool_call_id: None,
                tool_calls: vec![],
            },
            recipient.actor(),
        )
        .unwrap();
    repository
        .append_update(
            &recipient,
            &missing.id,
            missing.last_sequence,
            RunUpdateKind::State {
                status: RunStatus::Running,
            },
        )
        .unwrap();
    assert!(
        api.get_run(
            &recipient,
            GetRunRequest {
                run_id: missing.id.clone()
            }
        )
        .await
        .is_err()
    );
    assert!(
        !repository
            .get_run(&recipient, &missing.id)
            .unwrap()
            .unwrap()
            .status
            .is_terminal()
    );
    assert!(
        !repository
            .updates_after(&recipient, &missing.id, 0, 64)
            .unwrap()
            .iter()
            .any(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
    );
    let effects_after = fixture
        .runtime
        .journal()
        .read_global(1, 10_000)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "effect.started.v1")
        .map(|event| event.global_sequence)
        .collect::<Vec<_>>();
    assert_eq!(effects_after, effects_before);
    let retained = fixture
        .runtime
        .session_messages(&snapshot.session_id)
        .unwrap();
    assert!(
        retained
            .iter()
            .flat_map(|entry| entry.message.content.files())
            .any(|retained| retained == &file)
    );
    let created = fixture
        .runtime
        .journal()
        .read_stream_from(&format!("session:{}", snapshot.session_id), 0, 1)
        .unwrap();
    assert_eq!(created[0].actor, recipient.actor());
    api.set_workspace_sharing(
        &owner,
        SetWorkspaceSharingRequest {
            recipient_application_id: recipient.principal().application_id().into(),
            enabled: false,
            allow_continuation: false,
        },
    )
    .await
    .unwrap();
    branch.idempotency_key = IdempotencyKey::new("fork-revoked").unwrap();
    let before = fixture.runtime.journal().head().unwrap();
    let session_count = fixture.runtime.list_sessions(128).unwrap().len();
    assert_eq!(
        api.create_run(&recipient, branch.clone())
            .await
            .unwrap_err()
            .reason,
        ApiErrorReason::RunNotFound
    );
    assert_eq!(
        fixture.runtime.journal().head().unwrap(),
        before,
        "revoked fork must fail before allocation, copy or effects"
    );
    assert_eq!(
        fixture.runtime.list_sessions(128).unwrap().len(),
        session_count
    );
    assert_eq!(
        artifacts
            .download(&recipient, &artifact_id, 0)
            .await
            .unwrap_err()
            .reason,
        ApiErrorReason::ArtifactNotFound
    );
    // An independently owned snapshot made under an active grant remains a
    // legitimate branch source; revocation does not retract authorized copies.
    branch.idempotency_key = IdempotencyKey::new("fork-retained-snapshot").unwrap();
    branch.branch = Some(RunBranch {
        source_run_id: snapshot.id,
        source_message_count: fixture
            .runtime
            .get_session(&snapshot.session_id)
            .unwrap()
            .unwrap()
            .message_count,
        context_mode: RunBranchContextMode::Exact,
    });
    let independent = api.create_run(&recipient, branch).await.unwrap().run;
    wait_terminal(&api, &recipient, &independent.id).await;
    wait_inactive(&api).await;
    assert!(
        fixture
            .runtime
            .session_messages(&independent.session_id)
            .unwrap()
            .iter()
            .flat_map(|entry| entry.message.content.files())
            .any(|retained| retained == &file)
    );
}
