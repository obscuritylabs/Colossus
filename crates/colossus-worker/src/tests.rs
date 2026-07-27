use super::*;

#[cfg(windows)]
#[test]
fn windows_pipe_saturation_is_classified_as_busy() {
    let error = std::io::Error::from_raw_os_error(231);
    assert!(platform::connection_is_busy(&error));
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
    let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::channel(1);
    let (notice_tx, _notice_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge {
        prompts: prompt_tx,
        notices: notice_tx,
        responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    bridge
        .responses
        .lock()
        .await
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

#[tokio::test]
async fn prompt_bridge_covers_answer_cancel_disconnect_timeout_and_run_cancel() {
    let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::channel(4);
    let (notice_tx, _notice_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge {
        prompts: prompt_tx,
        notices: notice_tx,
        responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };

    let answered_bridge = bridge.clone();
    let answered = tokio::spawn(async move {
        answered_bridge
            .request(test_prompt("answered", WorkerPromptKind::UserInput))
            .await
    });
    let prompt = prompt_rx.recv().await.expect("answered prompt");
    bridge
        .respond(&prompt.prompt_id, Some("Allow once".into()))
        .await
        .expect("answer prompt");
    assert_eq!(
        answered.await.expect("answered task").expect("answer"),
        Some("Allow once".into())
    );

    let cancelled_bridge = bridge.clone();
    let cancelled = tokio::spawn(async move {
        cancelled_bridge
            .request(test_prompt("cancelled", WorkerPromptKind::UserInput))
            .await
    });
    prompt_rx.recv().await.expect("cancelled prompt");
    bridge.cancel_all().await;
    assert_eq!(cancelled.await.expect("cancel task").expect("cancel"), None);

    let timeout_bridge = bridge.clone();
    let timed_out = tokio::spawn(async move {
        timeout_bridge
            .request_with_timeout(
                test_prompt("timeout", WorkerPromptKind::UserInput),
                Duration::from_millis(1),
            )
            .await
    });
    prompt_rx.recv().await.expect("timeout prompt");
    assert!(matches!(
        timed_out.await.expect("timeout task"),
        Err(message) if message.contains("timed out")
    ));

    let control = RunControl::default();
    control.cancel();
    assert!(control.is_cancelled());

    let (disconnected_tx, disconnected_rx) = tokio::sync::mpsc::channel(1);
    drop(disconnected_rx);
    let (notice_tx, _notice_rx) = tokio::sync::mpsc::channel(1);
    let disconnected = InteractiveRunBridge {
        prompts: disconnected_tx,
        notices: notice_tx,
        responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };
    assert!(matches!(
        disconnected
            .request(test_prompt("disconnect", WorkerPromptKind::Approval))
            .await,
        Err(message) if message.contains("disconnected")
    ));
}

#[tokio::test]
async fn interactive_worker_approval_accepts_only_the_exact_allow_choice() {
    let request = colossus_policy::effect_request(
        colossus_policy::system_actor("worker-test"),
        "filesystem.write",
        "note.txt",
        json!({"content": "bounded"}),
    );
    let decision = PolicyDecision {
        decision_id: "decision-test".into(),
        policy_revision: "test-v1".into(),
        outcome: colossus_contracts::DecisionOutcome::RequireApproval,
        reason: "operator must approve".into(),
        obligations: colossus_contracts::PolicyObligations::default(),
    };

    for (answer, expected_approval) in [("Allow once", true), ("Deny", false)] {
        let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::channel(1);
        let (notice_tx, _notice_rx) = tokio::sync::mpsc::channel(1);
        let bridge = InteractiveRunBridge {
            prompts: prompt_tx,
            notices: notice_tx,
            responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        };
        let responder_bridge = bridge.clone();
        let answer = answer.to_owned();
        let responder = tokio::spawn(async move {
            let prompt = prompt_rx.recv().await.expect("approval prompt");
            assert_eq!(prompt.kind, WorkerPromptKind::Approval);
            responder_bridge
                .respond(&prompt.prompt_id, Some(answer))
                .await
                .expect("approval response");
        });
        let provider = WorkerInteractiveApproval {
            mode: WorkerApprovalMode::Ask,
        };
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
    let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::channel(1);
    let (notice_tx, mut notice_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge {
        prompts: prompt_tx,
        notices: notice_tx,
        responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };
    let provider = WorkerInteractiveApproval {
        mode: WorkerApprovalMode::RiskAuto,
    };
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
        notice_rx.recv().await,
        Some(ApprovalReviewNotice::AutomaticApproval { notice })
    );
}

#[tokio::test]
async fn interactive_worker_forwards_risk_review_fallback_without_prompting() {
    let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::channel(1);
    let (notice_tx, mut notice_rx) = tokio::sync::mpsc::channel(1);
    let bridge = InteractiveRunBridge {
        prompts: prompt_tx,
        notices: notice_tx,
        responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };
    let provider = WorkerInteractiveApproval {
        mode: WorkerApprovalMode::RiskAuto,
    };
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
        notice_rx.recv().await,
        Some(ApprovalReviewNotice::RiskReviewFallback { notice })
    );
}

#[tokio::test]
async fn interactive_worker_drops_approval_review_notice_when_queue_is_full() {
    let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::channel(1);
    let (notice_tx, mut notice_rx) = tokio::sync::mpsc::channel(1);
    let queued = ApprovalReviewNotice::AutomaticApproval {
        notice: AutomaticApprovalNotice {
            action: "web.search".into(),
            resource: "first configured search".into(),
            risk_level: colossus_contracts::RiskLevel::Low,
            reason: "first read-only configured search".into(),
        },
    };
    notice_tx
        .try_send(queued.clone())
        .expect("fill worker notice queue");
    let bridge = InteractiveRunBridge {
        prompts: prompt_tx,
        notices: notice_tx.clone(),
        responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };
    let provider = WorkerInteractiveApproval {
        mode: WorkerApprovalMode::RiskAuto,
    };

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

    assert_eq!(notice_rx.recv().await, Some(queued));
    assert!(matches!(
        notice_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn protocol_version_mismatch_has_restart_guidance() {
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
