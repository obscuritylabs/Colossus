//! Generated Rust messages and gRPC service contracts for the public Colossus API.
//!
//! This crate intentionally contains no runtime, policy, authentication, persistence,
//! or transport configuration. Callers must validate all generated request values at
//! an authenticated application boundary before invoking Colossus services.

/// Initial unstable public application API.
#[allow(clippy::doc_markdown)]
pub mod v1alpha1 {
    tonic::include_proto!("colossus.api.v1alpha1");
}

/// Standard rich gRPC status envelope used by `grpc-status-details-bin`.
pub mod google_rpc {
    tonic::include_proto!("google.rpc");
}

/// Encoded descriptors for compatibility checks and explicitly authorized reflection.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("colossus_api_descriptor");

#[cfg(test)]
mod tests;
