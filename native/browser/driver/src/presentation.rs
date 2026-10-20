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
    revocation_applied: bool,
    #[cfg(all(windows, debug_assertions))]
    waiting_observation_diagnosed: bool,
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
        Self {
            receiver,
            revoked,
            revocation_applied: false,
            #[cfg(all(windows, debug_assertions))]
            waiting_observation_diagnosed: false,
        }
    }
    pub(crate) fn service(&mut self, host: &mut impl Target, waiting: bool) {
        if self.revoked.load(Ordering::Acquire) {
            self.revoke_once(host);
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
                    // Agent readers use only cached metadata and the existing
                    // exact native document/lease/frame validators. They do not
                    // pump CEF, await DevTools or adopt the writer's document.
                    let agent_observation = matches!(
                        command,
                        PresentationCommand::Observe { lease } if lease.control_generation > 0
                    );
                    let allowed = closing
                        || agent_observation
                        || matches!(command, PresentationCommand::Renew { .. });
                    let result = if self.revoked.load(Ordering::Acquire) && !closing {
                        Err(PresentationError::Hidden)
                    } else if waiting && !allowed {
                        #[cfg(all(windows, debug_assertions))]
                        if matches!(command, PresentationCommand::Observe { .. })
                            && !self.waiting_observation_diagnosed
                        {
                            self.waiting_observation_diagnosed = true;
                            eprintln!(
                                "EVIDENCE native-host-observe rejected=core_wait_in_progress"
                            );
                        }
                        // Human observation can adopt native documents, while
                        // configuration/input/handoff can change the page owner.
                        // Reject them before dispatch during a Core operation.
                        Err(PresentationError::Stale)
                    } else {
                        host.control(command)
                    };
                    let _ = sender.send(result);
                }
                Command::Frame(lease, sender) if !sender.is_closed() => {
                    let result = if self.revoked.load(Ordering::Acquire) {
                        Err(PresentationError::Hidden)
                    } else if waiting && lease.control_generation == 0 {
                        Err(PresentationError::Stale)
                    } else {
                        host.frame(lease)
                    };
                    let _ = sender.send(result);
                }
                Command::Revoke => self.revoke_once(host),
                _ => {}
            }
        }
    }

    fn revoke_once(&mut self, host: &mut impl Target) {
        // Keep the terminal admission fence latched, including a queued revoke
        // observed before its atomic notification. Later Core waits must not
        // repeatedly hide or defocus a page activated by agent input.
        self.revoked.store(true, Ordering::Release);
        if !self.revocation_applied {
            self.revocation_applied = true;
            host.revoke();
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
