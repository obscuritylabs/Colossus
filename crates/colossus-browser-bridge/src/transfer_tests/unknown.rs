//! Host repeats the uncertainty fence before any already-queued command is admitted.
use super::*;
use crate::{
    framing::AuthenticatedChannel,
    wire::{ErrorCode, Request, Response},
};
#[tokio::test]
async fn unknown_host_reply_fences_queued_data_without_waiting_for_parent_cancel() {
    let (data, host_data) = channel_pair();
    let (control, host_control) = channel_pair();
    let mut admitted = enrollment();
    admitted
        .capabilities
        .actions
        .push(BrowserActionKind::Upload);
    let native = Arc::new(Native::default());
    native.bad_receipt.store(true, Ordering::SeqCst);
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_control,
        admitted.clone(),
        BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new([7; 32])),
        native.clone(),
    ));
    let key = Arc::new(BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new(
        [7; 32],
    )));
    let mut data = AuthenticatedChannel::new(data, key.clone(), &admitted, b"data").unwrap();
    let mut control = AuthenticatedChannel::new(control, key, &admitted, b"control").unwrap();
    data.write(
        b"request",
        1,
        &Request::Open {
            request: Box::new(open_request()),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        data.read::<Response>(b"response", 1).await.unwrap(),
        Response::Opened { .. }
    ));
    data.write(
        b"request",
        2,
        &Request::BeginUpload {
            request: Box::new(upload()),
        },
    )
    .await
    .unwrap();
    let Response::UploadPrepared { receipt } = data.read::<Response>(b"response", 2).await.unwrap()
    else {
        panic!("prepare");
    };
    data.write(
        b"request",
        3,
        &Request::WriteUpload {
            request: Box::new(write(&receipt.transfer_id, 0, 1)),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        data.read::<Response>(b"response", 3).await.unwrap(),
        Response::Rejected {
            code: ErrorCode::OutcomeUnknown
        }
    ));
    data.write(
        b"request",
        4,
        &Request::Execute {
            command: Box::new(command(1)),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        data.read::<Response>(b"response", 4).await.unwrap(),
        Response::Rejected {
            code: ErrorCode::Stale
        }
    ));
    assert_eq!(native.executions.load(Ordering::SeqCst), 0);
    assert_eq!(
        native.cancels.load(Ordering::SeqCst),
        0,
        "no parent cancel has arrived"
    );
    control
        .write(
            b"request",
            1,
            &Request::Close {
                session_id: open_request().session_id,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        control.read::<Response>(b"response", 1).await.unwrap(),
        Response::Acknowledged {}
    ));
    drop(data);
    drop(control);
    tokio::time::timeout(Duration::from_secs(2), host)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
