use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use colossus_ports::{BrowserDriverControl, BrowserDriverError};
use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};

use crate::{
    framing::AuthenticatedChannel,
    wire::{Request, Response},
};

pub(crate) struct Dispatch {
    pub(crate) request: Request,
    pub(crate) control: Option<BrowserDriverControl>,
    pub(crate) deadline: Duration,
    pub(crate) response: oneshot::Sender<Result<Response, BrowserDriverError>>,
}

pub(crate) fn spawn_channel(
    mut channel: AuthenticatedChannel,
    available: Arc<AtomicBool>,
    capacity: usize,
) -> (mpsc::Sender<Dispatch>, tokio::task::AbortHandle) {
    let (sender, mut receiver) = mpsc::channel::<Dispatch>(capacity);
    let owner = tokio::spawn(async move {
        let mut sequence = 0_u64;
        while let Some(dispatch) = receiver.recv().await {
            let result = if dispatch
                .control
                .as_ref()
                .is_some_and(BrowserDriverControl::is_cancelled)
            {
                Err(BrowserDriverError::Cancelled)
            } else if let Some(next) = sequence.checked_add(1) {
                sequence = next;
                timeout(dispatch.deadline, async {
                    channel
                        .write(b"request", sequence, &dispatch.request)
                        .await
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    channel
                        .read::<Response>(b"response", sequence)
                        .await
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)
                })
                .await
                .unwrap_or(Err(BrowserDriverError::OutcomeUnknown))
            } else {
                Err(BrowserDriverError::OutcomeUnknown)
            };
            let broken = matches!(&result, Err(error) if *error != BrowserDriverError::Cancelled);
            let _ = dispatch.response.send(result);
            if broken {
                available.store(false, Ordering::Release);
                receiver.close();
                while let Some(dispatch) = receiver.recv().await {
                    let _ = dispatch
                        .response
                        .send(Err(BrowserDriverError::OutcomeUnknown));
                }
                break;
            }
        }
    });
    (sender, owner.abort_handle())
}

pub(crate) async fn send(
    sender: &mpsc::Sender<Dispatch>,
    request: Request,
    control: Option<BrowserDriverControl>,
    deadline: Duration,
) -> Result<oneshot::Receiver<Result<Response, BrowserDriverError>>, BrowserDriverError> {
    if serde_json::to_vec(&request)
        .map_err(|_| BrowserDriverError::Failed)?
        .len()
        > crate::framing::MAX_FRAME_BYTES
    {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let (response, receiver) = oneshot::channel();
    sender
        .try_send(Dispatch {
            request,
            control,
            deadline,
            response,
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => BrowserDriverError::LimitExceeded,
            mpsc::error::TrySendError::Closed(_) => BrowserDriverError::OutcomeUnknown,
        })?;
    Ok(receiver)
}

pub(crate) fn checked_response(value: Response) -> Result<Response, BrowserDriverError> {
    match value {
        Response::Rejected { code } => Err(code.into()),
        value => Ok(value),
    }
}
