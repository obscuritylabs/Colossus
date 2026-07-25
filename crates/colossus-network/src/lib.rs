//! Shared outbound-network TLS configuration for Colossus-owned clients.

mod tls;

pub use tls::{AdditionalRootCertificates, TlsTrustError};
