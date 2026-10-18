use super::{CompletedSurface, Owner, Pixels, ffi};
use crate::BrowserError;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::sync::oneshot;

impl CompletedSurface {
    /// Destroy the static child and release retained pixels after native acknowledgment.
    ///
    /// # Errors
    /// A missing UI acknowledgment retains the image and cleanup obligation.
    pub async fn close(&self) -> Result<(), BrowserError> {
        let _closing = self.0.closing.lock().await;
        {
            let mut state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
            state.closed = true;
            state.visible = false;
        }
        let handle = self.0.handle.load(Ordering::Acquire);
        if handle == 0 {
            return Ok(());
        }
        let (send, receive) = oneshot::channel();
        self.0
            .window
            .run_on_main_thread(move || {
                // SAFETY: exact owned static child; no callbacks or browser host exist.
                let _ = send.send(unsafe { ffi::colossus_presenter_destroy(handle) } == 0);
            })
            .map_err(|_| BrowserError::Closed)?;
        let closed = tokio::time::timeout(Duration::from_secs(2), receive)
            .await
            .map_err(|_| BrowserError::TimedOut)?
            .map_err(|_| BrowserError::Closed)?;
        if !closed {
            return Err(BrowserError::Closed);
        }
        self.0.handle.store(0, Ordering::Release);
        self.0
            .pixels
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .take();
        Ok(())
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let handle = self.handle.swap(0, Ordering::AcqRel);
        if handle == 0 {
            return;
        }
        let pixels = self
            .pixels
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        // Keep the image and both reservations charged if native cleanup cannot
        // be acknowledged. Scheduling failure must not undercount native pixels.
        let retained = pixels.map_or(0, |pixels| Arc::into_raw(pixels) as usize);
        let _ = self.window.run_on_main_thread(move || {
            // SAFETY: creation-thread destruction for this exact opaque identity.
            if unsafe { ffi::colossus_presenter_destroy(handle) } == 0 && retained != 0 {
                // SAFETY: balance the sole raw retain installed immediately above.
                unsafe {
                    drop(Arc::from_raw(retained as *const Pixels));
                }
            }
        });
    }
}
