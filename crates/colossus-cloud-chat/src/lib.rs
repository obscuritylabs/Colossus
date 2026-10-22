//! Hosted chat composition with project-managed connections and user-owned execution.
//!
//! HTTP authentication and project membership belong to the Control Plane. This host
//! receives an already authenticated tenant binding, composes the ordinary runtime and
//! exposes only its caller-bound public SDK. It never provides a user environment.

mod connections;
mod credential_store;
mod credentials;
mod files;
mod host;
mod shared_keys;
pub use shared_keys::{
    SharedAnchorSnapshot, SharedAnchorUpdate, SharedChatKeyConfig, SharedChatKeys,
    shared_chat_keys, validate_shared_anchor_advance,
};

pub use connections::{
    ChatAuthentication, ChatConnectionBinding, ChatConnectionKind, ChatCredentialScope,
};
pub use credential_store::{CredentialStore, StoredCredentialKind, StoredCredentialMetadata};
pub use credentials::ChatCredentialResolver;
/// General Hosted execution composition; ChatRuntime remains a compatibility name.
pub use host::ChatRuntime as HostedRuntime;
pub use host::{ChatRuntime, ChatRuntimeOptions, ChatSharedRuntime};

/// Categorical hosted-chat failure. Private configuration and credential values are omitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatError {
    /// The trusted host supplied an invalid tenant, connection or runtime configuration.
    InvalidConfiguration,
    /// Protected state or credential custody is unavailable.
    StateUnavailable,
    /// A credential is absent or could not be resolved under its exact binding.
    AuthenticationRequired,
    /// Runtime composition or execution is unavailable.
    RuntimeUnavailable,
}
impl std::fmt::Display for ChatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "Hosted chat configuration is invalid.",
            Self::StateUnavailable => "Hosted chat protected state is unavailable.",
            Self::AuthenticationRequired => "This connection requires authentication.",
            Self::RuntimeUnavailable => "Hosted chat runtime is unavailable.",
        })
    }
}
impl std::error::Error for ChatError {}
