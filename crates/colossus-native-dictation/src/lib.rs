//! Shared offline dictation for Desktop, the local TUI, and the standalone probe.
//! Audio and model bytes stay native; only bounded text revisions and loudness leave.

#[cfg(feature = "probe")]
mod capture;
mod catalog;
mod contract;
mod punctuation;
pub use punctuation::format_spoken_punctuation;
#[cfg(feature = "probe")]
mod decoder;
#[cfg(feature = "probe")]
mod devices;
#[cfg(feature = "download")]
mod download;
#[cfg(feature = "probe")]
mod installed_model;
#[cfg(feature = "probe")]
mod meter;
#[cfg(any(feature = "probe", test))]
mod model;
#[cfg(any(feature = "probe", test))]
mod pipeline;
#[cfg(feature = "probe")]
mod preferences;
#[cfg(feature = "probe")]
mod probe;
#[cfg(feature = "probe")]
mod resample;
#[cfg(feature = "probe")]
mod session;
#[cfg(feature = "probe")]
mod worker;

pub use catalog::ModelId;
pub use contract::{DictationError, TranscriptUpdate};
#[cfg(feature = "probe")]
pub use devices::{Microphone, microphones};
#[cfg(feature = "download")]
pub use download::{DownloadCancellation, download_model};
#[cfg(feature = "probe")]
pub use installed_model::InstalledModel;
#[cfg(feature = "probe")]
pub use preferences::{DictationPreferences, DictationSettings};
#[cfg(feature = "probe")]
pub use probe::run;
#[cfg(feature = "probe")]
pub use session::{Session, SessionAction, SessionCancellation, SessionEvent, SessionPhase};
#[cfg(feature = "probe")]
pub use worker::run_if_requested;
