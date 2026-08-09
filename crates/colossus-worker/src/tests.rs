use super::*;

#[cfg(windows)]
#[test]
fn windows_pipe_saturation_is_classified_as_busy() {
    let error = std::io::Error::from_raw_os_error(231);
    assert!(platform::connection_is_busy(&error));
}

#[cfg(unix)]
#[test]
fn only_an_accepting_endpoint_reports_an_incompatible_worker() {
    let directory = tempfile::tempdir().expect("directory");
    let endpoint = directory.path().join("worker.sock");
    let path = endpoint.to_str().expect("endpoint path");

    assert!(!platform::endpoint_is_live(path));
    assert!(missing_secret_outcome(path).is_ok());

    let listener = std::os::unix::net::UnixListener::bind(&endpoint).expect("listener");
    assert!(platform::endpoint_is_live(path));
    assert!(matches!(
        missing_secret_outcome(path),
        Err(WorkerError::Incompatible(reported)) if reported == path
    ));

    // A killed worker leaves its socket file behind while accepting nothing.
    drop(listener);
    assert!(endpoint.exists());
    assert!(!platform::endpoint_is_live(path));
    assert!(missing_secret_outcome(path).is_ok());
}

#[test]
fn delayed_authenticated_handshake_is_never_worker_absence() {
    assert!(matches!(
        handshake_timeout_error("worker-endpoint"),
        WorkerError::Busy(endpoint) if endpoint == "worker-endpoint"
    ));
}

#[test]
fn only_explicit_transport_absence_allows_worker_fallback() {
    assert!(platform::connection_is_absent(&std::io::Error::from(
        std::io::ErrorKind::NotFound
    )));
    assert!(!platform::connection_is_absent(&std::io::Error::from(
        std::io::ErrorKind::PermissionDenied
    )));
}

#[test]
fn artifact_operations_round_trip_without_artifact_bytes_or_credentials() {
    let encoded = serde_json::to_value(WorkerOperation::ArtifactUpload {
        path: "docs/review.md".into(),
        purpose: ArtifactPurpose::RunInput,
        idempotency_key: "upload-review".into(),
    })
    .expect("serialize artifact upload");
    assert_eq!(encoded["operation"], "artifact_upload");
    assert_eq!(encoded["purpose"], "run_input");
    assert!(encoded.get("bytes").is_none());
    assert!(encoded.get("credential").is_none());

    let decoded: WorkerOperation =
        serde_json::from_value(encoded).expect("deserialize artifact upload");
    assert!(matches!(
        decoded,
        WorkerOperation::ArtifactUpload {
            purpose: ArtifactPurpose::RunInput,
            ..
        }
    ));
}

#[test]
fn approval_mode_control_is_an_explicit_authenticated_operation() {
    let operation = WorkerOperation::SetApprovalMode {
        approval_mode: WorkerApprovalMode::FullAccess,
    };
    let encoded = serde_json::to_value(&operation).expect("serialize approval mode");
    assert_eq!(encoded["operation"], "set_approval_mode");
    assert_eq!(encoded["approval_mode"], "full_access");
    assert_eq!(operation_name(&operation), "set_approval_mode");
    assert!(matches!(
        serde_json::from_value::<WorkerOperation>(encoded).expect("deserialize approval mode"),
        WorkerOperation::SetApprovalMode {
            approval_mode: WorkerApprovalMode::FullAccess
        }
    ));
}

#[test]
fn sandbox_boundary_acknowledgement_is_session_and_mode_bound() {
    let encoded = serde_json::to_value(WorkerOperation::RunInteractive {
        request: InteractiveWorkerRequest::SandboxBoundaryAcknowledge {
            session_id: "session-1".into(),
            mode: SandboxBoundaryMode::External,
        },
        approval_mode: None,
        sandbox_boundary_acknowledgement: None,
    })
    .expect("serialize sandbox boundary acknowledgement");
    assert_eq!(encoded["operation"], "run_interactive");
    assert_eq!(encoded["request"]["kind"], "sandbox_boundary_acknowledge");
    assert_eq!(encoded["request"]["session_id"], "session-1");
    assert_eq!(encoded["request"]["mode"], "external");
    let decoded: WorkerOperation =
        serde_json::from_value(encoded).expect("deserialize sandbox boundary acknowledgement");
    assert!(matches!(
        decoded,
        WorkerOperation::RunInteractive {
            request: InteractiveWorkerRequest::SandboxBoundaryAcknowledge {
                session_id,
                mode: SandboxBoundaryMode::External,
            },
            approval_mode: None,
            sandbox_boundary_acknowledgement: None,
        } if session_id == "session-1"
    ));
    assert!(
        serde_json::from_value::<WorkerOperation>(json!({
            "operation": "sandbox_boundary_acknowledge",
            "session_id": "session-1",
            "mode": "external",
        }))
        .is_err()
    );
    assert_eq!(
        operation_name(&WorkerOperation::SandboxBoundaryStatus {
            session_id: "session-1".into(),
        }),
        "sandbox_boundary_status"
    );
}

#[test]
fn mcp_oauth_worker_operations_carry_only_server_and_callback_metadata() {
    let encoded = serde_json::to_value(WorkerOperation::McpAuthComplete {
        server: "splunk".into(),
        callback_url: "http://127.0.0.1:8787/callback?code=code&state=state".into(),
    })
    .expect("serialize MCP OAuth completion");
    assert_eq!(encoded["operation"], "mcp_auth_complete");
    assert_eq!(
        operation_name(&WorkerOperation::McpAuthStatus {
            server: "splunk".into(),
        }),
        "mcp_auth_status"
    );
    assert!(encoded.get("access_token").is_none());
    assert!(encoded.get("refresh_token").is_none());
    let decoded: WorkerOperation =
        serde_json::from_value(encoded).expect("deserialize MCP OAuth completion");
    assert!(matches!(
        decoded,
        WorkerOperation::McpAuthComplete { server, .. } if server == "splunk"
    ));
}

