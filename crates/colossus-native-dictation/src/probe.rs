use std::{
    io::{self, BufRead as _, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use zeroize::Zeroizing;

use crate::{
    DictationError, TranscriptUpdate, capture::Capture, contract::SAMPLE_RATE,
    decoder::WhisperDecoder, pipeline::Pipeline,
};

/// Run the explicit developer probe; never download a model or contact a service.
///
/// # Errors
/// Returns a categorical error after releasing microphone capture on failure.
pub fn run() -> Result<(), DictationError> {
    let mut input = std::env::args_os().skip(1).peekable();
    if input
        .peek()
        .is_some_and(|argument| argument == "--dictation-worker")
    {
        input.next();
        let arguments = Arguments::parse(input)?;
        if arguments.show_text || arguments.wav.is_some() {
            return Err(DictationError::Arguments);
        }
        return crate::worker::run(&arguments.model, &arguments.digest);
    }
    let arguments = Arguments::parse(input)?;
    let started = Instant::now();
    let decoder = WhisperDecoder::load(&arguments.model, &arguments.digest)?;
    let mut output = io::stdout().lock();
    writeln!(output, "{}", json!({"state": "ready", "cold_start_ms": started.elapsed().as_millis(), "backend": if cfg!(feature = "metal") { "metal_requested" } else { "cpu" }}))
        .map_err(|_| DictationError::Console)?;
    let mut pipeline = Pipeline::new(decoder);
    let mut emit = |update: TranscriptUpdate| {
        let mut value = json!({"segment_id": update.segment_id, "revision": update.revision, "is_final": update.is_final, "audio_ms": update.audio_ms, "inference_ms": update.inference_ms, "text_bytes": update.text.len()});
        if arguments.show_text {
            value["text"] = json!(update.text);
        }
        writeln!(output, "{value}")
            .and_then(|()| output.flush())
            .map_err(|_| DictationError::Console)
    };
    if let Some(fixture) = arguments.wav {
        replay(&fixture, &mut pipeline, &mut emit)
    } else {
        live(&mut pipeline, &mut emit)
    }
}

struct Arguments {
    model: PathBuf,
    digest: String,
    show_text: bool,
    wav: Option<PathBuf>,
}

impl Arguments {
    fn parse(mut input: impl Iterator<Item = std::ffi::OsString>) -> Result<Self, DictationError> {
        let model = input.next().ok_or(DictationError::Arguments)?.into();
        let digest = input
            .next()
            .ok_or(DictationError::Arguments)?
            .into_string()
            .map_err(|_| DictationError::Arguments)?;
        let mut show_text = false;
        let mut wav = None;
        while let Some(argument) = input.next() {
            if argument == "--show-text" && !show_text {
                show_text = true;
            } else if argument == "--wav" && wav.is_none() {
                wav = Some(input.next().ok_or(DictationError::Arguments)?.into());
            } else {
                return Err(DictationError::Arguments);
            }
        }
        Ok(Self {
            model,
            digest,
            show_text,
            wav,
        })
    }
}

fn replay(
    path: &Path,
    pipeline: &mut Pipeline<WhisperDecoder>,
    emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
) -> Result<(), DictationError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| DictationError::FixtureUnsupported)?;
    if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
        return Err(DictationError::FixtureUnsupported);
    }
    let file = std::fs::File::open(path).map_err(|_| DictationError::FixtureUnsupported)?;
    let mut reader = hound::WavReader::new(file.take(4 * 1024 * 1024))
        .map_err(|_| DictationError::FixtureUnsupported)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 16_000
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || reader.duration() > 60 * 16_000
    {
        return Err(DictationError::FixtureUnsupported);
    }
    let mut chunk = Zeroizing::new(Vec::with_capacity(SAMPLE_RATE));
    for sample in reader.samples::<i16>() {
        chunk.push(f32::from(sample.map_err(|_| DictationError::FixtureUnsupported)?) / 32768.0);
        if chunk.len() == SAMPLE_RATE {
            pipeline.push(&chunk, emit)?;
            chunk.clear();
        }
    }
    pipeline.push(&chunk, emit)?;
    pipeline.finish(emit)
}

fn live(
    pipeline: &mut Pipeline<WhisperDecoder>,
    emit: &mut impl FnMut(TranscriptUpdate) -> Result<(), DictationError>,
) -> Result<(), DictationError> {
    let (sender, receiver) = mpsc::sync_channel::<String>(8);
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let mut command = String::new();
            // Bound console input independently of an unterminated line.
            let result = std::io::Read::by_ref(&mut input)
                .take(64)
                .read_line(&mut command);
            let done = !matches!(result, Ok(1..=63));
            if sender
                .send(if done {
                    "stop".into()
                } else {
                    command.trim().into()
                })
                .is_err()
                || done
            {
                break;
            }
        }
    });
    let mut capture = Some(Capture::start()?);
    eprintln!("recording; commands: pause, resume, flush, stop (EOF also stops)");
    loop {
        match receiver.try_recv().as_deref() {
            Ok("pause") => {
                if let Some(active) = capture.take() {
                    active.finish(pipeline, emit)?;
                }
                eprintln!("paused; microphone released");
            }
            Ok("resume") => {
                if capture.is_none() {
                    capture = Some(Capture::start()?);
                }
                eprintln!("recording");
            }
            Ok("flush") => {
                pipeline.finish(emit)?;
            }
            Ok("stop") => {
                if let Some(active) = capture.take() {
                    active.finish(pipeline, emit)?;
                }
                eprintln!("stopped; microphone released");
                return Ok(());
            }
            Ok(_) => eprintln!("commands: pause, resume, flush, stop"),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Some(active) = &mut capture {
            active.poll(pipeline, emit)?;
        }
        thread::sleep(Duration::from_millis(2));
    }
}
