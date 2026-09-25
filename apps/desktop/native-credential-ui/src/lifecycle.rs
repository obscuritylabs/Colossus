use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use colossus_contracts::HostSecret;
use tokio::sync::oneshot;

use crate::PromptError;

static ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) static TEST_OWNERSHIP: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Held by the native window, not the waiting future, so cancelling a caller
/// cannot admit another dialog until the first window has cleared and closed.
pub(crate) struct Completion {
    sender: Option<oneshot::Sender<Result<HostSecret, PromptError>>>,
}

impl Completion {
    pub(crate) fn acquire()
    -> Result<(Self, oneshot::Receiver<Result<HostSecret, PromptError>>), PromptError> {
        ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PromptError::Busy)?;
        let (sender, receiver) = oneshot::channel();
        Ok((
            Self {
                sender: Some(sender),
            },
            receiver,
        ))
    }

    pub(crate) fn finish(mut self, result: Result<HostSecret, PromptError>) {
        // Release ownership before waking a caller that might immediately retry.
        ACTIVE.store(false, Ordering::Release);
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(result);
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            ACTIVE.store(false, Ordering::Release);
            let _ = sender.send(Err(PromptError::Cancelled));
        }
    }
}

pub(crate) struct CancelOnDrop(pub(crate) Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_waiter_does_not_release_native_window_ownership() {
        let _ownership = TEST_OWNERSHIP.lock().unwrap();
        let (completion, receiver) = Completion::acquire().unwrap();
        drop(receiver);
        assert!(matches!(Completion::acquire(), Err(PromptError::Busy)));
        completion.finish(Err(PromptError::Cancelled));
        let (next, mut receiver) = Completion::acquire().unwrap();
        drop(next);
        assert!(matches!(
            receiver.try_recv(),
            Ok(Err(PromptError::Cancelled))
        ));
        let cancelled = Arc::new(AtomicBool::new(false));
        drop(CancelOnDrop(cancelled.clone()));
        assert!(cancelled.load(Ordering::Acquire));
    }
}