#[test]
fn model_attachment_protocol_carries_paths_without_client_read_content() {
    let operation = WorkerOperation::RunModel {
        role: "primary".into(),
        instructions: "Review safely".into(),
        prompt: "Inspect the attachment".into(),
        attachments: vec!["docs/review.md".into()],
        max_turns: Some(2),
        session_id: None,
        explicit_skills: Vec::new(),
        sticky_skills: Vec::new(),
    };
    let encoded = serde_json::to_value(&operation).expect("serialize model attachment");
    assert_eq!(
        encoded["attachments"],
        serde_json::json!(["docs/review.md"])
    );
    assert!(
        !encoded["prompt"]
            .as_str()
            .expect("prompt")
            .contains("private")
    );

    let mut preview_payload = encoded;
    preview_payload
        .as_object_mut()
        .expect("operation object")
        .remove("attachments");
    let decoded: WorkerOperation =
        serde_json::from_value(preview_payload).expect("read preview-era operation");
    assert!(matches!(
        decoded,
        WorkerOperation::RunModel { attachments, .. } if attachments.is_empty()
    ));
}

#[test]
fn interactive_run_diagnostics_are_explicit_and_backward_compatible() {
    const CAPABILITY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let operation = WorkerOperation::RunInteractive {
        request: InteractiveWorkerRequest::Run {
            mode: AgentRunMode::Execute,
            role: "primary".into(),
            instructions: "test".into(),
            prompt: "reproduce".into(),
            max_turns: None,
            session_id: "session".into(),
            explicit_skills: Vec::new(),
            sticky_skills: Vec::new(),
            include_provider_response_diagnostics: true,
        },
        approval_mode: Some(WorkerApprovalMode::RiskAuto),
        sandbox_boundary_acknowledgement: Some(
            SandboxBoundaryAcknowledgement::new(CAPABILITY.into()).expect("capability"),
        ),
    };
    assert!(!format!("{operation:?}").contains(CAPABILITY));
    let encoded = serde_json::to_value(&operation).expect("serialize interactive run");
    assert_eq!(
        encoded["request"]["include_provider_response_diagnostics"],
        true
    );
    assert_eq!(encoded["approval_mode"], "risk_auto");

    let mut prior = encoded;
    prior["request"]
        .as_object_mut()
        .expect("interactive request object")
        .remove("include_provider_response_diagnostics");
    let decoded: WorkerOperation =
        serde_json::from_value(prior).expect("deserialize interactive run without diagnostics");
    assert!(matches!(
        decoded,
        WorkerOperation::RunInteractive {
            request: InteractiveWorkerRequest::Run {
                include_provider_response_diagnostics: false,
                ..
            },
            approval_mode: Some(WorkerApprovalMode::RiskAuto),
            sandbox_boundary_acknowledgement: Some(acknowledgement),
        } if acknowledgement.expose() == CAPABILITY
    ));
    assert!(
        serde_json::from_value::<SandboxBoundaryAcknowledgement>(json!("low-entropy")).is_err()
    );
}

#[test]
fn interactive_plan_run_binds_the_selected_revision() {
    let encoded = serde_json::to_value(WorkerOperation::RunInteractive {
        request: InteractiveWorkerRequest::Run {
            mode: AgentRunMode::Plan(colossus_contracts::PlanDraftTarget::Update {
                plan_id: "plan".into(),
                revision: 7,
            }),
            role: "primary".into(),
            instructions: "plan safely".into(),
            prompt: "refine the plan".into(),
            max_turns: None,
            session_id: "session".into(),
            explicit_skills: Vec::new(),
            sticky_skills: Vec::new(),
            include_provider_response_diagnostics: false,
        },
        approval_mode: None,
        sandbox_boundary_acknowledgement: None,
    })
    .expect("serialize interactive Plan Mode run");
    assert_eq!(encoded["request"]["mode"]["mode"], "plan");
    assert_eq!(encoded["request"]["mode"]["target"]["operation"], "update");
    assert_eq!(encoded["request"]["mode"]["target"]["plan_id"], "plan");
    assert_eq!(encoded["request"]["mode"]["target"]["revision"], 7);
    serde_json::from_value::<WorkerOperation>(encoded)
        .expect("deserialize interactive Plan Mode run");
}

#[test]
fn interactive_plan_lifecycle_requests_round_trip_strictly() {
    for request in [
        InteractiveWorkerRequest::PlanApprove {
            session_id: "session".into(),
            plan_id: "plan".into(),
            revision: 2,
        },
        InteractiveWorkerRequest::PlanDiscard {
            session_id: "session".into(),
            plan_id: "plan".into(),
            revision: 2,
        },
        InteractiveWorkerRequest::PlanExecute {
            role: "primary".into(),
            session_id: "session".into(),
            plan_id: "plan".into(),
            revision: 2,
            strategy: PlanExecutionStrategy::Direct,
            max_turns: Some(4),
        },
        InteractiveWorkerRequest::GoalResume {
            role: "primary".into(),
            session_id: "session".into(),
            goal_id: "goal".into(),
        },
    ] {
        let encoded = serde_json::to_value(WorkerOperation::RunInteractive {
            request: request.clone(),
            approval_mode: None,
            sandbox_boundary_acknowledgement: None,
        })
        .expect("serialize interactive request");
        assert_eq!(encoded["operation"], "run_interactive");
        let decoded: WorkerOperation =
            serde_json::from_value(encoded).expect("deserialize interactive request");
        assert!(matches!(decoded, WorkerOperation::RunInteractive { .. }));
    }

    let invalid = json!({
        "operation": "run_interactive",
        "request": {
            "kind": "plan_approve",
            "plan_id": "plan",
            "revision": 2,
            "unexpected": true
        }
    });
    assert!(serde_json::from_value::<WorkerOperation>(invalid).is_err());

    assert_eq!(
        operation_name(&WorkerOperation::RunInteractive {
            request: InteractiveWorkerRequest::PlanExecute {
                role: "primary".into(),
                session_id: "session".into(),
                plan_id: "plan".into(),
                revision: 2,
                strategy: PlanExecutionStrategy::Direct,
                max_turns: None,
            },
            approval_mode: None,
            sandbox_boundary_acknowledgement: None,
        }),
        "run_interactive.plan_execute"
    );
}

