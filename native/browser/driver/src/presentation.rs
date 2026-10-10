//! Bounded private presenter adapter; all CEF effects remain on its owning thread.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use colossus_browser_presentation::{
    Frame, Lease, PresentationCommand, PresentationError, PresentationHost, PresentationReply,
};
use tokio::sync::{mpsc, oneshot};

pub enum Command {
    Control(
        PresentationCommand,
        oneshot::Sender<Result<PresentationReply, PresentationError>>,
    ),
    Frame(
        Lease,
        oneshot::Sender<Result<Option<Frame>, PresentationError>>,
    ),
    Revoke,
}

pub struct Adapter {
    pub sender: mpsc::Sender<Command>,
    pub revoked: Arc<AtomicBool>,
}
#[async_trait]
impl PresentationHost for Adapter {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(PresentationError::Hidden);
        }
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Command::Control(command, sender))
            .await
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        receiver
            .await
            .map_err(|_| PresentationError::OutcomeUnknown)?
    }
    async fn frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(PresentationError::Hidden);
        }
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Command::Frame(lease, sender))
            .await
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        receiver
            .await
            .map_err(|_| PresentationError::OutcomeUnknown)?
    }
    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    async fn revoke(&self) {
        self.revoke_now();
        let _ = self.sender.try_send(Command::Revoke);
    }
}
