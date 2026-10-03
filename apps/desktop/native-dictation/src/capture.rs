use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc::{self, Receiver, SyncSender},
};

use cpal::{
    FromSample, Sample as _, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _},
};
use zeroize::Zeroizing;

use crate::{
    DictationError, TranscriptUpdate,
    meter::InputMeter,
    pipeline::{Decoder, Pipeline},
    resample::Resampler,
};

const MAX_CALLBACK_FRAMES: usize = 8192;
const QUEUE_BLOCKS: usize = 256;

struct CaptureQueue {
    sender: SyncSender<CaptureBlock>,
    failure: Arc<AtomicU8>,
    meter: Arc<InputMeter>,
}

impl CaptureQueue {
    fn send<T: SizedSample>(&self, samples: &[T], channels: u16)
    where
        f32: FromSample<T>,
    {
        if self.failure.load(Ordering::Relaxed) != 0 {
            return;
        }
        if samples.len() / usize::from(channels) > MAX_CALLBACK_FRAMES
            || !samples.len().is_multiple_of(usize::from(channels))
        {
            self.failure.store(3, Ordering::Relaxed);
            return;
        }
        let mono: Zeroizing<Vec<f32>> = Zeroizing::new(
            samples
                .chunks_exact(usize::from(channels))
                .map(|frame| {
                    frame
                        .iter()
                        .map(|sample| f32::from_sample(*sample))
                        .sum::<f32>()
                        / f32::from(channels)
                })
                .collect(),
        );
        if mono.iter().any(|sample| !sample.is_finite()) {
            self.failure.store(3, Ordering::Relaxed);
        } else {
            self.meter.observe(&mono);
            if self.sender.try_send(CaptureBlock::Audio(mono)).is_err() {
                self.failure.store(2, Ordering::Relaxed);
            }
        }
    }
}

enum CaptureBlock {
    Audio(Zeroizing<Vec<f32>>),
    Boundary,
}

pub(crate) struct Capture {
    stream: Option<Stream>,
    sender: SyncSender<CaptureBlock>,
    receiver: Receiver<CaptureBlock>,
    failure: Arc<AtomicU8>,
    resampler: Resampler,
}

impl Capture {
    pub(crate) fn start() -> Result<Self, DictationError> {
        Self::start_with_meter(Arc::default())
    }