#[test]
fn workflow_schedule_operation_round_trips_the_worker_contract() {
    let encoded = serde_json::to_value(WorkerOperation::WorkflowScheduleCreate {
        schedule_id: "nightly".into(),
        name: "smoke".into(),
        version: "1.0.0".into(),
        inputs_source: r#"{"message":"scheduled"}"#.into(),
        cadence_seconds: 3_600,
        misfire_policy: WorkflowScheduleMisfirePolicy::Skip,
        enabled: false,
        starts_at: Some("2026-01-01T12:00:00Z".into()),
    })
    .expect("serialize schedule operation");
    assert_eq!(encoded["operation"], "workflow_schedule_create");
    assert_eq!(encoded["misfire_policy"], "skip");
    let decoded: WorkerOperation =
        serde_json::from_value(encoded).expect("deserialize schedule operation");
    let WorkerOperation::WorkflowScheduleCreate {
        schedule_id,
        cadence_seconds,
        misfire_policy,
        enabled,
        starts_at,
        ..
    } = decoded
    else {
        panic!("expected schedule creation operation");
    };
    assert_eq!(schedule_id, "nightly");
    assert_eq!(cadence_seconds, 3_600);
    assert_eq!(misfire_policy, WorkflowScheduleMisfirePolicy::Skip);
    assert!(!enabled);
    assert_eq!(starts_at.as_deref(), Some("2026-01-01T12:00:00Z"));
}

#[test]
fn durable_queue_operations_wake_the_background_drain() {
    for operation in [
        WorkerOperation::AgentQueue {
            session_id: "session".into(),
            task: "task".into(),
            role: "primary".into(),
        },
        WorkerOperation::AgentRequeue {
            job_id: "job".into(),
        },
        WorkerOperation::WorkflowStart {
            name: "workflow".into(),
            version: "1.0.0".into(),
            inputs_source: "{}".into(),
            queued: true,
        },
    ] {
        assert!(server::operation_requests_drain(&operation));
    }

    assert!(!server::operation_requests_drain(
        &WorkerOperation::WorkflowStart {
            name: "workflow".into(),
            version: "1.0.0".into(),
            inputs_source: "{}".into(),
            queued: false,
        }
    ));
    assert!(!server::operation_requests_drain(&WorkerOperation::Ping));
}

#[test]
fn workflow_webhook_operation_round_trips_without_secret_material() {
    let encoded = serde_json::to_value(WorkerOperation::WorkflowWebhookCreate {
        webhook_id: "github-main".into(),
        name: "smoke".into(),
        version: "1.0.0".into(),
        secret_reference: "env:COLOSSUS_WEBHOOK_SECRET".into(),
        replay_window_seconds: 300,
        max_body_bytes: 4096,
        enabled: true,
    })
    .expect("serialize webhook operation");
    assert_eq!(encoded["operation"], "workflow_webhook_create");
    assert_eq!(encoded["secret_reference"], "env:COLOSSUS_WEBHOOK_SECRET");
    assert!(encoded.to_string().find("actual-secret-value").is_none());
    let decoded: WorkerOperation =
        serde_json::from_value(encoded).expect("deserialize webhook operation");
    let WorkerOperation::WorkflowWebhookCreate {
        webhook_id,
        replay_window_seconds,
        max_body_bytes,
        enabled,
        ..
    } = decoded
    else {
        panic!("expected webhook creation operation");
    };
    assert_eq!(webhook_id, "github-main");
    assert_eq!(replay_window_seconds, 300);
    assert_eq!(max_body_bytes, 4096);
    assert!(enabled);
}

#[test]
fn workflow_subscription_operation_round_trips_the_worker_contract() {
    let encoded = serde_json::to_value(WorkerOperation::WorkflowSubscriptionCreate {
        subscription_id: "new-tasks".into(),
        name: "subscription-smoke".into(),
        version: "1.0.0".into(),
        event_type: "task.created.v1".into(),
        stream_prefix: Some("task:".into()),
        enabled: true,
        after_sequence: Some(41),
    })
    .expect("serialize subscription operation");
    assert_eq!(encoded["operation"], "workflow_subscription_create");
    let decoded: WorkerOperation =
        serde_json::from_value(encoded).expect("deserialize subscription operation");
    let WorkerOperation::WorkflowSubscriptionCreate {
        subscription_id,
        event_type,
        stream_prefix,
        enabled,
        after_sequence,
        ..
    } = decoded
    else {
        panic!("expected subscription creation operation");
    };
    assert_eq!(subscription_id, "new-tasks");
    assert_eq!(event_type, "task.created.v1");
    assert_eq!(stream_prefix.as_deref(), Some("task:"));
    assert!(enabled);
    assert_eq!(after_sequence, Some(41));
}

#[test]
fn authenticated_frames_cover_exact_serialized_payload_bytes() {
    let key = [6_u8; 32];
    let content = WorkerFrameContent::Complete {
        result: json!({
            "z": {"two": 2, "one": 1},
            "a": [true, null, "value"],
        }),
    };
    let timestamp_ms = now_ms();
    let content_base64 = BASE64.encode(serde_json::to_vec(&content).expect("content JSON"));
    let authentication_tag = request_tag(
        &key,
        &UnsignedFrame {
            version: PROTOCOL_VERSION,
            request_id: "canonical-frame",
            sequence: 1,
            timestamp_ms,
            content_base64: &content_base64,
        },
    )
    .expect("tag");
    let encoded = serde_json::to_vec(&WorkerFrame {
        version: PROTOCOL_VERSION,
        request_id: "canonical-frame".into(),
        sequence: 1,
        timestamp_ms,
        content_base64,
        authentication_tag,
    })
    .expect("frame JSON");
    let decoded: WorkerFrame = serde_json::from_slice(&encoded).expect("decoded frame");
    let mut sequence = 0;
    let decoded = validate_frame(&key, "canonical-frame", &mut sequence, &decoded).expect("frame");
    assert!(matches!(decoded, WorkerFrameContent::Complete { .. }));
    assert_eq!(sequence, 1);
}

