use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(any(feature = "probe", test))]
pub(crate) const SAMPLE_RATE: usize = 16_000;
#[cfg(any(feature = "probe", test))]
pub(crate) const SEGMENT_SAMPLES: usize = SAMPLE_RATE * 6;
#[cfg(any(feature = "probe", test))]
pub(crate) const PARTIAL_SAMPLES: usize = SAMPLE_RATE * 2;
#[cfg(any(feature = "probe", test))]
pub(crate) const MAX_TEXT_BYTES: usize = 8 * 1024;

/// A replacement for one segment, never a text delta to append.
///
/// Order by `segment_id`, replace only with a greater `revision`, and settle
/// exactly once on `is_final`. A final empty string clears an earlier partial.
#[derive(Debug, Serialize)]
pub struct TranscriptUpdate {
    /// Monotonic within one probe process, including pause and resume.
    pub segment_id: u64,
    /// Monotonic within this segment.
    pub revision: u32,
    /// A settled segment will never receive another revision.
    pub is_final: bool,
    /// Complete replacement text, limited to 8 KiB of UTF-8.
    pub text: String,
    /// Audio duration, excluding padding added for a short final segment.
    pub audio_ms: u64,
    /// Time spent in this decode, excluding microphone and queue wait.
    pub inference_ms: u64,
}

/// Categorical failures exclude model paths, audio, transcripts and OS errors.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DictationError {
    /// A model transfer failed or was cancelled before publication.
    Download,
    /// Native preferences cannot be safely read or saved.
    Settings,
    /// The user has not enabled microphone dictation.
    Disabled,
    /// Invalid probe invocation.
    Arguments,
    /// A model could not be read as a bounded regular file.
    ModelUnavailable,
    /// The expected SHA-256 is malformed or differs from the installed model.
    ModelIntegrity,
    /// The verified model cannot be loaded by this runtime.
    ModelUnsupported,
    /// The default input device is absent.
    MicrophoneMissing,
    /// Permission, device configuration, or capture failed.
    CaptureUnavailable,
    /// Input format is outside the probe's resource bounds.
    CaptureUnsupported,
    /// Capture outpaced inference. Stop instead of silently dropping speech.
    CaptureOverrun,
    /// Local inference failed or exceeded its ten-second deadline.
    Inference,
    /// Transcript output exceeded the bounded segment contract.
    TranscriptLimit,
    /// A fixture is not a bounded 16 kHz, mono, signed 16-bit WAV.
    FixtureUnsupported,
    /// Probe input or output was closed or failed.
    Console,
}

impl fmt::Display for DictationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Download => "model download failed or cancelled; check the connection and retry",
            Self::Settings => "dictation settings are unavailable or invalid",
            Self::Disabled => "enable dictation in Settings before recording",
            Self::Arguments => "usage: dictation-probe MODEL SHA256 [--show-text] [--wav FILE]",
            Self::ModelUnavailable => {
                "model unavailable: select a regular model file of at most 200 MiB"
            }
            Self::ModelIntegrity => {
                "model integrity check failed: obtain the expected SHA-256 from trusted provenance"
            }
            Self::ModelUnsupported => "verified model is unsupported by this local runtime",
            Self::MicrophoneMissing => {
                "microphone unavailable: reconnect it or choose another input in Settings"
            }
            Self::CaptureUnavailable => {
                "microphone unavailable: check OS permission and input device settings"
            }
            Self::CaptureUnsupported => "microphone format is unsupported by this local recorder",
            Self::CaptureOverrun => {
                "capture exceeded its bounded queue: recording stopped; try a smaller local model"
            }
            Self::Inference => "local inference failed or timed out: recording stopped",
            Self::TranscriptLimit => "transcript segment exceeded 8 KiB: recording stopped",
            Self::FixtureUnsupported => {
                "fixture must be a mono 16 kHz signed 16-bit WAV of at most 60 seconds"
            }
            Self::Console => "probe console closed or failed",
        })
    }
}

impl std::error::Error for DictationError {}
