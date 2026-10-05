//! Project-scoped cloud placement and released-data services over the canonical journal.
//!
//! This crate never constructs a runtime, executes tools, or grants local authority.
//! Transports authenticate callers and nodes before entering these services.

mod enrollment;
mod error;
mod identity;
mod repository;
mod types;
mod validation;

pub use enrollment::{CertificateRedemption, Enrollment, RenewalIdentity};

pub use error::{CloudError, CloudResult};
pub use identity::{CloudCaller, CloudPermission};
pub use repository::CloudRepository;
pub use types::*;
pub use validation::validate_identifier;

#[cfg(test)]
mod tests;
