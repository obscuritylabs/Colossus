use super::*;

struct Echo;
impl Decoder for Echo {
    fn decode(&mut self, samples: &[f32]) -> Result<String, DictationError> {
        // Every non-padding frame contributes to the result, so reordered,
        // repeated or omitted audio changes the assertion.
        Ok(samples
            .iter()
            .filter(|sample| **sample > 0.0)
            .sum::<f32>()
            .to_string())
    }
}

#[test]
fn long_session_replaces_partials_and_finalizes_each_frame_once() {
    let mut pipeline = Pipeline::new(Echo);
    let mut updates = Vec::new();
    let mut emit = |update| {
        updates.push(update);
        Ok(())
    };
    for _ in 0..600 {
        pipeline.push(&vec![1.0; SAMPLE_RATE], &mut emit).unwrap();
    }
    pipeline.finish(&mut emit).unwrap();
    assert_eq!(updates.len(), 300);
    for (index, segment) in updates.chunks_exact(3).enumerate() {
        assert_eq!(
            segment
                .iter()
                .map(|update| update.revision)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(
            segment
                .iter()
                .all(|update| update.segment_id == index as u64 + 1)
        );
        assert!(!segment[0].is_final);
        assert!(!segment[1].is_final);
        assert!(segment[2].is_final);
        assert_eq!(segment[2].text, SEGMENT_SAMPLES.to_string());
        assert_eq!(segment[2].audio_ms, 6000);
    }
    assert!(pipeline.audio.is_empty());
    assert!(pipeline.audio.capacity() <= SEGMENT_SAMPLES);
}

#[test]
fn pause_and_successive_turn_boundaries_flush_tail_without_replay() {
    let mut pipeline = Pipeline::new(Echo);
    let mut updates = Vec::new();
    let mut emit = |update| {
        updates.push(update);
        Ok(())
    };
    pipeline.push(&vec![1.0; 8000], &mut emit).unwrap();
    pipeline.finish(&mut emit).unwrap();
    pipeline.finish(&mut emit).unwrap();
    pipeline.push(&vec![2.0; 8000], &mut emit).unwrap();
    pipeline.finish(&mut emit).unwrap();
    assert_eq!(updates.len(), 2);
    assert_eq!(
        (
            updates[0].segment_id,
            updates[0].text.as_str(),
            updates[0].audio_ms
        ),
        (1, "8000", 500)
    );
    assert_eq!(
        (
            updates[1].segment_id,
            updates[1].text.as_str(),
            updates[1].audio_ms
        ),
        (2, "16000", 500)
    );
}

#[test]
fn empty_final_replaces_a_partial_and_errors_do_not_emit_a_final() {
    struct Failing;
    impl Decoder for Failing {
        fn decode(&mut self, _: &[f32]) -> Result<String, DictationError> {
            Err(DictationError::Inference)
        }
    }
    struct Revising(bool);
    impl Decoder for Revising {
        fn decode(&mut self, _: &[f32]) -> Result<String, DictationError> {
            self.0 = !self.0;
            Ok(if self.0 { "provisional" } else { "" }.into())
        }
    }
    let mut pipeline = Pipeline::new(Revising(false));
    let mut updates = Vec::new();
    let mut emit = |update| {
        updates.push(update);
        Ok(())
    };
    pipeline.push(&vec![0.0; SAMPLE_RATE], &mut emit).unwrap();
    pipeline.push(&vec![0.0; SAMPLE_RATE], &mut emit).unwrap();
    pipeline.finish(&mut emit).unwrap();
    assert_eq!(updates[0].text, "provisional");
    assert_eq!(updates[1].text, "");
    assert!(updates[1].is_final);
    let mut pipeline = Pipeline::new(Failing);
    pipeline.push(&[1.0], &mut |_| Ok(())).unwrap();
    assert_eq!(
        pipeline.finish(&mut |_| panic!("failed inference must not settle speech")),
        Err(DictationError::Inference)
    );
}

#[test]
fn malformed_audio_and_oversized_transcripts_fail_closed() {
    struct Oversized;
    impl Decoder for Oversized {
        fn decode(&mut self, _: &[f32]) -> Result<String, DictationError> {
            Ok("é".repeat(MAX_TEXT_BYTES))
        }
    }
    let mut pipeline = Pipeline::new(Echo);
    assert_eq!(
        pipeline.push(&[f32::NAN], &mut |_| Ok(())),
        Err(DictationError::CaptureUnsupported)
    );
    assert_eq!(
        pipeline.push(&vec![0.0; SAMPLE_RATE + 1], &mut |_| Ok(())),
        Err(DictationError::CaptureUnsupported)
    );
    let mut pipeline = Pipeline::new(Oversized);
    pipeline.push(&[1.0], &mut |_| Ok(())).unwrap();
    assert_eq!(
        pipeline.finish(&mut |_| panic!("oversized text must not be released")),
        Err(DictationError::TranscriptLimit)
    );
}

#[test]
fn silence_markers_are_removed_without_discarding_speech_or_final_revisions() {
    struct Revising(u8);
    impl Decoder for Revising {
        fn decode(&mut self, _: &[f32]) -> Result<String, DictationError> {
            self.0 += 1;
            Ok(match self.0 {
                1 => "[BLANK_AUDIO]",
                2 => " Keep this speech. [BLANK_AUDIO] ",
                _ => "[BLANK_AUDIO] [BLANK_AUDIO]",
            }
            .into())
        }
    }
    let mut pipeline = Pipeline::new(Revising(0));
    let mut updates = Vec::new();
    let mut emit = |update| {
        updates.push(update);
        Ok(())
    };
    for _ in 0..5 {
        pipeline.push(&vec![0.0; SAMPLE_RATE], &mut emit).unwrap();
    }
    pipeline.finish(&mut emit).unwrap();
    assert_eq!(
        updates
            .iter()
            .map(|update| update.text.as_str())
            .collect::<Vec<_>>(),
        ["", "Keep this speech.", ""]
    );
    assert_eq!(
        updates
            .iter()
            .map(|update| update.revision)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(updates[2].is_final);
}
