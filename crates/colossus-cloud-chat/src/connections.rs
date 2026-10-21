use crate::ChatError;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Explicit credential policy for a project-managed connector.
///
/// Setting a shared credential is a deliberate administrative choice. Failure to load,
/// decrypt or refresh a personal credential never selects a shared identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatAuthentication {
    /// Each authenticated user connects their own account.
    #[default]
    Personal,
    /// An administrator explicitly configured a project service account.
    Shared,
    /// This reviewed connection requires no credential.
    None,
}

/// Independent credential purpose; matching profile and MCP names never share a record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatConnectionKind {
    /// Model-provider authentication.
    Provider,
    /// Remote MCP-server authentication.
    Mcp,
}
impl ChatConnectionKind {
    fn purpose(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Mcp => "mcp",
        }
    }
}

/// Credential identity selected by trusted project configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatCredentialScope {
    /// Private to the exact project and authenticated account.
    Personal {
        /// Exact authenticated project namespace.
        project: String,
        /// Stable authenticated account identity.
        user: String,
    },
    /// Explicitly shared within this project only.
    Project {
        /// Exact project namespace under explicit shared-account policy.
        project: String,
    },
}

/// Immutable authorization and credential-destination binding for one connection.
/// No raw key, token, callback code or password is contained in this record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatConnectionBinding {
    scope: ChatCredentialScope,
    kind: ChatConnectionKind,
    connection: String,
    generation: u64,
    endpoint: String,
}
impl ChatConnectionBinding {
    /// Bind the authenticated user and current project connection policy before lookup.
    pub fn new(
        project: &str,
        user: &str,
        kind: ChatConnectionKind,
        connection: &str,
        generation: u64,
        endpoint: &str,
        authentication: ChatAuthentication,
    ) -> Result<Option<Self>, ChatError> {
        for value in [project, user, connection] {
            if !valid_id(value) {
                return Err(ChatError::InvalidConfiguration);
            }
        }
        if generation == 0 {
            return Err(ChatError::InvalidConfiguration);
        }
        let url = url::Url::parse(endpoint).map_err(|_| ChatError::InvalidConfiguration)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
        {
            return Err(ChatError::InvalidConfiguration);
        }
        let scope = match authentication {
            ChatAuthentication::Personal => ChatCredentialScope::Personal {
                project: project.into(),
                user: user.into(),
            },
            ChatAuthentication::Shared => ChatCredentialScope::Project {
                project: project.into(),
            },
            ChatAuthentication::None => return Ok(None),
        };
        Ok(Some(Self {
            scope,
            kind,
            connection: connection.into(),
            generation,
            endpoint: url.into(),
        }))
    }

    /// Nonsecret scope metadata for connection status presentation.
    pub const fn scope(&self) -> &ChatCredentialScope {
        &self.scope
    }

    /// Opaque key bound to tenant, endpoint and connection generation.
    pub fn record_id(&self) -> String {
        let owner = match &self.scope {
            ChatCredentialScope::Personal { project, user } => {
                ("personal", project.as_str(), user.as_str())
            }
            ChatCredentialScope::Project { project } => ("project", project.as_str(), ""),
        };
        // Separate length-delimited values prevent ambiguous identity concatenations.
        let mut digest = Sha256::new();
        for value in [
            "colossus-cloud-chat-credential-v1",
            owner.0,
            owner.1,
            owner.2,
            self.kind.purpose(),
            &self.connection,
            &self.generation.to_string(),
            &self.endpoint,
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        hex::encode(digest.finalize())
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'@')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        project: &str,
        user: &str,
        generation: u64,
        endpoint: &str,
        auth: ChatAuthentication,
    ) -> ChatConnectionBinding {
        ChatConnectionBinding::new(
            project,
            user,
            ChatConnectionKind::Mcp,
            "connector",
            generation,
            endpoint,
            auth,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn personal_is_default_and_binds_tenant_user_generation_and_destination() {
        assert_eq!(ChatAuthentication::default(), ChatAuthentication::Personal);
        let first = binding(
            "project",
            "alice",
            1,
            "https://mcp.example/api",
            ChatAuthentication::Personal,
        )
        .record_id();
        for candidate in [
            binding(
                "other",
                "alice",
                1,
                "https://mcp.example/api",
                ChatAuthentication::Personal,
            ),
            binding(
                "project",
                "bob",
                1,
                "https://mcp.example/api",
                ChatAuthentication::Personal,
            ),
            binding(
                "project",
                "alice",
                2,
                "https://mcp.example/api",
                ChatAuthentication::Personal,
            ),
            binding(
                "project",
                "alice",
                1,
                "https://other.example/api",
                ChatAuthentication::Personal,
            ),
            binding(
                "project",
                "alice",
                1,
                "https://mcp.example/api",
                ChatAuthentication::Shared,
            ),
        ] {
            assert_ne!(first, candidate.record_id());
        }
    }

    #[test]
    fn explicit_shared_binding_stays_within_project() {
        let alice = binding(
            "project",
            "alice",
            1,
            "https://mcp.example/api",
            ChatAuthentication::Shared,
        );
        let bob = binding(
            "project",
            "bob",
            1,
            "https://mcp.example/api",
            ChatAuthentication::Shared,
        );
        assert_eq!(alice.record_id(), bob.record_id());
        assert_ne!(
            alice.record_id(),
            binding(
                "other",
                "alice",
                1,
                "https://mcp.example/api",
                ChatAuthentication::Shared
            )
            .record_id()
        );
    }
}

#[cfg(test)]
mod purpose_tests {
    use super::*;

    #[test]
    fn provider_and_mcp_authentication_never_share_the_same_record() {
        let record = |kind| {
            ChatConnectionBinding::new(
                "project",
                "alice",
                kind,
                "identical-name",
                1,
                "https://same.example/api",
                ChatAuthentication::Personal,
            )
            .unwrap()
            .unwrap()
            .record_id()
        };
        assert_ne!(
            record(ChatConnectionKind::Provider),
            record(ChatConnectionKind::Mcp)
        );
    }

    #[test]
    fn credential_binding_rejects_unsafe_destinations_and_invalid_tenant_ids() {
        for endpoint in [
            "http://api.example/",
            "https://user:secret@api.example/",
            "https://api.example/?key=secret",
            "https://api.example/#fragment",
        ] {
            assert!(
                ChatConnectionBinding::new(
                    "project",
                    "alice",
                    ChatConnectionKind::Provider,
                    "provider",
                    1,
                    endpoint,
                    ChatAuthentication::Personal
                )
                .is_err()
            );
        }
        for value in ["../project", "", "..", "alice\n"] {
            assert!(
                ChatConnectionBinding::new(
                    value,
                    "alice",
                    ChatConnectionKind::Mcp,
                    "mcp",
                    1,
                    "https://api.example/",
                    ChatAuthentication::Personal
                )
                .is_err()
            );
        }
    }
}