#[test]
fn worker_frames_round_trip_both_approval_review_notice_kinds() {
    for notice in [
        ApprovalReviewNotice::AutomaticApproval {
            notice: AutomaticApprovalNotice {
                action: "web.search".into(),
                resource: "configured search provider".into(),
                risk_level: colossus_contracts::RiskLevel::Low,
                reason: "read-only configured search".into(),
            },
        },
        ApprovalReviewNotice::RiskReviewFallback {
            notice: RiskReviewFallbackNotice {
                action: "web.search".into(),
                resource: "configured search provider".into(),
                failure: colossus_contracts::RiskReviewFailure::InvalidAssessment,
                reason: "manual approval is required".into(),
            },
        },
    ] {
        let encoded = serde_json::to_vec(&WorkerFrameContent::Notice {
            notice: notice.clone(),
        })
        .expect("notice frame");
        let decoded: WorkerFrameContent =
            serde_json::from_slice(&encoded).expect("decode notice frame");
        assert!(matches!(
            decoded,
            WorkerFrameContent::Notice {
                notice: decoded_notice
            } if decoded_notice == notice
        ));
    }
}

#[test]
fn authentication_detects_tampering_and_replay() {
    let key = [7_u8; 32];
    let mut request =
        signed_request(&key, WorkerOperation::Ping, "connection-one").expect("request");
    request.operation = WorkerOperation::Echo {
        message: "tampered".into(),
    };
    let replay = Mutex::new(ReplayGuard::default());
    assert!(matches!(
        validate_request(&key, &request, &replay, "connection-one"),
        Err(WorkerError::Protocol(_))
    ));

    let request = signed_request(&key, WorkerOperation::Ping, "connection-two").expect("request");
    validate_request(&key, &request, &replay, "connection-two").expect("first request");
    assert!(matches!(
        validate_request(&key, &request, &replay, "connection-two"),
        Err(WorkerError::Protocol(message)) if message.contains("replayed")
    ));
}

fn signed_client_frame(
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    sequence: u64,
    content: ClientFrameContent,
) -> WorkerClientFrame {
    let timestamp_ms = now_ms();
    let content_base64 = BASE64.encode(serde_json::to_vec(&content).expect("content"));
    let authentication_tag = request_tag(
        key,
        &UnsignedClientFrame {
            version: PROTOCOL_VERSION,
            request_id,
            connection_nonce,
            sequence,
            timestamp_ms,
            content_base64: &content_base64,
        },
    )
    .expect("tag");
    WorkerClientFrame {
        version: PROTOCOL_VERSION,
        request_id: request_id.into(),
        connection_nonce: connection_nonce.into(),
        sequence,
        timestamp_ms,
        content_base64,
        authentication_tag,
    }
}

#[test]
fn client_frames_reject_wrong_connection_request_and_replay() {
    let key = [11_u8; 32];
    let frame = signed_client_frame(
        &key,
        "request-one",
        "connection-one",
        1,
        ClientFrameContent::Cancel,
    );
    let mut sequence = 0;
    assert!(matches!(
        validate_client_frame(
            &key,
            "request-one",
            "wrong-connection",
            &mut sequence,
            &frame,
        ),
        Err(WorkerError::Protocol(_))
    ));
    assert!(matches!(
        validate_client_frame(
            &key,
            "wrong-request",
            "connection-one",
            &mut sequence,
            &frame,
        ),
        Err(WorkerError::Protocol(_))
    ));
    validate_client_frame(&key, "request-one", "connection-one", &mut sequence, &frame)
        .expect("first frame");
    assert!(matches!(
        validate_client_frame(&key, "request-one", "connection-one", &mut sequence, &frame,),
        Err(WorkerError::Protocol(_))
    ));
}

