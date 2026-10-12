//! Reviewed opaque native registry references admitted for Core's exact owner.
//! No certificate container, password, key, path, signer or OS-store API exists.
use colossus_contracts::{BrowserOrigin, BrowserScope, BrowserSessionBinding};
use colossus_ports::{BrowserDriverError, BrowserDriverOpenRequest};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Opaque reference in the independently signed native broker registry.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MacosBrowserPkiId(String);
impl MacosBrowserPkiId {
    /// Validate an opaque256-bit ID; no filename or user-supplied path is derived.
    pub fn new(value: String) -> Result<Self, BrowserDriverError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || value.bytes().all(|b| b == b'0')
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(Self(value))
    }
    /// Public registry reference only, never a secret or native key handle.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Explicit native identity review consent using existing application scope types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacosBrowserPkiScopeAuthorization {
    /// One already authenticated conversation or workflow.
    Exact(BrowserScope),
    /// Explicit consent for current/future conversations in the same application.
    FutureConversations,
    /// Explicit consent for current/future workflow executions in that application.
    FutureWorkflows,
    /// Explicit consent for both application scope kinds.
    FutureConversationsAndWorkflows,
}
impl MacosBrowserPkiScopeAuthorization {
    fn admits(&self, scope: &BrowserScope) -> bool {
        match (self, scope) {
            (Self::Exact(expected), actual) => expected == actual,
            (Self::FutureConversations, BrowserScope::Conversation { .. })
            | (Self::FutureWorkflows, BrowserScope::Workflow { .. })
            | (Self::FutureConversationsAndWorkflows, _) => true,
            _ => false,
        }
    }
}
/// Trusted local import review metadata; never model arguments or runtime YAML.
pub struct MacosBrowserPkiAuthorization {
    /// Canonical workspace identity reviewed at native enrollment.
    pub workspace_id: String,
    /// Authenticated application that owns this enrollment.
    pub application_id: String,
    /// Explicit review scope, including any separately authorized future scopes.
    pub scope: MacosBrowserPkiScopeAuthorization,
    /// Exact HTTPS origins permitted to use the native trust/identity.
    pub origins: Vec<BrowserOrigin>,
}
/// Reviewed public client selection. The native broker retains all secret inputs.
#[derive(Clone)]
pub struct MacosBrowserClientIdentity {
    /// Exact reviewed TLS request origin.
    pub origin: BrowserOrigin,
    /// Opaque independently signed native registry reference.
    pub identity_id: MacosBrowserPkiId,
    /// Reviewed exact public leaf fingerprint; no certificate/key bytes.
    pub fingerprint_sha256: [u8; 32],
}
/// One immutable review registration, containing public references only.
pub struct MacosBrowserPkiRegistration {
    authorization: MacosBrowserPkiAuthorization,
    cas: Vec<MacosBrowserPkiId>,
    identities: Vec<MacosBrowserClientIdentity>,
}
impl MacosBrowserPkiRegistration {
    /// Retain explicit owner/origin consent without receiving native secrets.
    pub fn new(
        authorization: MacosBrowserPkiAuthorization,
        cas: Vec<MacosBrowserPkiId>,
        identities: Vec<MacosBrowserClientIdentity>,
    ) -> Result<Self, BrowserDriverError> {
        if !valid_id(&authorization.workspace_id)
            || !valid_id(&authorization.application_id)
            || authorization.origins.is_empty()
            || authorization.origins.len() > 8
            || cas.len() > 8
            || identities.len() > 8
            || (cas.is_empty() && identities.is_empty())
        {
            return Err(BrowserDriverError::Denied);
        }
        if let MacosBrowserPkiScopeAuthorization::Exact(
            BrowserScope::Conversation { id } | BrowserScope::Workflow { id },
        ) = &authorization.scope
            && !valid_id(id)
        {
            return Err(BrowserDriverError::Denied);
        }
        if authorization
            .origins
            .iter()
            .enumerate()
            .any(|(index, origin)| {
                !origin.as_str().starts_with("https://")
                    || authorization.origins[..index].contains(origin)
            })
            || cas.iter().collect::<BTreeSet<_>>().len() != cas.len()
        {
            return Err(BrowserDriverError::Denied);
        }
        for (index, identity) in identities.iter().enumerate() {
            if identity.fingerprint_sha256 == [0; 32]
                || !authorization.origins.contains(&identity.origin)
                || identities[..index]
                    .iter()
                    .any(|prior| prior.origin == identity.origin)
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        Ok(Self {
            authorization,
            cas,
            identities,
        })
    }
}
/// Concrete bounded per-request registry; references do not themselves prove custody.
pub struct MacosBrowserPkiRegistry {
    registrations: Vec<MacosBrowserPkiRegistration>,
}
impl MacosBrowserPkiRegistry {
    /// Retain up to32 explicitly reviewed public registrations.
    pub fn new(
        registrations: Vec<MacosBrowserPkiRegistration>,
    ) -> Result<Self, BrowserDriverError> {
        if registrations.is_empty() || registrations.len() > 32 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        Ok(Self { registrations })
    }
    pub(super) fn bind(
        self,
        catalog: &NativeCatalog,
    ) -> Result<AdmittedRegistry, BrowserDriverError> {
        for registration in &self.registrations {
            if registration.cas.iter().any(|id| {
                catalog.cas.get(id).is_none_or(|origins| {
                    !registration
                        .authorization
                        .origins
                        .iter()
                        .all(|origin| origins.contains(origin))
                })
            }) || registration.identities.iter().any(|identity| {
                catalog
                    .identities
                    .get(&identity.identity_id)
                    .is_none_or(|native| {
                        native.fingerprint != identity.fingerprint_sha256
                            || !native.origins.contains(&identity.origin)
                    })
            }) {
                return Err(BrowserDriverError::Denied);
            }
        }
        Ok(AdmittedRegistry(self))
    }
}
// Minted only by the private Installation after its independent signed inventory
// verification. A registration/caller cannot nominate the expected catalog.
pub(super) struct NativeCatalog {
    pub cas: BTreeMap<MacosBrowserPkiId, Vec<BrowserOrigin>>,
    pub identities: BTreeMap<MacosBrowserPkiId, NativeIdentity>,
}
pub(super) struct NativeIdentity {
    pub fingerprint: [u8; 32],
    pub origins: Vec<BrowserOrigin>,
}
pub(super) struct AdmittedRegistry(MacosBrowserPkiRegistry);
#[derive(Clone)]
pub(super) struct Selection {
    pub binding: BrowserSessionBinding,
    pub cas: Vec<MacosBrowserPkiId>,
    pub identities: Vec<MacosBrowserClientIdentity>,
}
impl AdmittedRegistry {
    pub fn has_private_ca(&self) -> bool {
        self.0.registrations.iter().any(|r| !r.cas.is_empty())
    }
    pub fn has_client_identities(&self) -> bool {
        self.0
            .registrations
            .iter()
            .any(|r| !r.identities.is_empty())
    }
    pub fn enroll(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<Option<Selection>, BrowserDriverError> {
        let (BrowserScope::Conversation { id } | BrowserScope::Workflow { id }) =
            &request.binding.scope;
        if !valid_id(&request.binding.runtime_id)
            || !valid_id(&request.binding.workspace_id)
            || !valid_id(&request.binding.application_id)
            || !valid_id(id)
            || request.options.allowed_origins.is_empty()
            || request.options.allowed_origins.len() > 8
        {
            return Err(BrowserDriverError::Denied);
        }
        let mut selected = None;
        for registration in &self.0.registrations {
            let authorization = &registration.authorization;
            if request.binding.workspace_id != authorization.workspace_id
                || request.binding.application_id != authorization.application_id
                || !authorization.scope.admits(&request.binding.scope)
                || !request
                    .options
                    .allowed_origins
                    .iter()
                    .any(|origin| authorization.origins.contains(origin))
            {
                continue;
            }
            if !request
                .options
                .allowed_origins
                .iter()
                .all(|origin| authorization.origins.contains(origin))
                || selected.is_some()
            {
                return Err(BrowserDriverError::Denied);
            }
            selected = Some(registration);
        }
        Ok(selected.map(|registration| Selection {
            binding: request.binding.clone(),
            cas: registration.cas.clone(),
            identities: registration
                .identities
                .iter()
                .filter(|identity| request.options.allowed_origins.contains(&identity.origin))
                .cloned()
                .collect(),
        }))
    }
}
impl Selection {
    pub fn tls(
        &self,
    ) -> Result<colossus_native_browser_tls_broker::PkiSelection, BrowserDriverError> {
        let decode = |id: &str| {
            hex::decode(id)
                .map_err(|_| BrowserDriverError::Denied)?
                .try_into()
                .map_err(|_| BrowserDriverError::Denied)
        };
        if self.identities.len() > 1 {
            return Err(BrowserDriverError::Denied);
        }
        let mut digest = Sha256::new();
        digest.update(b"colossus-macos-reviewed-pki-core-scope-v1\0");
        let mut text = |value: &str| {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        };
        text(&self.binding.runtime_id);
        text(&self.binding.workspace_id);
        text(&self.binding.application_id);
        match &self.binding.scope {
            BrowserScope::Conversation { id } => {
                text("conversation");
                text(id);
            }
            BrowserScope::Workflow { id } => {
                text("workflow");
                text(id);
            }
        }
        let scope_binding = digest.finalize().into();
        Ok(colossus_native_browser_tls_broker::PkiSelection {
            scope_binding,
            ca_ids: self
                .cas
                .iter()
                .map(|id| decode(id.as_str()))
                .collect::<Result<Vec<_>, _>>()?,
            client: self
                .identities
                .first()
                .map(|identity| {
                    Ok::<_, BrowserDriverError>(
                        colossus_native_browser_tls_broker::ReviewedClientIdentity {
                            identity_id: decode(identity.identity_id.as_str())?,
                            leaf_sha256: identity.fingerprint_sha256,
                        },
                    )
                })
                .transpose()?,
        })
    }
}
pub(super) fn from_metadata(
    metadata: &colossus_native_browser_tls_broker::registry_metadata::RegistryMetadata,
) -> Result<AdmittedRegistry, BrowserDriverError> {
    use colossus_native_browser_tls_broker::registry_metadata::ScopeConsent;
    let origins = |values: &[String]| {
        values
            .iter()
            .map(|value| BrowserOrigin::parse(value).map_err(|_| BrowserDriverError::Denied))
            .collect::<Result<Vec<_>, _>>()
    };
    let fingerprint = |value: &str| {
        hex::decode(value)
            .map_err(|_| BrowserDriverError::Denied)?
            .try_into()
            .map_err(|_| BrowserDriverError::Denied)
    };
    let catalog = NativeCatalog {
        cas: metadata
            .cas
            .iter()
            .map(|ca| {
                Ok((
                    MacosBrowserPkiId::new(ca.id.clone())?,
                    origins(&ca.origins)?,
                ))
            })
            .collect::<Result<_, BrowserDriverError>>()?,
        identities: metadata
            .identities
            .iter()
            .map(|client| {
                Ok((
                    MacosBrowserPkiId::new(client.id.clone())?,
                    NativeIdentity {
                        fingerprint: fingerprint(&client.leaf_sha256)?,
                        origins: origins(&client.origins)?,
                    },
                ))
            })
            .collect::<Result<_, BrowserDriverError>>()?,
    };
    let registrations = metadata
        .reviews
        .iter()
        .map(|review| {
            let scope = match &review.scope {
                ScopeConsent::Conversation { id } => {
                    MacosBrowserPkiScopeAuthorization::Exact(BrowserScope::Conversation {
                        id: id.clone(),
                    })
                }
                ScopeConsent::Workflow { id } => {
                    MacosBrowserPkiScopeAuthorization::Exact(BrowserScope::Workflow {
                        id: id.clone(),
                    })
                }
                ScopeConsent::FutureConversations {} => {
                    MacosBrowserPkiScopeAuthorization::FutureConversations
                }
                ScopeConsent::FutureWorkflows {} => {
                    MacosBrowserPkiScopeAuthorization::FutureWorkflows
                }
                ScopeConsent::FutureConversationsAndWorkflows {} => {
                    MacosBrowserPkiScopeAuthorization::FutureConversationsAndWorkflows
                }
            };
            let approved = origins(&review.origins)?;
            let identities = if let Some(id) = &review.client_id {
                let client = metadata
                    .identities
                    .iter()
                    .find(|client| client.id == *id)
                    .ok_or(BrowserDriverError::Denied)?;
                approved
                    .iter()
                    .map(|origin| {
                        Ok(MacosBrowserClientIdentity {
                            origin: origin.clone(),
                            identity_id: MacosBrowserPkiId::new(id.clone())?,
                            fingerprint_sha256: fingerprint(&client.leaf_sha256)?,
                        })
                    })
                    .collect::<Result<_, BrowserDriverError>>()?
            } else {
                Vec::new()
            };
            MacosBrowserPkiRegistration::new(
                MacosBrowserPkiAuthorization {
                    workspace_id: review.workspace_id.clone(),
                    application_id: review.application_id.clone(),
                    scope,
                    origins: approved,
                },
                review
                    .ca_ids
                    .iter()
                    .map(|id| MacosBrowserPkiId::new(id.clone()))
                    .collect::<Result<_, _>>()?,
                identities,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    MacosBrowserPkiRegistry::new(registrations)?.bind(&catalog)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}
#[cfg(test)]
#[path = "pki/tests.rs"]
mod tests;
