//! Shared outbound-network TLS configuration for Colossus-owned clients.

mod http;
mod tls;

pub use http::{
    PinnedHttpClientError, non_public_network_address, parse_host_ip, pinned_reqwest_client,
    resolve_destinations,
};
pub use tls::{AdditionalRootCertificates, TlsTrustError};
