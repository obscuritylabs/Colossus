use rubato::{FftFixedInOut, Resampler as _};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{DictationError, contract::SAMPLE_RATE};

pub(crate) struct Resampler {
    filter: FftFixedInOut<f32>,
    input: Zeroizing<Vec<f32>>,
    rate: usize,
    received: u64,
    emitted: u64,
    delay: usize,
}

impl Resampler {
    pub(crate) fn new(rate: usize) -> Result<Self, DictationError> {
        if ![8000, 16000, 22050, 24000, 32000, 44100, 48000, 88200, 96000].contains(&rate) {
            return Err(DictationError::CaptureUnsupported);
        }
        let filter = FftFixedInOut::new(rate, SAMPLE_RATE, 1024, 1)
            .map_err(|_| DictationError::CaptureUnsupported)?;
        let input = Zeroizing::new(Vec::with_capacity(filter.input_frames_next()));
        let delay = filter.output_delay();
        Ok(Self {
            filter,
            input,
            rate,
            received: 0,
            emitted: 0,
            delay,
        })
    }

    pub(crate) fn push(
        &mut self,
        mut samples: &[f32],
        emit: &mut impl FnMut(&[f32]) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        self.received += samples.len() as u64;
        while !samples.is_empty() {
            let count = samples
                .len()
                .min(self.filter.input_frames_next() - self.input.len());
            self.input.extend_from_slice(&samples[..count]);
            samples = &samples[count..];
            if self.input.len() == self.filter.input_frames_next() {
                self.process(emit)?;
            }
        }
        Ok(())
    }

    fn process(
        &mut self,
        emit: &mut impl FnMut(&[f32]) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        let mut output = self
            .filter
            .process(&[self.input.as_slice()], None)
            .map_err(|_| DictationError::CaptureUnsupported)?;
        self.input.zeroize();
        self.input.clear();
        let skip = self.delay.min(output[0].len());
        self.delay -= skip;
        let remaining = self.received * SAMPLE_RATE as u64 / self.rate as u64 - self.emitted;
        let count = output[0][skip..]
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let result = emit(&output[0][skip..skip + count]);
        self.emitted += count as u64;
        output.zeroize();
        result
    }

    pub(crate) fn finish(
        &mut self,
        emit: &mut impl FnMut(&[f32]) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        if self.received == 0 {
            return Ok(());
        }
        self.input.resize(self.filter.input_frames_next(), 0.0);
        self.process(emit)?;
        // One zero block releases the FFT filter's trailing overlap. Trim it
        // to the exact received duration, so padding cannot become new speech.
        self.input.resize(self.filter.input_frames_next(), 0.0);
        self.process(emit)
    }

    pub(crate) fn reset(&mut self) -> Result<(), DictationError> {
        *self = Self::new(self.rate)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_boundaries_and_filter_tail_preserve_duration() {
        for rate in [16000, 44100, 48000, 96000] {
            let mut resampler = Resampler::new(rate).unwrap();
            let mut output = Vec::new();
            let mut emit = |samples: &[f32]| {
                output.extend_from_slice(samples);
                Ok(())
            };
            for chunk in vec![0.25; rate + rate / 2].chunks(137) {
                resampler.push(chunk, &mut emit).unwrap();
            }
            resampler.finish(&mut emit).unwrap();
            assert_eq!(output.len(), 24000, "rate {rate}");
            assert!(
                output[1000..23000]
                    .iter()
                    .all(|value| (*value - 0.25).abs() < 0.01)
            );
        }
        assert!(matches!(
            Resampler::new(0),
            Err(DictationError::CaptureUnsupported)
        ));
        assert!(matches!(
            Resampler::new(96001),
            Err(DictationError::CaptureUnsupported)
        ));
    }
}
