use super::*;
use colossus_sdk::{ApiErrorCode, ApiOutcomeCertainty};
use futures::StreamExt;

fn lease(generation: u64) -> ConnectionLease {
    ConnectionLease {
        project_id: "project".into(),
        node_id: "node".into(),
        owner_id: format!("replica-{generation}"),
        generation,
        expires_at: crate::http::now() + 30,
        capabilities: vec![colossus_cloud_protocol::TERMINAL_CAPABILITY.into()],
    }
}
fn request(operation: HumanTerminalOperation) -> HumanTerminalRequest {
    HumanTerminalRequest {
        owner: "human.alice".into(),
        operation,
    }
}
async fn admitted(stream: &mut crate::connection::outbound::FrameStream) -> TerminalRequest {
    let frame = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Some(control_frame::Body::TerminalRequest(request)) = frame.body else {
        panic!("terminal request expected");
    };
    request
}

#[tokio::test]
async fn terminal_reply_cannot_cross_connection_generation_and_old_disconnect_preserves_new_owner()
{
    let hub = TerminalHub::default();
    let (old_sender, _old_stream) = crate::connection::outbound::channel();
    hub.connect(lease(1), old_sender).unwrap();
    let (sender, mut stream) = crate::connection::outbound::channel();
    hub.connect(lease(2), sender).unwrap();
    hub.disconnect(&lease(1));
    let active = hub.clone();
    let task = tokio::spawn(async move {
        active
            .request(lease(2), request(HumanTerminalOperation::Context))
            .await
    });
    let frame = admitted(&mut stream).await;
    let reply = TerminalReply::Result {
        reply: HumanTerminalReply {
            available: true,
            incarnation: "worker-incarnation".into(),
            ..Default::default()
        },
    };
    let response = TerminalResponse {
        request_id: frame.request_id,
        reply_json: encode(&reply).unwrap(),
    };
    assert_eq!(
        hub.complete(&lease(1), response.clone())
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    hub.complete(&lease(2), response).unwrap();
    assert_eq!(
        task.await.unwrap().unwrap().incarnation,
        "worker-incarnation"
    );
    assert!(hub.inner.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn disconnected_observation_is_retryable_but_dispatched_input_is_unknown_and_never_replayed()
{
    for (operation, mutation) in [
        (HumanTerminalOperation::Context, false),
        (
            HumanTerminalOperation::Write {
                terminal_id: "terminal".into(),
                writer_epoch: 1,
                input_sequence: 1,
                data_base64: "eA==".into(),
            },
            true,
        ),
    ] {
        let hub = TerminalHub::default();
        let (sender, mut stream) = crate::connection::outbound::channel();
        hub.connect(lease(1), sender).unwrap();
        let active = hub.clone();
        let task = tokio::spawn(async move { active.request(lease(1), request(operation)).await });
        admitted(&mut stream).await;
        hub.disconnect(&lease(1));
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(
            error.outcome,
            if mutation {
                ApiOutcomeCertainty::Unknown
            } else {
                ApiOutcomeCertainty::Known
            }
        );
        assert_eq!(error.retryable, !mutation);
        assert_eq!(
            error.code,
            if mutation {
                ApiErrorCode::OutcomeUnknown
            } else {
                ApiErrorCode::Unavailable
            }
        );
        assert!(hub.inner.pending.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn dropped_browser_cleans_pending_without_closing_native_shell_or_retaining_response() {
    let hub = TerminalHub::default();
    let (sender, mut stream) = crate::connection::outbound::channel();
    hub.connect(lease(1), sender).unwrap();
    let active = hub.clone();
    let task = tokio::spawn(async move {
        active
            .request(lease(1), request(HumanTerminalOperation::Context))
            .await
    });
    let frame = admitted(&mut stream).await;
    task.abort();
    let _ = task.await;
    assert!(hub.inner.pending.lock().unwrap().is_empty());
    assert_eq!(hub.inner.connections.lock().unwrap().len(), 1);
    hub.complete(
        &lease(1),
        TerminalResponse {
            request_id: frame.request_id,
            reply_json: encode(&TerminalReply::Result {
                reply: HumanTerminalReply::default(),
            })
            .unwrap(),
        },
    )
    .unwrap();
    assert!(hub.inner.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn terminal_capacity_is_bounded_per_node_before_dispatch() {
    let hub = TerminalHub::default();
    let (sender, mut stream) = crate::connection::outbound::channel();
    hub.connect(lease(1), sender).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..MAX_TERMINAL_CONTROL_REQUESTS {
        let active = hub.clone();
        tasks.push(tokio::spawn(async move {
            active
                .request(lease(1), request(HumanTerminalOperation::Context))
                .await
        }));
        admitted(&mut stream).await;
    }
    let error = hub
        .request(lease(1), request(HumanTerminalOperation::Context))
        .await
        .unwrap_err();
    assert_eq!(error.code, ApiErrorCode::ResourceExhausted);
    hub.disconnect(&lease(1));
    for task in tasks {
        assert!(task.await.unwrap().is_err());
    }
    assert!(hub.inner.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn eight_quiet_reads_leave_native_interrupt_and_close_admission_available() {
    let hub = TerminalHub::default();
    let (sender, mut stream) = crate::connection::outbound::channel();
    hub.connect(lease(1), sender).unwrap();
    let mut reads = Vec::new();
    let read = || {
        request(HumanTerminalOperation::Read {
            terminal_id: "terminal".into(),
            after_sequence: 0,
            max_bytes: 65536,
            wait_ms: 1000,
        })
    };
    for _ in 0..MAX_TERMINAL_READ_REQUESTS {
        let active = hub.clone();
        let input = read();
        reads.push(tokio::spawn(async move {
            active.request(lease(1), input).await
        }));
        admitted(&mut stream).await;
    }
    assert_eq!(
        hub.request(lease(1), read()).await.unwrap_err().code,
        ApiErrorCode::ResourceExhausted
    );
    for operation in [
        HumanTerminalOperation::Signal {
            terminal_id: "terminal".into(),
            writer_epoch: 1,
            signal: colossus_sdk::HumanTerminalSignal::Interrupt,
        },
        HumanTerminalOperation::Close {
            terminal_id: "terminal".into(),
            writer_epoch: 1,
        },
    ] {
        let active = hub.clone();
        let control =
            tokio::spawn(async move { active.request(lease(1), request(operation)).await });
        let input = admitted(&mut stream).await;
        hub.complete(
            &lease(1),
            TerminalResponse {
                request_id: input.request_id,
                reply_json: encode(&TerminalReply::Result {
                    reply: HumanTerminalReply::default(),
                })
                .unwrap(),
            },
        )
        .unwrap();
        assert!(control.await.unwrap().is_ok());
        assert_eq!(
            hub.inner.pending.lock().unwrap().len(),
            MAX_TERMINAL_READ_REQUESTS
        );
    }
    hub.disconnect(&lease(1));
    for read in reads {
        assert!(read.await.unwrap().is_err());
    }
}
