//! Caller-bound browser lifecycle and single-writer coordination.
//!
//! Runtime adapters consume effect permits before entering this service. Engine
//! adapters enforce guest/network containment. This service does neither and never
//! releases page observations into model context.

mod coordinator;
mod error;
mod state;
mod validation;

pub use coordinator::BrowserCoordinator;
pub use error::BrowserError;
