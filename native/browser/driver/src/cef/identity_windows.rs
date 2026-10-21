//! Apply a native human decision only on the owning CEF thread and live request.
use super::*;

impl Callbacks {
    pub fn enable_windows_identity_review(&self, enabled: bool) -> Result<(), BrowserDriverError> {
        self.identity_review
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .enable(enabled);
        Ok(())
    }
}

impl Host {
    pub(super) fn service_identity_review(&self) {
        let ready = self
            .callbacks
            .identity_review
            .lock()
            .ok()
            .and_then(|mut review| review.ready());
        let Some((request, index)) = ready else {
            return;
        };
        let current = !request.cancelled()
            && !self.suspended
            && !self.cancelled.load(Ordering::Acquire)
            && !self.callbacks.identity_revoked.load(Ordering::Acquire)
            && self.presentation.control_generation == request.control_generation
            && self.callbacks.state.lock().is_ok_and(|state| {
                state.generations.get(&request.tab) == Some(&request.generation)
                    && state.tab_outcome(request.tab).is_ok()
            })
            && self
                .callbacks
                .origins
                .lock()
                .is_ok_and(|origins| origins.contains(&request.origin));
        let selected = if current { index } else { -1 };
        // SAFETY: exact native pending request on the CEF owning thread. The
        // native one-use ID independently rejects changed documents, cancelled
        // requests, replaced tabs and expired handshakes before accessing a key.
        let status = unsafe {
            ffi::colossus_cef_select_identity(
                request.tab,
                request.generation,
                request.request_id,
                selected,
            )
        };
        let outcome = if status != 0 {
            "stale"
        } else if !current {
            "revoked"
        } else if selected < 0 {
            "declined"
        } else {
            "selected"
        };
        eprintln!("EVIDENCE native-host-client-identity outcome={outcome}");
    }
}
