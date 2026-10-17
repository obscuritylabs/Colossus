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

/// One UI-thread queue, serviced even while a finite Core operation is waiting.
pub(crate) struct Lane {
    receiver: mpsc::Receiver<Command>,
    revoked: Arc<AtomicBool>,
}

pub(crate) trait Target {
    fn control(
        &mut self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError>;
    fn frame(&mut self, lease: Lease) -> Result<Option<Frame>, PresentationError>;
    fn revoke(&mut self);
}

impl Lane {
    pub(crate) fn new(receiver: mpsc::Receiver<Command>, revoked: Arc<AtomicBool>) -> Self {
        Self { receiver, revoked }
    }
    pub(crate) fn service(&mut self, host: &mut impl Target, waiting: bool) {
        if self.revoked.load(Ordering::Acquire) {
            host.revoke();
        }
        // Each caller retains its own absolute operation deadline. Continuous
        // viewer traffic cannot hold this main-thread wait or starve CEF work.
        for _ in 0..16 {
            let Ok(command) = self.receiver.try_recv() else {
                break;
            };
            match command {
                Command::Control(command, sender) if !sender.is_closed() => {
                    let closing = matches!(
                        command,
                        PresentationCommand::Hide { .. }
                            | PresentationCommand::Focus { focused: false, .. }
                    );
                    let allowed = closing || matches!(command, PresentationCommand::Renew { .. });
                    let result = if self.revoked.load(Ordering::Acquire) && !closing {
                        Err(PresentationError::Hidden)
                    } else if waiting && !allowed {
                        // Retire read-only queries and unexecuted mutations as
                        // known stale, rather than leaving their channel to time
                        // out behind this potentially long data operation.
                        Err(PresentationError::Stale)
                    } else {
                        host.control(command)
                    };
                    let _ = sender.send(result);
                }
                Command::Frame(lease, sender) if !sender.is_closed() => {
                    let result = if self.revoked.load(Ordering::Acquire) {
                        Err(PresentationError::Hidden)
                    } else if waiting {
                        Err(PresentationError::Stale)
                    } else {
                        host.frame(lease)
                    };
                    let _ = sender.send(result);
                }
                Command::Revoke => host.revoke(),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "presentation_wait_tests.rs"]
mod wait_tests;
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
