//! Independently versioned, bounded cloud connection messages and released DTOs.

mod messages;
mod resources;
pub use messages::*;
pub use resources::*;

/// Generated Protobuf connection envelope. Authentication is outside the payload.
pub mod v1alpha1 {
    #![allow(missing_docs)]
    tonic::include_proto!("colossus.cloud.v1alpha1");
}

#[cfg(test)]
mod tests;