    pub(crate) fn start_with_meter(meter: Arc<InputMeter>) -> Result<Self, DictationError> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or(DictationError::MicrophoneMissing)?;
        let supported = device
            .default_input_config()
            .map_err(|_| DictationError::CaptureUnavailable)?;
        let config: StreamConfig = supported.into();
        if config.channels == 0 || config.channels > 8 {
            return Err(DictationError::CaptureUnsupported);
        }
        let resampler = Resampler::new(config.sample_rate as usize)?;
        let (sender, receiver) = mpsc::sync_channel(QUEUE_BLOCKS);
        let failure = Arc::new(AtomicU8::new(0));
        let queue = CaptureQueue {
            sender: sender.clone(),
            failure: failure.clone(),
            meter,
        };
        let error_flag = failure.clone();
        let channels = config.channels;
        let on_error = move |_| {
            error_flag.store(1, Ordering::Relaxed);
        };
        let stream = match supported.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _| queue.send(data, channels),
                on_error,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _| queue.send(data, channels),
                on_error,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                config,
                move |data: &[u16], _| queue.send(data, channels),
                on_error,
                None,
            ),
            SampleFormat::I32 => device.build_input_stream(
                config,
                move |data: &[i32], _| queue.send(data, channels),
                on_error,
                None,
            ),
            _ => return Err(DictationError::CaptureUnsupported),
        }
        .map_err(|_| DictationError::CaptureUnavailable)?;
        stream
            .play()
            .map_err(|_| DictationError::CaptureUnavailable)?;
        Ok(Self {
            stream: Some(stream),
            sender,
            receiver,
            failure,
            resampler,
        })
    }

    pub(crate) fn poll<D: Decoder>(
        &mut self,
        pipeline: &mut Pipeline<D>,
        emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        // Consume at most one block per poll, leaving the controller able to
        // service pause and stop between bounded inference calls.
        self.check()?;
        if let Ok(CaptureBlock::Audio(block)) = self.receiver.try_recv() {
            self.resampler
                .push(&block, &mut |samples| pipeline.push(samples, emit))?;
        }
        self.check()
    }

    /// Insert a FIFO boundary while retaining the same microphone stream. Audio
    /// enqueued after the marker belongs to the next draft, including callbacks
    /// that were still being prepared when the boundary was requested.
    pub(crate) fn finish_turn<D: Decoder>(
        &mut self,
        pipeline: &mut Pipeline<D>,
        emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        self.check()?;
        self.sender
            .try_send(CaptureBlock::Boundary)
            .map_err(|_| DictationError::CaptureOverrun)?;
        while let Ok(block) = self.receiver.try_recv() {
            match block {
                CaptureBlock::Audio(audio) => self
                    .resampler
                    .push(&audio, &mut |samples| pipeline.push(samples, emit))?,
                CaptureBlock::Boundary => {
                    self.resampler
                        .finish(&mut |samples| pipeline.push(samples, emit))?;
                    pipeline.finish(emit)?;
                    return self.resampler.reset();
                }
            }
        }
        Err(DictationError::CaptureUnavailable)
    }

    pub(crate) fn finish<D: Decoder>(
        mut self,
        pipeline: &mut Pipeline<D>,
        emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
    ) -> Result<(), DictationError> {
        // Dropping the stream releases capture before draining or inference.
        self.stream.take();
        self.check()?;
        for block in self.receiver.try_iter() {
            if let CaptureBlock::Audio(audio) = block {
                self.resampler
                    .push(&audio, &mut |samples| pipeline.push(samples, emit))?;
            }
        }
        self.resampler
            .finish(&mut |samples| pipeline.push(samples, emit))?;
        pipeline.finish(emit)
    }

    fn check(&self) -> Result<(), DictationError> {
        match self.failure.load(Ordering::Relaxed) {
            0 => Ok(()),
            2 => Err(DictationError::CaptureOverrun),
            3 => Err(DictationError::CaptureUnsupported),
            _ => Err(DictationError::CaptureUnavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overload_and_device_failure_stop_instead_of_dropping_speech() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let failure = Arc::new(AtomicU8::new(0));
        let queue = CaptureQueue {
            sender,
            failure: failure.clone(),
            meter: Arc::default(),
        };
        queue.send(&[0.5_f32, 0.25], 2);
        let CaptureBlock::Audio(audio) = receiver.try_recv().unwrap() else {
            panic!("expected audio")
        };
        assert_eq!(audio.as_slice(), [0.375]);
        assert_eq!(queue.meter.take(), 219);
        queue.send(&[0.0_f32], 1);
        queue.send(&[0.0_f32], 1);
        assert_eq!(failure.load(Ordering::Relaxed), 2);
        receiver.try_recv().unwrap();
        queue.send(&[1.0_f32], 1);
        assert!(receiver.try_recv().is_err());
        failure.store(1, Ordering::Relaxed);
        queue.send(&[1.0_f32], 1);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn callback_bounds_are_checked_before_audio_allocation() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let failure = Arc::new(AtomicU8::new(0));
        let queue = CaptureQueue {
            sender,
            failure: failure.clone(),
            meter: Arc::default(),
        };
        queue.send(&vec![0.0_f32; MAX_CALLBACK_FRAMES + 1], 1);
        assert_eq!(failure.load(Ordering::Relaxed), 3);
        failure.store(0, Ordering::Relaxed);
        queue.send(&[f32::NAN], 1);
        assert_eq!(failure.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn send_marker_settles_prior_audio_and_keeps_later_audio_for_the_next_turn() {
        struct DurationDecoder;
        impl Decoder for DurationDecoder {
            fn decode(&mut self, samples: &[f32]) -> Result<String, DictationError> {
                Ok(samples.len().to_string())
            }
        }
        let (sender, receiver) = mpsc::sync_channel(QUEUE_BLOCKS);
        for _ in 0..4 {
            sender
                .send(CaptureBlock::Audio(Zeroizing::new(vec![0.25; 8000])))
                .unwrap();
        }
        let mut capture = Capture {
            stream: None,
            sender: sender.clone(),
            receiver,
            failure: Arc::new(AtomicU8::new(0)),
            resampler: Resampler::new(16_000).unwrap(),
        };
        let mut pipeline = Pipeline::new(DurationDecoder);
        let mut updates = Vec::new();
        let mut later_sent = false;
        capture
            .finish_turn(&mut pipeline, &mut |update| {
                if !later_sent {
                    // Simulates a callback enqueuing new speech after the FIFO marker.
                    sender
                        .send(CaptureBlock::Audio(Zeroizing::new(vec![0.5; 8000])))
                        .unwrap();
                    later_sent = true;
                }
                updates.push(update);
                Ok(())
            })
            .unwrap();
        assert_eq!(updates.last().unwrap().audio_ms, 2000);
        assert!(updates.last().unwrap().is_final);
        capture
            .poll(&mut pipeline, &mut |update| {
                updates.push(update);
                Ok(())
            })
            .unwrap();
        capture
            .finish_turn(&mut pipeline, &mut |update| {
                updates.push(update);
                Ok(())
            })
            .unwrap();
        assert_eq!(updates.last().unwrap().segment_id, 2);
        assert_eq!(updates.last().unwrap().audio_ms, 500);
        assert!(updates.last().unwrap().is_final);
    }
}
