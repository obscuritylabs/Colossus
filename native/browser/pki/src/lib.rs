//! Shared public-certificate validation. No operating-system, key, or serialization APIs.
#![forbid(unsafe_code)]

mod validation;
pub use validation::{ValidationError, ca_der, fingerprint, identity_validity};

mod label;
pub use label::identity_label;

#[cfg(test)]
mod tests;
