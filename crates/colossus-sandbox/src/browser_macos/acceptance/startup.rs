//! Cooperative startup cancellation without discarding retained native owners.

use colossus_ports::{BrowserDriverControl, BrowserDriverError};
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) struct Startup<'a> {
    control: &'a BrowserDriverControl,
    stopped: &'a AtomicBool,
}

impl<'a> Startup<'a> {
    pub(super) fn new(control: &'a BrowserDriverControl, stopped: &'a AtomicBool) -> Self {
        Self { control, stopped }
    }

    /// Check immediately before each native admission boundary. This is a
    /// cooperative checkpoint, not atomic cancellation-versus-syscall exclusion.
    pub(super) fn check(&self) -> Result<(), BrowserDriverError> {
        if self.control.is_cancelled() || self.stopped.load(Ordering::Acquire) {
            Err(BrowserDriverError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(super) fn check_io(&self) -> std::io::Result<()> {
        self.check().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Interrupted, "browser startup cancelled")
        })
    }

    /// Drop only the startup future on cancellation. Its resource mutex guard
    /// and temporary sockets are released; native owners remain in the session
    /// map for explicit cleanup, including any interrupted bootstrap exchange.
    pub(super) async fn wait<T>(
        &self,
        future: impl Future<Output = Result<T, BrowserDriverError>>,
    ) -> Result<T, BrowserDriverError> {
        self.check()?;
        tokio::select! {
            biased;
            error = self.cancelled() => Err(error),
            result = future => {
                self.check()?;
                result
            }
        }
    }

    async fn cancelled(&self) -> BrowserDriverError {
        let mut poll = tokio::time::interval(Duration::from_millis(10));
        loop {
            poll.tick().await;
            if let Err(error) = self.check() {
                return error;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_ports::RunControl;
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::UnixStream,
        sync::{Mutex, oneshot},
    };

    #[tokio::test]
    async fn cancelled_broker_readiness_releases_guard_and_prevents_later_dispatch() {
        for (cause, becomes_ready) in [
            ("request", false),
            ("authority", false),
            ("supervisor", false),
            ("request", true),
            ("authority", true),
            ("supervisor", true),
        ] {
            let request = RunControl::default();
            let authority = RunControl::default();
            let control = BrowserDriverControl::new(request.clone(), authority.clone());
            let stopped = Arc::new(AtomicBool::new(false));
            // An owner is retained independently of the cancellable startup
            // future. No native process, certificate store or cleanup receipt
            // is involved in this socket-boundary test.
            let retained = Arc::new(Mutex::new(false));
            let owner = Arc::clone(&retained);
            let stop = Arc::clone(&stopped);
            let (mut readiness, mut broker) = UnixStream::pair().unwrap();
            let (mut bootstrap, mut native) = UnixStream::pair().unwrap();
            let (entered, waiting) = oneshot::channel();
            let task = tokio::spawn(async move {
                let startup = Startup::new(&control, &stop);
                let checkpoint = &startup;
                startup
                    .wait(async move {
                        let mut owner = owner.lock().await;
                        entered.send(()).unwrap();
                        let mut ready = [0; 1];
                        readiness
                            .read_exact(&mut ready)
                            .await
                            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                        checkpoint.check()?;
                        *owner = true; // Represents the next native dispatch.
                        checkpoint.check()?;
                        bootstrap
                            .write_all(&[7; 32])
                            .await
                            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                        Ok(())
                    })
                    .await
            });
            waiting.await.unwrap();
            match cause {
                "request" => request.cancel(),
                "authority" => authority.cancel(),
                "supervisor" => stopped.store(true, Ordering::Release),
                _ => unreachable!(),
            }
            if becomes_ready {
                // The awaited response can become ready in the same turn as
                // cancellation. A completed wait cannot admit the next effect.
                broker.write_all(&[1]).await.unwrap();
            }
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), task)
                    .await
                    .unwrap()
                    .unwrap(),
                Err(BrowserDriverError::Cancelled)
            );
            assert!(!*retained.try_lock().unwrap());
            let mut byte = [0; 1];
            assert_eq!(native.read(&mut byte).await.unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn already_cancelled_startup_never_polls_the_effect_future() {
        let request = RunControl::default();
        request.cancel();
        let control = BrowserDriverControl::new(request, RunControl::default());
        let stopped = AtomicBool::new(false);
        let dispatched = AtomicBool::new(false);
        let result = Startup::new(&control, &stopped)
            .wait(async {
                dispatched.store(true, Ordering::Release);
                Ok(())
            })
            .await;
        assert_eq!(result, Err(BrowserDriverError::Cancelled));
        assert!(!dispatched.load(Ordering::Acquire));
    }
}