#[tokio::test]
async fn prompt_ids_are_one_use_and_unknown_ids_fail_closed() {
    let (outbound_tx, _outbound_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge::new(outbound_tx.clone(), None);
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    bridge
        .responses
        .lock()
        .await
        .pending
        .insert("prompt-one".into(), response_tx);
    bridge
        .respond("prompt-one", Some("answer".into()))
        .await
        .expect("first response");
    assert_eq!(
        response_rx.await.expect("answer").as_deref(),
        Some("answer")
    );
    assert!(matches!(
        bridge.respond("prompt-one", None).await,
        Err(WorkerError::Protocol(message)) if message.contains("replayed")
    ));
    assert!(matches!(
        bridge.respond("wrong-prompt", None).await,
        Err(WorkerError::Protocol(_))
    ));
}

fn test_prompt(id: &str, kind: WorkerPromptKind) -> WorkerPrompt {
    WorkerPrompt {
        prompt_id: id.into(),
        kind,
        title: "Test prompt".into(),
        question: "Continue?".into(),
        choices: vec!["Allow once".into(), "Deny".into()],
        allow_free_form: false,
        details: Value::Null,
    }
}

async fn receive_test_prompt(
    outbound: &mut tokio::sync::mpsc::Receiver<WorkerFrameContent>,
) -> WorkerPrompt {
    match outbound.recv().await.expect("worker outbound frame") {
        WorkerFrameContent::Prompt { prompt } => prompt,
        other => panic!("expected prompt, received {other:?}"),
    }
}

#[tokio::test]
async fn prompt_bridge_covers_answer_cancel_disconnect_timeout_and_run_cancel() {
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(4);
    let bridge = InteractiveRunBridge::new(outbound_tx, None);

    let answered_bridge = bridge.clone();
    let answered = tokio::spawn(async move {
        answered_bridge
            .request(test_prompt("answered", WorkerPromptKind::UserInput))
            .await
    });
    let prompt = receive_test_prompt(&mut outbound_rx).await;
    bridge
        .respond(&prompt.prompt_id, Some("Allow once".into()))
        .await
        .expect("answer prompt");
    assert_eq!(
        answered.await.expect("answered task").expect("answer"),
        Some("Allow once".into())
    );

    let timeout_bridge = bridge.clone();
    let timed_out = tokio::spawn(async move {
        timeout_bridge
            .request_with_timeout(
                test_prompt("timeout", WorkerPromptKind::UserInput),
                Duration::from_millis(1),
            )
            .await
    });
    receive_test_prompt(&mut outbound_rx).await;
    assert!(matches!(
        timed_out.await.expect("timeout task"),
        Err(message) if message.contains("timed out")
    ));

    let cancelled_bridge = bridge.clone();
    let cancelled = tokio::spawn(async move {
        cancelled_bridge
            .request(test_prompt("cancelled", WorkerPromptKind::UserInput))
            .await
    });
    receive_test_prompt(&mut outbound_rx).await;
    let control = RunControl::default();
    bridge.cancel_run(&control).await;
    assert!(control.is_cancelled());
    assert_eq!(cancelled.await.expect("cancel task").expect("cancel"), None);
    assert_eq!(
        bridge
            .request(test_prompt("after-cancel", WorkerPromptKind::Approval))
            .await
            .expect("post-cancel request"),
        None
    );
    assert!(matches!(
        outbound_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));

    let (disconnected_tx, disconnected_rx) = tokio::sync::mpsc::channel(1);
    drop(disconnected_rx);
    let disconnected = InteractiveRunBridge::new(disconnected_tx, None);
    assert!(matches!(
        disconnected
            .request(test_prompt("disconnect", WorkerPromptKind::Approval))
            .await,
        Err(message) if message.contains("disconnected")
    ));
}

struct SocketTestObserver {
    order: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunEventObserver for SocketTestObserver {
    async fn observe(&mut self, _event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.order.lock().expect("order lock").push("event");
        Ok(())
    }
}

struct SilentSocketObserver;

#[async_trait]
impl RunEventObserver for SilentSocketObserver {
    async fn observe(&mut self, _event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        Ok(())
    }
}

struct SocketTestPromptHandler {
    order: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl WorkerPromptHandler for SocketTestPromptHandler {
    async fn notice(&self, _notice: ApprovalReviewNotice) -> Result<(), WorkerError> {
        self.order.lock().expect("order lock").push("notice");
        Ok(())
    }

    async fn prompt(&self, _prompt: WorkerPrompt) -> Result<Option<String>, WorkerError> {
        self.order.lock().expect("order lock").push("prompt");
        Ok(Some("Allow once".into()))
    }
}

struct PromptDropSignal(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for PromptDropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

struct BlockingSocketPromptHandler {
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    dropped: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl WorkerPromptHandler for BlockingSocketPromptHandler {
    async fn prompt(&self, _prompt: WorkerPrompt) -> Result<Option<String>, WorkerError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let _drop_signal = PromptDropSignal(self.dropped.lock().expect("drop signal lock").take());
        if let Some(started) = self.started.lock().expect("start signal lock").take() {
            let _ = started.send(());
        }
        std::future::pending().await
    }
}

struct FailingSocketPromptHandler;

#[async_trait]
impl WorkerPromptHandler for FailingSocketPromptHandler {
    async fn prompt(&self, _prompt: WorkerPrompt) -> Result<Option<String>, WorkerError> {
        Err(WorkerError::Unavailable(
            "test interactive client disconnected".into(),
        ))
    }
}

fn socket_test_event() -> RunEventEnvelope {
    RunEventEnvelope {
        schema_version: 1,
        run_id: "run".into(),
        session_id: "session".into(),
        event: colossus_contracts::RunEvent::Phase {
            phase: colossus_contracts::RunPhase::Preparing,
            turn: None,
            action: Some("test".into()),
            elapsed_seconds: 0.0,
        },
    }
}

fn socket_test_notice() -> ApprovalReviewNotice {
    ApprovalReviewNotice::AutomaticApproval {
        notice: AutomaticApprovalNotice {
            action: "filesystem.read".into(),
            resource: "test".into(),
            risk_level: colossus_contracts::RiskLevel::Low,
            reason: "test".into(),
        },
    }
}

const SOCKET_TEST_REQUEST_ID: &str = "interactive-request";
const SOCKET_TEST_CONNECTION_NONCE: &str = "interactive-connection";

#[tokio::test]
async fn interactive_socket_preserves_event_notice_prompt_order() {
    let key = [21_u8; 32];
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel(8);
        let bridge = InteractiveRunBridge::new(outbound_tx.clone(), None);
        let run_bridge = bridge.clone();
        let control = RunControl::default();
        let run = async move {
            outbound_tx
                .send(WorkerFrameContent::Event {
                    event: socket_test_event(),
                })
                .await
                .map_err(|_| WorkerError::Protocol("ordered event channel closed".into()))?;
            outbound_tx
                .send(WorkerFrameContent::Notice {
                    notice: socket_test_notice(),
                })
                .await
                .map_err(|_| WorkerError::Protocol("ordered notice channel closed".into()))?;
            let answer = run_bridge
                .request(test_prompt("ordered", WorkerPromptKind::Approval))
                .await
                .map_err(WorkerError::Protocol)?;
            Ok(json!({ "answer": answer }))
        };
        server::drive_interactive_connection(
            server_stream,
            server::InteractiveConnectionContext {
                key: &key,
                request_id: SOCKET_TEST_REQUEST_ID,
                connection_nonce: SOCKET_TEST_CONNECTION_NONCE,
            },
            outbound_rx,
            bridge,
            &control,
            run,
        )
        .await
    });

    let order = Arc::new(Mutex::new(Vec::new()));
    let mut observer = SocketTestObserver {
        order: Arc::clone(&order),
    };
    let prompts = SocketTestPromptHandler {
        order: Arc::clone(&order),
    };
    let result = client::drive_interactive_client::<_, Value>(
        client_stream,
        &key,
        SOCKET_TEST_REQUEST_ID,
        SOCKET_TEST_CONNECTION_NONCE,
        &mut observer,
        &prompts,
        &RunControl::default(),
    )
    .await
    .expect("interactive result");
    assert_eq!(result["answer"], "Allow once");
    assert_eq!(
        *order.lock().expect("order lock"),
        vec!["event", "notice", "prompt"]
    );
    server
        .await
        .expect("server task")
        .expect("server connection");
}

#[tokio::test]
async fn interactive_socket_cancel_releases_waiter_and_rejects_late_prompt() {
    let key = [22_u8; 32];
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel(8);
        let bridge = InteractiveRunBridge::new(outbound_tx, None);
        let run_bridge = bridge.clone();
        let control = RunControl::default();
        let run_control = control.clone();
        let run = async move {
            let first = run_bridge
                .request(test_prompt("cancelled", WorkerPromptKind::UserInput))
                .await
                .map_err(WorkerError::Protocol)?;
            let second = run_bridge
                .request(test_prompt("after-cancel", WorkerPromptKind::Approval))
                .await
                .map_err(WorkerError::Protocol)?;
            Ok(json!({
                "first": first,
                "second": second,
                "cancelled": run_control.is_cancelled(),
            }))
        };
        server::drive_interactive_connection(
            server_stream,
            server::InteractiveConnectionContext {
                key: &key,
                request_id: SOCKET_TEST_REQUEST_ID,
                connection_nonce: SOCKET_TEST_CONNECTION_NONCE,
            },
            outbound_rx,
            bridge,
            &control,
            run,
        )
        .await
    });

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let prompts = BlockingSocketPromptHandler {
        started: Mutex::new(Some(started_tx)),
        dropped: Mutex::new(Some(dropped_tx)),
        calls: Arc::clone(&calls),
    };
    let control = RunControl::default();
    let client_control = control.clone();
    let client_task = tokio::spawn(async move {
        let mut observer = SilentSocketObserver;
        client::drive_interactive_client::<_, Value>(
            client_stream,
            &key,
            SOCKET_TEST_REQUEST_ID,
            SOCKET_TEST_CONNECTION_NONCE,
            &mut observer,
            &prompts,
            &client_control,
        )
        .await
    });
    started_rx.await.expect("prompt started");
    control.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), client_task)
        .await
        .expect("cancelled client timeout")
        .expect("client task")
        .expect("cancelled interactive result");
    dropped_rx.await.expect("local prompt future dropped");
    assert_eq!(result["first"], Value::Null);
    assert_eq!(result["second"], Value::Null);
    assert_eq!(result["cancelled"], true);
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::Acquire),
        1,
        "the post-cancel prompt must never reach the client"
    );
    server
        .await
        .expect("server task")
        .expect("server connection");
}

