//! Trusted import consent and exact per-allocation PKI enrollment; no model/serde surface.
use super::pki::{OciBrowserIdentity, OciBrowserPki, OciClientIdentityBinding, valid_fingerprint};
use colossus_contracts::{BrowserOrigin, BrowserScope};
use colossus_native_browser_pki::ca_der;
use colossus_ports::{BrowserDriverError, BrowserDriverOpenRequest};
use sha2::{Digest as _, Sha256};

/// Native import consent for existing or explicitly authorized future application scopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OciBrowserPkiScopeAuthorization {
    /// Only this immutable authenticated conversation or workflow identity.
    Exact(BrowserScope),
    /// Explicit import consent for current and future conversations in this application.
    FutureConversations,
    /// Explicit import consent for current and future workflow executions in this application.
    FutureWorkflows,
    /// Explicit import consent for both scope kinds within this application.
    FutureConversationsAndWorkflows,
}
impl OciBrowserPkiScopeAuthorization {
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

/// Public native consent metadata; never accepted from runtime YAML or model arguments.
pub struct OciBrowserPkiAuthorization {
    /// Exact canonical workspace identity approved at import.
    pub workspace_id: String,
    /// Exact authenticated application identity approved at import.
    pub application_id: String,
    /// Explicit current/future scope consent.
    pub scope: OciBrowserPkiScopeAuthorization,
    /// Every origin that may use this profile's trust or client identities; no wildcards.
    pub origins: Vec<BrowserOrigin>,
}

/// Retained native encrypted material and consent. Deliberately not Debug/Clone/Serialize.
pub struct OciBrowserPkiRegistration {
    authorization: OciBrowserPkiAuthorization,
    cas: Vec<Vec<u8>>,
    identities: Vec<OciBrowserIdentity>,
    bindings: Vec<OciClientIdentityBinding>,
}
impl OciBrowserPkiRegistration {
    /// Take explicitly reviewed import consent and native-owned secret lifetimes.
    pub fn new(
        authorization: OciBrowserPkiAuthorization,
        ca_certificates: Vec<Vec<u8>>,
        identities: Vec<OciBrowserIdentity>,
        bindings: Vec<OciClientIdentityBinding>,
    ) -> Result<Self, BrowserDriverError> {
        if !valid_id(&authorization.workspace_id)
            || !valid_id(&authorization.application_id)
            || authorization.origins.is_empty()
            || authorization.origins.len() > 32
            || ca_certificates.len() > 32
            || identities.len() > 32
            || bindings.len() > 32
            || (ca_certificates.is_empty() && identities.is_empty())
            || identities.is_empty() != bindings.is_empty()
        {
            return Err(BrowserDriverError::Denied);
        }
        if let OciBrowserPkiScopeAuthorization::Exact(scope) = &authorization.scope {
            let (BrowserScope::Conversation { id } | BrowserScope::Workflow { id }) = scope;
            if !valid_id(id) {
                return Err(BrowserDriverError::Denied);
            }
        }
        for (index, origin) in authorization.origins.iter().enumerate() {
            if !origin.as_str().starts_with("https://")
                || authorization.origins[..index].contains(origin)
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        let mut cas = Vec::with_capacity(ca_certificates.len());
        for certificate in ca_certificates {
            let der = ca_der(&certificate).map_err(|_| BrowserDriverError::Denied)?;
            if cas.contains(&der) {
                return Err(BrowserDriverError::Denied);
            }
            cas.push(der);
        }
        for (index, binding) in bindings.iter().enumerate() {
            if !authorization.origins.contains(&binding.origin)
                || !valid_fingerprint(&binding.fingerprint_sha256)
                || bindings[..index]
                    .iter()
                    .any(|other| other.origin == binding.origin)
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        let registration = Self {
            authorization,
            cas,
            identities,
            bindings,
        };
        if registration.material_bytes() > 32 * 1024 * 1024 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        Ok(registration)
    }
    fn material_bytes(&self) -> usize {
        self.cas.iter().map(Vec::len).sum::<usize>()
            + self
                .identities
                .iter()
                .map(OciBrowserIdentity::encrypted_bytes)
                .sum::<usize>()
    }
}

/// Called only by the trusted native factory with Core's complete authenticated open request.
pub trait OciBrowserPkiEnrollmentProvider: Send + Sync {
    /// Reviewed material is available; this does not establish artifact acceptance.
    fn has_private_ca(&self) -> bool;
    /// Reviewed encrypted identity material is available; this does not establish custody.
    fn has_client_identities(&self) -> bool;
    /// Public import policy hash; never includes password or private-key bytes.
    fn policy_digest(&self) -> [u8; 32];
    /// Make a fresh exact-owner lease, or withhold material outside native consent.
    fn enroll(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<Option<OciBrowserPki>, BrowserDriverError>;
}

/// Immutable bounded native import registry. Runtime IDs are bound only after Core mints them.
pub struct OciBrowserPkiRegistry {
    registrations: Vec<OciBrowserPkiRegistration>,
}
impl OciBrowserPkiRegistry {
    /// Retain at most32 imports and32MiB of encrypted/public certificate material.
    pub fn new(registrations: Vec<OciBrowserPkiRegistration>) -> Result<Self, BrowserDriverError> {
        if registrations.is_empty()
            || registrations.len() > 32
            || registrations
                .iter()
                .map(OciBrowserPkiRegistration::material_bytes)
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        Ok(Self { registrations })
    }
}
impl OciBrowserPkiEnrollmentProvider for OciBrowserPkiRegistry {
    fn has_private_ca(&self) -> bool {
        self.registrations.iter().any(|r| !r.cas.is_empty())
    }
    fn has_client_identities(&self) -> bool {
        self.registrations.iter().any(|r| !r.identities.is_empty())
    }
    fn policy_digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"colossus-browser-oci-native-import-v1\0");
        hash.update((self.registrations.len() as u64).to_be_bytes());
        for r in &self.registrations {
            text(&mut hash, &r.authorization.workspace_id);
            text(&mut hash, &r.authorization.application_id);
            match &r.authorization.scope {
                OciBrowserPkiScopeAuthorization::Exact(BrowserScope::Conversation { id }) => {
                    hash.update([0]);
                    text(&mut hash, id);
                }
                OciBrowserPkiScopeAuthorization::Exact(BrowserScope::Workflow { id }) => {
                    hash.update([1]);
                    text(&mut hash, id);
                }
                OciBrowserPkiScopeAuthorization::FutureConversations => hash.update([2]),
                OciBrowserPkiScopeAuthorization::FutureWorkflows => hash.update([3]),
                OciBrowserPkiScopeAuthorization::FutureConversationsAndWorkflows => {
                    hash.update([4])
                }
            }
            hash.update((r.authorization.origins.len() as u64).to_be_bytes());
            for origin in &r.authorization.origins {
                text(&mut hash, origin.as_str());
            }
            hash.update((r.cas.len() as u64).to_be_bytes());
            for ca in &r.cas {
                hash.update(Sha256::digest(ca));
            }
            hash.update((r.identities.len() as u64).to_be_bytes());
            hash.update((r.bindings.len() as u64).to_be_bytes());
            for binding in &r.bindings {
                text(&mut hash, binding.origin.as_str());
                text(&mut hash, &binding.fingerprint_sha256);
            }
        }
        hash.finalize().into()
    }
    fn enroll(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<Option<OciBrowserPki>, BrowserDriverError> {
        if !valid_id(&request.binding.runtime_id)
            || request.options.allowed_origins.is_empty()
            || request.options.allowed_origins.len() > 32
        {
            return Err(BrowserDriverError::Denied);
        }
        let mut selected = None;
        for registration in &self.registrations {
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
            // NSS CA trust is global within one profile. Any intersection must
            // admit the complete immutable envelope before importing any CA/key.
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
        let Some(registration) = selected else {
            return Ok(None);
        };
        let bindings = registration
            .bindings
            .iter()
            .filter(|binding| request.options.allowed_origins.contains(&binding.origin))
            .map(|binding| OciClientIdentityBinding {
                origin: binding.origin.clone(),
                fingerprint_sha256: binding.fingerprint_sha256.clone(),
            })
            .collect::<Vec<_>>();
        let identities = if bindings.is_empty() {
            Vec::new()
        } else {
            registration
                .identities
                .iter()
                .map(OciBrowserIdentity::duplicate)
                .collect()
        };
        OciBrowserPki::new(
            request.binding.clone(),
            registration.cas.clone(),
            identities,
            bindings,
        )
        .map(Some)
    }
}
fn text(hash: &mut Sha256, text: &str) {
    hash.update((text.len() as u64).to_be_bytes());
    hash.update(text.as_bytes());
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests;
