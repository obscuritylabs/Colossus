//! One process allocation stays fenced until positively acknowledged retirement.
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Slot<'a>(&'a AtomicBool);
impl<'a> Slot<'a> {
    pub(super) fn reserve(gate: &'a AtomicBool) -> Option<Self> {
        gate.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(gate))
    }

    // Consuming the sole non-Clone owner prevents an old receipt from releasing
    // a subsequent allocation. Dropping an unacknowledged slot stays fenced.
    pub(super) fn retire(self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Drop for Slot<'_> {
    fn drop(&mut self) {
        // Only retire() can acknowledge cleanup. Lost custody stays fenced.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_unknown_custody_denies_new_allocations_until_positive_retirement() {
        let gate = AtomicBool::new(false);
        let mut keeper = Some(Slot::reserve(&gate).unwrap());
        for _ in 0..4096 {
            assert!(Slot::reserve(&gate).is_none());
        }
        keeper.take().unwrap().retire();
        let next = Slot::reserve(&gate).unwrap();
        assert!(Slot::reserve(&gate).is_none());
        next.retire();
        assert!(!gate.load(Ordering::Acquire));
    }

    #[test]
    fn dropped_custody_is_not_a_cleanup_acknowledgement() {
        let gate = AtomicBool::new(false);
        drop(Slot::reserve(&gate).unwrap());
        assert!(gate.load(Ordering::Acquire));
        assert!(Slot::reserve(&gate).is_none());
    }
}