#[tokio::test]
async fn interactive_socket_interrupts_open_prompt_on_terminal_frame_or_disconnect() {
    for terminal in [
        Some(WorkerFrameContent::Error {
            message: "terminal failure".into(),
        }),
        None,
    ] {
        let key = [23_u8; 32];
        let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            write_signed_frame(
                &mut server_stream,
                &key,
                SOCKET_TEST_REQUEST_ID,
                1,
                WorkerFrameContent::Prompt {
                    prompt: test_prompt("blocking", WorkerPromptKind::UserInput),
                },
            )
            .await
            .expect("write prompt frame");
            started_rx.await.expect("client opened prompt");
            if let Some(content) = terminal {
                write_signed_frame(&mut server_stream, &key, SOCKET_TEST_REQUEST_ID, 2, content)
                    .await
                    .expect("write terminal frame");
            }
        });

        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let prompts = BlockingSocketPromptHandler {
            started: Mutex::new(Some(started_tx)),
            dropped: Mutex::new(Some(dropped_tx)),
            calls: Arc::clone(&calls),
        };
        let mut observer = SilentSocketObserver;
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            client::drive_interactive_client::<_, Value>(
                client_stream,
                &key,
                SOCKET_TEST_REQUEST_ID,
                SOCKET_TEST_CONNECTION_NONCE,
                &mut observer,
                &prompts,
                &RunControl::default(),
            ),
        )
        .await
        .expect("prompt interruption timeout");
        assert!(result.is_err());
        dropped_rx.await.expect("prompt future dropped");
        assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), 1);
        server.await.expect("server task");
    }
}

#[tokio::test]
async fn interactive_socket_disconnect_cancels_server_and_clears_waiters() {
    let key = [24_u8; 32];
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel(8);
        let bridge = InteractiveRunBridge::new(outbound_tx, None);
        let inspect = bridge.clone();
        let run_bridge = bridge.clone();
        let control = RunControl::default();
        let run = async move {
            run_bridge
                .request(test_prompt("disconnect", WorkerPromptKind::Approval))
                .await
                .map(|answer| json!({ "answer": answer }))
                .map_err(WorkerError::Protocol)
        };
        let result = server::drive_interactive_connection(
            server_stream,
            server::InteractiveConnectionContext {
                key: &key,
                request_id: SOCKET_TEST_REQUEST_ID,
                connection_nonce: SOCKET_TEST_CONNECTION_NONCE,
            },
            outbound_rx,
            bridge,
            &control,
            run,
        )
        .await;
        let responses = inspect.responses.lock().await;
        (result, responses.cancelled, responses.pending.len())
    });

    let mut observer = SilentSocketObserver;
    let result = client::drive_interactive_client::<_, Value>(
        client_stream,
        &key,
        SOCKET_TEST_REQUEST_ID,
        SOCKET_TEST_CONNECTION_NONCE,
        &mut observer,
        &FailingSocketPromptHandler,
        &RunControl::default(),
    )
    .await;
    assert!(matches!(result, Err(WorkerError::Unavailable(_))));
    let (server_result, cancelled, pending) = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("server disconnect timeout")
        .expect("server task");
    assert!(server_result.is_err());
    assert!(cancelled);
    assert_eq!(pending, 0);
}

#[tokio::test]
async fn interactive_client_stale_worker_error_has_restart_guidance() {
    let key = [25_u8; 32];
    let (mut client_stream, mut server_stream) = tokio::io::duplex(1024);
    let server = tokio::spawn(async move {
        let hello: ClientHello = read_message(&mut server_stream, 1024)
            .await
            .expect("client hello");
        write_message(
            &mut server_stream,
            &ServerHello {
                version: PROTOCOL_VERSION - 1,
                challenge: hello.challenge,
                server_nonce: "a".repeat(64),
                timestamp_ms: now_ms(),
                authentication_tag: String::new(),
            },
            1024,
        )
        .await
        .expect("stale server hello");
    });

    let error = client_handshake(&mut client_stream, &key)
        .await
        .expect_err("stale worker must fail");
    assert!(error.to_string().contains("restart the worker"));
    server.await.expect("server task");
}

