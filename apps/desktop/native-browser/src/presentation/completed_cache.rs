//! Bounded native custody of the last frame actually accepted by an agent viewer.
use crate::PageState;
use colossus_browser_presentation::{Frame, Lease};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const MAX_RETAINED_BYTES: usize = 32 * 1024 * 1024;
static RETAINED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Charges both cached pixels and a completed native child's separate paint copy.
pub(super) struct Reservation(usize);
impl Reservation {
    pub(super) fn acquire(bytes: usize) -> Option<Self> {
        if reserve(&RETAINED_BYTES, bytes, MAX_RETAINED_BYTES) {
            Some(Self(bytes))
        } else {
            None
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        RETAINED_BYTES.fetch_sub(self.0, Ordering::AcqRel);
    }
}
fn reserve(counter: &AtomicUsize, bytes: usize, limit: usize) -> bool {
    bytes != 0
        && counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(bytes).filter(|total| *total <= limit)
            })
            .is_ok()
}

pub(super) struct RetainedFrame {
    pub(super) frame: Frame,
    pub(super) page: PageState,
    _reservation: Reservation,
}
#[derive(Default)]
struct State {
    sealed: bool,
    authority: Option<(u64, u64)>,
    invalidated_through: u64,
    frame: Option<Arc<RetainedFrame>>,
    page: Option<(Lease, PageState)>,
}
/// Shared across this Remote attachment's document/viewport surface replacements.
#[derive(Default)]
pub(crate) struct CompletedCache(Mutex<State>);
impl CompletedCache {
    /// Only trusted attach/handoff may establish a new native writer authority.
    pub(crate) fn activate(&self, lease: Lease) {
        if lease.control_generation == 0 {
            return;
        }
        if let Ok(mut state) = self.0.lock()
            && state.authority != Some(authority(lease))
        {
            *state = State {
                authority: Some(authority(lease)),
                ..State::default()
            };
        }
    }
    /// A known document transition cannot retain its prior image as a final candidate.
    pub(crate) fn invalidate(&self, lease: Lease) {
        if let Ok(mut state) = self.0.lock()
            && state.authority == Some(authority(lease))
        {
            state.invalidated_through = state.invalidated_through.max(lease.viewport_generation);
            if state.frame.as_ref().is_some_and(|frame| {
                frame.frame.lease.viewport_generation <= state.invalidated_through
            }) {
                state.frame.take();
            }
            if state
                .page
                .as_ref()
                .is_some_and(|(lease, _)| lease.viewport_generation <= state.invalidated_through)
            {
                state.page.take();
            }
        }
    }
    /// Supplied only by an authenticated observation of this lease's target.
    pub(crate) fn observed(&self, lease: Lease, page: PageState) {
        if let Ok(mut state) = self.0.lock()
            && !state.sealed
            && state.authority == Some(authority(lease))
            && lease.viewport_generation > state.invalidated_through
            && state.page.as_ref().is_none_or(|(current, _)| {
                current.viewport_generation < lease.viewport_generation || *current == lease
            })
        {
            if state
                .frame
                .as_ref()
                .is_some_and(|frame| frame.frame.lease != lease)
            {
                state.frame.take();
            }
            state.page = Some((lease, page));
        }
    }
    /// Call only after authenticated decode AND synchronous native paint acceptance.
    pub(super) fn accepted(&self, frame: Frame) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.sealed
            || state.authority != Some(authority(frame.lease))
            || frame.lease.viewport_generation <= state.invalidated_through
        {
            return;
        }
        let Some(page) = state
            .page
            .as_ref()
            .filter(|(lease, _)| *lease == frame.lease)
            .map(|(_, page)| page.clone())
        else {
            return;
        };
        // Release the previous candidate only after exact current metadata is
        // found. A late old paint cannot remove a newer accepted document.
        state.frame.take();
        let Some(reservation) = Reservation::acquire(frame.pixels.capacity()) else {
            return;
        };
        state.frame = Some(Arc::new(RetainedFrame {
            frame,
            page,
            _reservation: reservation,
        }));
    }
    /// One-way extraction: later frames cannot replace a completed tab's pixels.
    pub(super) fn take(&self, lease: Lease) -> Option<Arc<RetainedFrame>> {
        let mut state = self.0.lock().ok()?;
        state.sealed = true;
        state
            .frame
            .take()
            .filter(|frame| frame.frame.lease == lease)
    }
    pub(crate) fn clear(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.sealed = true;
            state.frame.take();
            state.page.take();
        }
    }
}

fn authority(lease: Lease) -> (u64, u64) {
    (lease.session_generation, lease.control_generation)
}

#[cfg(test)]
#[path = "completed_cache_tests.rs"]
mod tests;
