use std::time::Instant;
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    DictationError, TranscriptUpdate,
    contract::{MAX_TEXT_BYTES, PARTIAL_SAMPLES, SAMPLE_RATE, SEGMENT_SAMPLES},
};

pub(crate) trait Decoder {
    fn decode(&mut self, samples: &[f32]) -> Result<String, DictationError>;
}

pub(crate) struct Pipeline<D> {
    decoder: D,
    audio: Zeroizing<Vec<f32>>,
    segment_id: u64,
    revision: u32,
    next_partial: usize,
}

impl<D: Decoder> Pipeline<D> {
    pub(crate) fn new(decoder: D) -> Self {
        Self {
            decoder,
            audio: Zeroizing::new(Vec::with_capacity(SEGMENT_SAMPLES)),
            segment_id: 1,
            revision: 0,
            next_partial: PARTIAL_SAMPLES,
        }
    }

    pub(crate) fn push(
        &mut self,
        mut samples: &[f32],
        emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        if samples.len() > SAMPLE_RATE || samples.iter().any(|sample| !sample.is_finite()) {
            return Err(DictationError::CaptureUnsupported);
        }
        while !samples.is_empty() {
            let count = samples.len().min(self.next_partial - self.audio.len());
            self.audio.extend_from_slice(&samples[..count]);
            samples = &samples[count..];
            if self.audio.len() == SEGMENT_SAMPLES {
                self.finish(emit)?;
            } else if self.audio.len() == self.next_partial {
                emit(self.update(false)?)?;
                self.next_partial += PARTIAL_SAMPLES;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(
        &mut self,
        emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        if !self.audio.is_empty() {
            emit(self.update(true)?)?;
            self.audio.zeroize();
            self.audio.clear();
            self.segment_id = self
                .segment_id
                .checked_add(1)
                .ok_or(DictationError::TranscriptLimit)?;
            self.revision = 0;
            self.next_partial = PARTIAL_SAMPLES;
        }
        Ok(())
    }

    fn update(&mut self, is_final: bool) -> Result<TranscriptUpdate, DictationError> {
        let count = self.audio.len();
        self.audio.resize(count.max(SAMPLE_RATE), 0.0);
        let started = Instant::now();
        let text = self.decoder.decode(&self.audio)?;
        let inference_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.audio.truncate(count);
        if text.len() > MAX_TEXT_BYTES {
            return Err(DictationError::TranscriptLimit);
        }
        self.revision += 1;
        Ok(TranscriptUpdate {
            segment_id: self.segment_id,
            revision: self.revision,
            is_final,
            text,
            audio_ms: u64::try_from(count).unwrap_or(u64::MAX) / 16,
            inference_ms,
        })
    }
}

#[cfg(test)]
mod tests;
