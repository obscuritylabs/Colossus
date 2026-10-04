use std::sync::atomic::{AtomicU8, Ordering};

/// One coalesced loudness value; neither samples nor queued telemetry are retained.
#[derive(Default)]
pub(crate) struct InputMeter(AtomicU8);

impl InputMeter {
    pub(crate) fn observe(&self, samples: &[f32]) {
        let Ok(count) = u32::try_from(samples.len()) else {
            return;
        };
        if count == 0 {
            return;
        }
        let energy: f64 = samples
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum();
        let rms = (energy / f64::from(count)).sqrt();
        if !rms.is_finite() || rms <= 0.001 {
            return;
        }
        // Map -60..0 dBFS onto a byte. The cast follows an explicit finite clamp.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let level = ((20.0 * rms.log10() + 60.0) / 60.0 * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        self.0.fetch_max(level, Ordering::Relaxed);
    }

    pub(crate) fn take(&self) -> u8 {
        self.0.swap(0, Ordering::Relaxed)
    }

    pub(crate) fn clear(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::InputMeter;

    #[test]
    fn silence_is_zero_and_loudness_is_bounded_and_coalesced() {
        let meter = InputMeter::default();
        meter.observe(&[]);
        meter.observe(&[0.0; 64]);
        meter.observe(&[0.0001; 64]);
        assert_eq!(meter.take(), 0);
        meter.observe(&[0.01; 64]);
        assert_eq!(meter.take(), 85);
        meter.observe(&[0.1; 64]);
        meter.observe(&[0.01; 64]);
        assert_eq!(meter.take(), 170);
        assert_eq!(meter.take(), 0);
        meter.observe(&[2.0; 64]);
        assert_eq!(meter.take(), 255);
        meter.observe(&[f32::NAN]);
        assert_eq!(meter.take(), 0);
        meter.observe(&[1.0; 64]);
        meter.clear();
        assert_eq!(meter.take(), 0);
    }
}