#[tokio::test]
async fn interactive_worker_approval_accepts_only_the_exact_allow_choice() {
    let mut request = colossus_policy::effect_request(
        colossus_policy::system_actor("worker-test"),
        "filesystem.write",
        "note.txt",
        json!({"content": "bounded"}),
    );
    request.risk.reason = Some("risk-auto skipped because this action is ineligible".into());
    let decision = PolicyDecision {
        decision_id: "decision-test".into(),
        policy_revision: "test-v1".into(),
        outcome: colossus_contracts::DecisionOutcome::RequireApproval,
        reason: "operator must approve".into(),
        obligations: colossus_contracts::PolicyObligations::default(),
    };

    for (answer, expected_approval) in [("Allow once", true), ("Deny", false)] {
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
        let bridge = InteractiveRunBridge::new(outbound_tx, None);
        let responder_bridge = bridge.clone();
        let answer = answer.to_owned();
        let responder = tokio::spawn(async move {
            let prompt = receive_test_prompt(&mut outbound_rx).await;
            assert_eq!(prompt.kind, WorkerPromptKind::Approval);
            assert!(prompt.question.contains("risk-auto skipped"));
            assert_eq!(
                prompt.details["risk"]["reason"],
                "risk-auto skipped because this action is ineligible"
            );
            assert_eq!(prompt.details["actor"]["actor_type"], "system");
            assert_eq!(prompt.details["actor"]["id"], "worker-test");
            assert_eq!(prompt.details["reason"], "operator must approve");
            responder_bridge
                .respond(&prompt.prompt_id, Some(answer))
                .await
                .expect("approval response");
        });
        let provider = WorkerInteractiveApproval::new(WorkerApprovalModeState::new(Some(
            WorkerApprovalMode::Ask,
        )));
        let proof = ACTIVE_INTERACTIVE_RUN
            .scope(
                bridge,
                provider.request_approval(&request, "request-hash", &decision),
            )
            .await
            .expect("approval result");
        responder.await.expect("responder");
        assert_eq!(proof.is_some(), expected_approval);
    }
}

#[tokio::test]
async fn interactive_worker_forwards_automatic_approval_notices_without_prompting() {
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge::new(outbound_tx, None);
    let provider = WorkerInteractiveApproval::new(WorkerApprovalModeState::new(Some(
        WorkerApprovalMode::RiskAuto,
    )));
    let notice = AutomaticApprovalNotice {
        action: "web.search".into(),
        resource: "configured search provider".into(),
        risk_level: colossus_contracts::RiskLevel::Low,
        reason: "read-only configured search".into(),
    };

    ACTIVE_INTERACTIVE_RUN
        .scope(bridge, provider.automatic_approval_granted(notice.clone()))
        .await;

    assert_eq!(
        outbound_rx.recv().await,
        Some(WorkerFrameContent::Notice {
            notice: ApprovalReviewNotice::AutomaticApproval { notice }
        })
    );
}

