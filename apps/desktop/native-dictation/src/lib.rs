//! Opt-in, offline Desktop dictation feasibility probe.
//!
//! The standalone probe and opt-in Desktop preview own audio and model bytes;
//! only bounded transcript revisions and loudness bytes leave this native boundary.
//! Hardware acceptance is required before enabling dictation in ordinary releases.

#[cfg(feature = "probe")]
mod capture;
mod contract;
#[cfg(feature = "probe")]
mod decoder;
#[cfg(feature = "probe")]
mod installed_model;
#[cfg(feature = "probe")]
mod meter;
#[cfg(any(feature = "probe", test))]
mod model;
#[cfg(any(feature = "probe", test))]
mod pipeline;
#[cfg(feature = "probe")]
mod probe;
#[cfg(feature = "probe")]
mod resample;
#[cfg(feature = "probe")]
mod session;
#[cfg(feature = "probe")]
mod worker;

pub use contract::{DictationError, TranscriptUpdate};
#[cfg(feature = "probe")]
pub use installed_model::InstalledModel;
#[cfg(feature = "probe")]
pub use probe::run;
#[cfg(feature = "probe")]
pub use session::{Session, SessionAction, SessionCancellation, SessionEvent, SessionPhase};
#[cfg(feature = "probe")]
pub use worker::run_if_requested;
