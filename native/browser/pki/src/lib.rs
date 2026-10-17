//! Shared public-certificate validation. No operating-system, key, or serialization APIs.
#![forbid(unsafe_code)]

mod validation;
pub use validation::{ValidationError, ca_der, fingerprint, identity_validity};

#[cfg(test)]
mod tests;
