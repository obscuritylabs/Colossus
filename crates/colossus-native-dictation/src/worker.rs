use std::{
    io::{self, Read as _, Write as _},
    path::Path,
};

use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use zeroize::Zeroizing;

use crate::{
    DictationError,
    contract::{MAX_TEXT_BYTES, SAMPLE_RATE, SEGMENT_SAMPLES},
    model,
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Reply {
    Ready,
    Text { text: String },
    Failure { error: DictationError },
}

fn send(reply: &Reply) -> Result<(), DictationError> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, reply).map_err(|_| DictationError::Console)?;
    output
        .write_all(b"\n")
        .and_then(|()| output.flush())
        .map_err(|_| DictationError::Console)
}

pub(crate) fn run(path: &Path, digest: &str) -> Result<(), DictationError> {
    let result = serve(path, digest);
    if let Err(error) = result {
        let _ = send(&Reply::Failure { error });
    }
    result
}

/// Serve the private helper protocol before the Desktop event loop starts.
/// Returns `None` for an ordinary application launch.
#[must_use]
pub fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--dictation-worker")) {
        return None;
    }
    let result = (|| {
        let path = args.next().ok_or(DictationError::Arguments)?;
        let digest = args
            .next()
            .ok_or(DictationError::Arguments)?
            .into_string()
            .map_err(|_| DictationError::Arguments)?;
        if args.next().is_some() {
            return Err(DictationError::Arguments);
        }
        run(Path::new(&path), &digest)
    })();
    Some(i32::from(result.is_err()))
}

fn serve(path: &Path, digest: &str) -> Result<(), DictationError> {
    let bytes = model::verify(path, digest)?;
    whisper_rs::install_logging_hooks();
    let mut parameters = WhisperContextParameters::default();
    parameters.use_gpu(cfg!(feature = "metal"));
    let context = WhisperContext::new_from_buffer_with_params(&bytes, parameters)
        .map_err(|_| DictationError::ModelUnsupported)?;
    drop(bytes);
    let mut state = context
        .create_state()
        .map_err(|_| DictationError::ModelUnsupported)?;
    send(&Reply::Ready)?;
    let mut input = io::stdin().lock();
    loop {
        let mut header = [0; 4];
        match input.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(_) => return Err(DictationError::Inference),
        }
        let count = u32::from_le_bytes(header) as usize;
        if !(SAMPLE_RATE..=SEGMENT_SAMPLES).contains(&count) {
            return Err(DictationError::Inference);
        }
        let mut bytes = Zeroizing::new(vec![0; count * 4]);
        input
            .read_exact(&mut bytes)
            .map_err(|_| DictationError::Inference)?;
        let samples = Zeroizing::new(
            bytes
                .chunks_exact(4)
                .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
                .collect::<Vec<_>>(),
        );
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(DictationError::Inference);
        }
        let mut parameters = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        parameters.set_n_threads(4);
        parameters.set_language(Some("en"));
        parameters.set_translate(false);
        parameters.set_no_context(true);
        parameters.set_print_special(false);
        parameters.set_print_progress(false);
        parameters.set_print_realtime(false);
        parameters.set_print_timestamps(false);
        // The parent owns a hard process deadline. Do not use whisper-rs's
        // callback adapter: 0.16.0 erases its closure type before invoking a
        // differently typed trampoline and loses the allocation's owner.
        state
            .full(parameters, &samples)
            .map_err(|_| DictationError::Inference)?;
        let mut text = String::new();
        for segment in state.as_iter() {
            let fragment = segment.to_str().map_err(|_| DictationError::Inference)?;
            if text.len() + fragment.len() > MAX_TEXT_BYTES {
                return Err(DictationError::TranscriptLimit);
            }
            text.push_str(fragment);
        }
        send(&Reply::Text {
            text: text.trim().to_owned(),
        })?;
    }
}