#[tokio::test]
async fn interactive_worker_uses_the_client_scoped_approval_mode_override() {
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge::new(outbound_tx, Some(WorkerApprovalMode::FullAccess));
    let provider = WorkerInteractiveApproval::new(WorkerApprovalModeState::new(Some(
        WorkerApprovalMode::Deny,
    )));
    let request = colossus_policy::effect_request(
        colossus_policy::system_actor("worker-mode-test"),
        "shell.run",
        "workspace",
        json!({"command": "cargo test"}),
    );
    let decision = PolicyDecision {
        decision_id: "decision-mode-test".into(),
        policy_revision: "test-v1".into(),
        outcome: colossus_contracts::DecisionOutcome::RequireApproval,
        reason: "operator must approve".into(),
        obligations: colossus_contracts::PolicyObligations::default(),
    };

    let proof = ACTIVE_INTERACTIVE_RUN
        .scope(
            bridge,
            provider.request_approval(&request, "request-hash", &decision),
        )
        .await
        .expect("client-scoped full access");
    assert!(proof.is_some());
    assert!(outbound_rx.try_recv().is_err());

    let (outbound_tx, _outbound_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge::new(outbound_tx, Some(WorkerApprovalMode::RiskAuto));
    assert!(
        ACTIVE_INTERACTIVE_RUN
            .scope(bridge, async { provider.risk_auto_enabled() })
            .await
    );
}

#[tokio::test]
async fn worker_wide_approval_mode_changes_apply_without_a_restart() {
    let mode = WorkerApprovalModeState::new(Some(WorkerApprovalMode::Deny));
    assert_eq!(
        colossus_api_runtime::PublicApprovalModeProvider::public_approval_mode(&mode),
        colossus_api_runtime::PublicApprovalMode::Deny
    );
    let provider = WorkerInteractiveApproval::new(mode.clone());
    let request = colossus_policy::effect_request(
        colossus_policy::system_actor("worker-live-mode-test"),
        "shell.run",
        "workspace",
        json!({"command": "cargo test"}),
    );
    let decision = PolicyDecision {
        decision_id: "decision-live-mode-test".into(),
        policy_revision: "test-v1".into(),
        outcome: colossus_contracts::DecisionOutcome::RequireApproval,
        reason: "operator must approve".into(),
        obligations: colossus_contracts::PolicyObligations::default(),
    };

    assert!(
        provider
            .request_approval(&request, "deny-hash", &decision)
            .await
            .expect("deny mode")
            .is_none()
    );
    assert!(mode.set(WorkerApprovalMode::FullAccess));
    assert_eq!(
        colossus_api_runtime::PublicApprovalModeProvider::public_approval_mode(&mode),
        colossus_api_runtime::PublicApprovalMode::FullAccess
    );
    assert!(
        provider
            .request_approval(&request, "allow-hash", &decision)
            .await
            .expect("full access mode")
            .is_some()
    );
}

#[tokio::test]
async fn interactive_worker_forwards_risk_review_fallback_without_prompting() {
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge::new(outbound_tx, None);
    let provider = WorkerInteractiveApproval::new(WorkerApprovalModeState::new(Some(
        WorkerApprovalMode::RiskAuto,
    )));
    let notice = RiskReviewFallbackNotice {
        action: "web.search".into(),
        resource: "configured search provider".into(),
        failure: colossus_contracts::RiskReviewFailure::InvalidAssessment,
        reason:
            "The risk evaluator response failed strict validation, so manual approval is required."
                .into(),
    };

    ACTIVE_INTERACTIVE_RUN
        .scope(bridge, provider.risk_review_fallback(notice.clone()))
        .await;

    assert_eq!(
        outbound_rx.recv().await,
        Some(WorkerFrameContent::Notice {
            notice: ApprovalReviewNotice::RiskReviewFallback { notice }
        })
    );
}

#[tokio::test]
async fn interactive_worker_drops_approval_review_notice_when_queue_is_full() {
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
    let queued = ApprovalReviewNotice::AutomaticApproval {
        notice: AutomaticApprovalNotice {
            action: "web.search".into(),
            resource: "first configured search".into(),
            risk_level: colossus_contracts::RiskLevel::Low,
            reason: "first read-only configured search".into(),
        },
    };
    outbound_tx
        .try_send(WorkerFrameContent::Notice {
            notice: queued.clone(),
        })
        .expect("fill worker notice queue");
    let bridge = InteractiveRunBridge::new(outbound_tx.clone(), None);
    let provider = WorkerInteractiveApproval::new(WorkerApprovalModeState::new(Some(
        WorkerApprovalMode::RiskAuto,
    )));

    tokio::time::timeout(
        Duration::from_millis(100),
        ACTIVE_INTERACTIVE_RUN.scope(
            bridge,
            provider.risk_review_fallback(RiskReviewFallbackNotice {
                action: "web.search".into(),
                resource: "second configured search".into(),
                failure: colossus_contracts::RiskReviewFailure::EvaluatorUnavailable,
                reason: "The risk evaluator was unavailable, so manual approval is required."
                    .into(),
            }),
        ),
    )
    .await
    .expect("a best-effort worker notice must not wait for queue capacity");

    assert_eq!(
        outbound_rx.recv().await,
        Some(WorkerFrameContent::Notice { notice: queued })
    );
    assert!(matches!(
        outbound_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn protocol_version_mismatch_has_restart_guidance() {
    assert_eq!(PROTOCOL_VERSION, 11);
    let key = [13_u8; 32];
    let mut frame =
        signed_client_frame(&key, "request", "connection", 1, ClientFrameContent::Cancel);
    frame.version = PROTOCOL_VERSION - 1;
    let mut sequence = 0;
    assert!(matches!(
        validate_client_frame(&key, "request", "connection", &mut sequence, &frame),
        Err(WorkerError::Protocol(message)) if message.contains("version")
    ));

    let (mut client, mut server) = tokio::io::duplex(1024);
    let writer = tokio::spawn(async move {
        write_message(
            &mut client,
            &ClientHello {
                version: PROTOCOL_VERSION - 1,
                challenge: "a".repeat(64),
            },
            1024,
        )
        .await
    });
    assert!(matches!(
        server_handshake(&mut server, &key).await,
        Err(WorkerError::Protocol(message)) if message.contains("restart the worker")
    ));
    writer.await.expect("hello writer").expect("hello");
}

#[tokio::test]
async fn stale_worker_that_closes_the_handshake_is_reported_as_incompatible() {
    let key = [17_u8; 32];
    let (mut client, mut server) = tokio::io::duplex(1024);
    // A worker from the previous protocol rejects this client's hello and drops
    // the stream before it writes any ServerHello.
    let stale_worker = tokio::spawn(async move {
        let _hello: ClientHello = read_message(&mut server, 1024).await.expect("client hello");
        drop(server);
    });

    let error = client_handshake(&mut client, &key)
        .await
        .expect_err("stale worker must fail the client handshake");
    assert!(matches!(error, WorkerError::Io(_)));
    let outcome = handshake_failure_outcome("worker-endpoint", error);
    assert!(
        matches!(&outcome, WorkerError::Incompatible(endpoint) if endpoint == "worker-endpoint")
    );
    assert!(outcome.to_string().contains("restart the worker"));
    stale_worker.await.expect("stale worker task");
}

#[tokio::test]
async fn client_handshake_protocol_rejection_is_not_reported_as_incompatible() {
    let outcome = handshake_failure_outcome(
        "worker-endpoint",
        WorkerError::Protocol("invalid handshake".into()),
    );
    assert!(matches!(outcome, WorkerError::Protocol(message) if message.contains("invalid")));
}

#[tokio::test]
async fn oversized_client_prompt_response_is_rejected_before_write() {
    let key = [12_u8; 32];
    let (mut writer, _reader) = tokio::io::duplex(64);
    let result = write_signed_client_frame(
        &mut writer,
        &key,
        "request",
        "connection",
        1,
        ClientFrameContent::PromptResponse {
            prompt_id: "prompt".into(),
            answer: Some("x".repeat(MAX_REQUEST_BYTES + 1)),
        },
    )
    .await;
    assert!(matches!(result, Err(WorkerError::Protocol(message)) if message.contains("1 MiB")));
}

#[tokio::test]
async fn framing_rejects_oversized_lengths_before_allocation() {
    let (mut writer, mut reader) = tokio::io::duplex(64);
    let task = tokio::spawn(async move {
        writer
            .write_u32((MAX_REQUEST_BYTES + 1) as u32)
            .await
            .expect("length");
    });
    let result = read_message::<_, WorkerRequest>(&mut reader, MAX_REQUEST_BYTES).await;
    assert!(matches!(result, Err(WorkerError::Protocol(_))));
    task.await.expect("writer");
}

#[tokio::test]
async fn client_discloses_no_operation_to_an_unauthenticated_server() {
    let expected_key = [8_u8; 32];
    let fake_key = [9_u8; 32];
    let (mut client, mut server) = tokio::io::duplex(4_096);
    let fake = tokio::spawn(async move {
        let hello: ClientHello = read_message(&mut server, 1024).await.expect("hello");
        let server_nonce = hex::encode([3_u8; 32]);
        let timestamp_ms = now_ms();
        let authentication_tag = request_tag(
            &fake_key,
            &UnsignedServerHello {
                version: PROTOCOL_VERSION,
                challenge: &hello.challenge,
                server_nonce: &server_nonce,
                timestamp_ms,
            },
        )
        .expect("tag");
        write_message(
            &mut server,
            &ServerHello {
                version: PROTOCOL_VERSION,
                challenge: hello.challenge,
                server_nonce,
                timestamp_ms,
                authentication_tag,
            },
            1024,
        )
        .await
        .expect("server hello");
        read_message::<_, WorkerRequest>(&mut server, MAX_REQUEST_BYTES).await
    });
    assert!(matches!(
        client_handshake(&mut client, &expected_key).await,
        Err(WorkerError::Protocol(_))
    ));
    drop(client);
    assert!(matches!(
        fake.await.expect("fake server"),
        Err(WorkerError::Io(_))
    ));
}
