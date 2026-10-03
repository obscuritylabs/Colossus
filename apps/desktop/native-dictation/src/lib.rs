//! Opt-in, offline Desktop dictation feasibility probe.
//!
//! This crate is not linked into the production Desktop application. Its probe
//! owns audio and model bytes; only bounded transcript revisions leave that
//! boundary. Hardware acceptance is required before enabling product commands.

#[cfg(feature = "probe")]
mod capture;
mod contract;
#[cfg(feature = "probe")]
mod decoder;
#[cfg(any(feature = "probe", test))]
mod model;
#[cfg(any(feature = "probe", test))]
mod pipeline;
#[cfg(feature = "probe")]
mod probe;
#[cfg(feature = "probe")]
mod resample;
#[cfg(feature = "probe")]
mod worker;

pub use contract::{DictationError, TranscriptUpdate};
#[cfg(feature = "probe")]
pub use probe::run;
