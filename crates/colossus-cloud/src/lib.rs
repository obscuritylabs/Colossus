//! Project-scoped cloud placement and released-data services over cloud-owned relational storage.
//!
//! This crate never constructs a runtime, executes tools, or grants local authority.
//! Transports authenticate callers and nodes before entering these services.

mod accounts;
mod enrollment;
mod error;
mod identity;
/// Read-only controller operational projections.
pub mod observability;
mod repository;
/// Audited public display settings and read-only policy expectations.
pub mod settings;
/// Cloud-owned persistence and coordination contracts.
pub mod storage;
mod types;
mod validation;

pub use accounts::*;
pub use enrollment::{CertificateRedemption, Enrollment, RenewalIdentity};

pub use error::{CloudError, CloudResult};
pub use identity::{CloudCaller, CloudPermission};
pub use repository::CloudRepository;
pub use types::*;
pub use validation::validate_identifier;

#[cfg(test)]
mod tests;
