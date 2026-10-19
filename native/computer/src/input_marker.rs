//! Input loopback correlation, never an authorization token.
use std::num::NonZeroU32;

#[derive(Clone, Copy)]
pub(crate) struct InputMarker(NonZeroU32);

impl InputMarker {
    pub(crate) fn generate(mut fill: impl FnMut(&mut [u8; 4]) -> bool) -> Option<Self> {
        for _ in 0..32 {
            let mut bytes = [0; 4];
            if !fill(&mut bytes) {
                return None;
            }
            // Some Windows input paths transform the upper pointer bits. Select
            // a positive value representable unchanged in 32- and 64-bit paths;
            // never truncate or normalize a value received by our hooks.
            if let Some(value) = NonZeroU32::new(u32::from_le_bytes(bytes) & 0x7fff_ffff) {
                return Some(Self(value));
            }
        }
        None
    }

    pub(crate) fn value(self) -> usize {
        self.0.get() as usize
    }

    pub(crate) fn matches(self, received: usize, injected: bool) -> bool {
        injected && received == self.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_rejected_and_regenerated_without_unbounded_work() {
        let mut calls = 0;
        let marker = InputMarker::generate(|bytes| {
            calls += 1;
            *bytes = if calls == 1 {
                [0; 4]
            } else {
                713u32.to_le_bytes()
            };
            true
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(marker.value(), 713);
        calls = 0;
        assert!(
            InputMarker::generate(|bytes| {
                calls += 1;
                *bytes = [0; 4];
                true
            })
            .is_none()
        );
        assert_eq!(calls, 32);
        assert!(InputMarker::generate(|_| false).is_none());
    }

    #[test]
    fn selected_marker_is_nonzero_and_positive_in_both_pointer_widths() {
        for source in [1u32, 0x7fff_ffff, 0x8000_0001, 0xffff_ffff] {
            let marker = InputMarker::generate(|bytes| {
                *bytes = source.to_le_bytes();
                true
            })
            .unwrap();
            assert!((1..=0x7fff_ffff).contains(&marker.value()));
            assert_eq!(marker.value(), (source & 0x7fff_ffff) as usize);
        }
    }

    #[test]
    fn physical_or_foreign_events_never_match_an_injected_loopback() {
        let marker = InputMarker::generate(|bytes| {
            *bytes = 713u32.to_le_bytes();
            true
        })
        .unwrap();
        assert!(marker.matches(713, true));
        assert!(!marker.matches(713, false));
        assert!(!marker.matches(0, true));
        assert!(!marker.matches(714, true));
        if usize::BITS > 32 {
            let same_low_bits = ((1u64 << 32) | 713) as usize;
            assert_eq!(same_low_bits as u32, marker.value() as u32);
            assert!(!marker.matches(same_low_bits, true));
        }
    }
}
