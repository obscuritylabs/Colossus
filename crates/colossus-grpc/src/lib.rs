//! Authenticated loopback gRPC transport for the public Colossus application API.
//!
//! This crate owns transport authentication and error translation. Runtime policy and
//! durable application behavior remain behind the transport-neutral `colossus-api`
//! ports.

#![allow(clippy::missing_errors_doc)]

mod agent_run;
mod artifact;
mod auth;
mod endpoint;
mod journal_credentials;
mod request_guard;
mod server;
mod status;
mod system;
mod tls_identity;

pub use agent_run::{AgentRunServiceAdapter, MAX_ACTIVE_WATCH_STREAMS};
pub use artifact::ArtifactServiceAdapter;
pub use auth::{
    ApplicationGrant, AuthenticationError, AuthenticationInterceptor, CredentialAuthenticator,
    CredentialRecord, CredentialRepository, CredentialStoreError, InMemoryCredentialRepository,
    IssuedCredential,
};
pub use endpoint::{
    ENDPOINT_DESCRIPTOR_SCHEMA_VERSION, EndpointDescriptor, EndpointDescriptorError,
    EndpointDescriptorStorage, NativeEndpointDescriptorStorage, PUBLIC_API_VERSION,
    read_endpoint_certificate, read_endpoint_certificate_with, read_endpoint_descriptor,
    read_endpoint_descriptor_with, validate_endpoint_certificate_pem, write_endpoint_certificate,
    write_endpoint_certificate_with, write_endpoint_descriptor, write_endpoint_descriptor_with,
};
pub use journal_credentials::JournalCredentialRepository;
pub use server::{BoundPublicGrpcServer, PublicGrpcServerError, RESERVED_UNARY_REQUEST_HEADROOM};
pub use status::api_status;
pub use system::{
    FixedReadiness, PublicReadiness, ReadinessProvider, SystemMetadata, SystemServiceAdapter,
};
pub use tls_identity::{TlsIdentity, TlsIdentityError, TlsKeySeed};
