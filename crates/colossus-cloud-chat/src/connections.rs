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
    /// First-party web search authentication, separate from model and MCP keys.
    Search,
}
impl ChatConnectionKind {
    fn purpose(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Mcp => "mcp",
            Self::Search => "search",
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
    catalog_generation: Option<u64>,
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
        // Match declared Core connections: plaintext transport is confined to
        // explicit loopback endpoints on the Control Plane host.
        let loopback = url.scheme() == "http"
            && match url.host() {
                Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                None => false,
            };
        if (url.scheme() != "https" && !loopback)
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
            catalog_generation: None,
        }))
    }

    /// Bind a connection inherited from an instance definition to its exact identity generation.
    pub fn with_catalog_generation(mut self, generation: u64) -> Result<Self, ChatError> {
        if generation == 0 {
            return Err(ChatError::InvalidConfiguration);
        }
        self.catalog_generation = Some(generation);
        Ok(self)
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
        if let Some(generation) = self.catalog_generation {
            digest.update(b"instance-catalog-generation");
            digest.update(generation.to_be_bytes());
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
    fn declared_loopback_connections_support_every_credential_scope_and_purpose() {
        for endpoint in [
            "https://api.example/v1",
            "http://localhost:8080/v1",
            "http://127.0.0.1:8080/v1",
            "http://127.42.9.3:8080/v1",
            "http://[::1]:8080/v1",
        ] {
            for kind in [
                ChatConnectionKind::Provider,
                ChatConnectionKind::Mcp,
                ChatConnectionKind::Search,
            ] {
                for authentication in [
                    ChatAuthentication::None,
                    ChatAuthentication::Personal,
                    ChatAuthentication::Shared,
                ] {
                    let result = ChatConnectionBinding::new(
                        "project",
                        "alice",
                        kind,
                        "connection",
                        1,
                        endpoint,
                        authentication,
                    )
                    .unwrap();
                    assert_eq!(result.is_some(), authentication != ChatAuthentication::None);
                }
            }
        }
    }

    #[test]
    fn loopback_credentials_remain_bound_to_exact_destination_user_and_purpose() {
        let record = |user, kind, endpoint| {
            ChatConnectionBinding::new(
                "project",
                user,
                kind,
                "connection",
                1,
                endpoint,
                ChatAuthentication::Personal,
            )
            .unwrap()
            .unwrap()
            .record_id()
        };
        let first = record(
            "alice",
            ChatConnectionKind::Provider,
            "http://127.0.0.1:8080/v1",
        );
        for candidate in [
            record(
                "bob",
                ChatConnectionKind::Provider,
                "http://127.0.0.1:8080/v1",
            ),
            record("alice", ChatConnectionKind::Mcp, "http://127.0.0.1:8080/v1"),
            record(
                "alice",
                ChatConnectionKind::Provider,
                "http://127.0.0.1:8081/v1",
            ),
            record(
                "alice",
                ChatConnectionKind::Provider,
                "http://127.0.0.1:8080/v2",
            ),
        ] {
            assert_ne!(first, candidate);
        }
    }

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
            "http://192.168.1.1/",
            "http://localhost.example/",
            "http://127.0.0.1.example/",
            "http://[::ffff:127.0.0.1]/",
            "http://user:secret@localhost/",
            "http://localhost/?key=secret",
            "http://localhost/?",
            "http://localhost/#fragment",
            "http://localhost/#",
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
