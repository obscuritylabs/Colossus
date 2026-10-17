use super::{Owner, PresentationSurface, ffi};
use crate::BrowserError;
use colossus_browser_presentation::Lease;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

impl PresentationSurface {
    fn revoke(&self) -> Result<(), BrowserError> {
        self.owner
            .guard
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .hide();
        self.owner
            .frames
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .clear();
        Ok(())
    }
    /// Revoke input synchronously and schedule native pixel/focus removal.
    ///
    /// # Errors
    /// Reports poisoned ownership or unavailable UI scheduling.
    pub fn hide(&self) -> Result<(), BrowserError> {
        self.revoke()?;
        let handle = self.handle.load(Ordering::Acquire);
        self.window
            .run_on_main_thread(move || {
                // SAFETY: exact native identity; retired identities cannot route reused OS handles.
                let _ = unsafe { ffi::colossus_presenter_visible(handle, 0) };
            })
            .map_err(|_| BrowserError::Closed)
    }
    /// Wait until native pixels and input focus are removed on the owning UI thread.
    ///
    /// # Errors
    /// Scheduling, native removal or missing acknowledgment never proves a hide barrier.
    pub async fn hide_confirmed(&self) -> Result<(), BrowserError> {
        self.revoke()?;
        let handle = self.handle.load(Ordering::Acquire);
        let (send, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                // SAFETY: exact native view; acknowledgment is sent only after actual native removal.
                let status = unsafe { ffi::colossus_presenter_visible(handle, 0) };
                let _ = send.send(status == 0);
            })
            .map_err(|_| BrowserError::Closed)?;
        tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?
            .then_some(())
            .ok_or(BrowserError::Closed)
    }
    /// Renew an unchanged lease only after the real native timer acknowledges it.
    ///
    /// # Errors
    /// An expired native epoch requires a fresh view; scheduling cannot resurrect it.
    pub async fn renew_confirmed(&self, lease: Lease, ttl: Duration) -> Result<(), BrowserError> {
        self.owner
            .guard
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .renew(lease, Instant::now(), ttl)
            .map_err(|_| BrowserError::Closed)?;
        let until = Instant::now() + ttl;
        let handle = self.handle.load(Ordering::Acquire);
        let (send, receive) = oneshot::channel();
        self.window
            .run_on_main_thread(move || {
                let remaining =
                    u32::try_from(until.saturating_duration_since(Instant::now()).as_millis())
                        .unwrap_or(0);
                // SAFETY: exact native epoch and TTL measured after main-thread scheduling delay.
                let status = unsafe {
                    ffi::colossus_presenter_lease(handle, lease.viewport_generation, remaining)
                };
                let _ = send.send(status == 0);
            })
            .map_err(|_| BrowserError::Closed)?;
        let accepted = tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?;
        if !accepted {
            self.revoke()?;
            return Err(BrowserError::Closed);
        }
        self.owner
            .guard
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .renew(
                lease,
                Instant::now(),
                until.saturating_duration_since(Instant::now()),
            )
            .map_err(|_| BrowserError::Closed)
    }
}
impl Drop for PresentationSurface {
    fn drop(&mut self) {
        let _ = self.revoke();
        let handle = self.handle.swap(0, Ordering::AcqRel);
        if handle == 0 {
            return;
        }
        let retained = Arc::as_ptr(&self.owner) as usize;
        let _ = self.window.run_on_main_thread(move || {
            // SAFETY: native view retains this separate Arc until acknowledged destruction.
            if unsafe { ffi::colossus_presenter_destroy(handle) } == 0 {
                // SAFETY: release exactly the Arc::into_raw retain installed at creation.
                unsafe {
                    drop(Arc::from_raw(retained as *const Owner));
                }
            }
            // Unacknowledged cleanup deliberately retains callback ownership.
        });
    }
}
