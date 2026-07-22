use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_api::{
    ApiResult, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, RequestId,
};
use getrandom::fill;
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, Mutex, Weak},
};
use subtle::ConstantTimeEq as _;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::{Request, Status, service::Interceptor};
use uuid::Uuid;
use zeroize::Zeroizing;

const TOKEN_PREFIX: &str = "cls_v1";
const TOKEN_SECRET_BYTES: usize = 32;
const MAX_AUTHORIZATION_BYTES: usize = 768;
const CREDENTIAL_DOMAIN: &[u8] = b"colossus-public-api-credential-v1\0";
pub(crate) const MAX_CREDENTIAL_BATCH_SIZE: usize = 2;
/// Maximum authenticated protobuf decodes in progress across all applications.
pub const MAX_CONCURRENT_AUTHENTICATED_DECODES: usize = 8;
/// Maximum authenticated protobuf decodes in progress for one application.
pub const MAX_CONCURRENT_AUTHENTICATED_DECODES_PER_APPLICATION: usize = 2;

type HmacSha256 = Hmac<Sha256>;

/// Validated authorization ceilings assigned to one application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationGrant {
    application_id: String,
    kind: ApplicationKind,
    scopes: Vec<ApiScope>,
    allowed_roles: Vec<String>,
    allowed_tools: Vec<String>,
}

impl ApplicationGrant {
    /// Validate a grant before any credential is issued.
    pub fn new(
        application_id: impl Into<String>,
        kind: ApplicationKind,
        scopes: impl IntoIterator<Item = ApiScope>,
        allowed_roles: impl IntoIterator<Item = String>,
        allowed_tools: impl IntoIterator<Item = String>,
    ) -> ApiResult<Self> {
        let application_id = application_id.into();
        let scopes = scopes.into_iter().collect::<Vec<_>>();
        let allowed_roles = allowed_roles.into_iter().collect::<Vec<_>>();
        let allowed_tools = allowed_tools.into_iter().collect::<Vec<_>>();
        ApplicationPrincipal::authenticated(
            application_id.clone(),
            "grant-validation",
            kind,
            scopes.clone(),
            allowed_roles.clone(),
            allowed_tools.clone(),
        )?;
        Ok(Self {
            application_id,
            kind,
            scopes,
            allowed_roles,
            allowed_tools,
        })
    }

    fn principal(&self, credential_id: &str) -> ApiResult<ApplicationPrincipal> {
        ApplicationPrincipal::authenticated(
            self.application_id.clone(),
            credential_id,
            self.kind,
            self.scopes.clone(),
            self.allowed_roles.clone(),
            self.allowed_tools.clone(),
        )
    }
}

/// Persistable credential verifier and authorization metadata.
///
/// The bearer secret is never stored. `verifier` is a keyed HMAC under the independent
/// API authentication root, so a copied repository is insufficient to validate guesses.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialState {
    Pending,
    Active,
    Revoked,
}

/// Durable credential grant, verifier, and bootstrap activation state.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRecord {
    credential_id: String,
    application_id: String,
    kind: ApplicationKind,
    scopes: BTreeSet<ApiScope>,
    allowed_roles: BTreeSet<String>,
    allowed_tools: BTreeSet<String>,
    verifier: [u8; 32],
    state: CredentialState,
}

impl CredentialRecord {
    /// Stable non-secret credential identifier.
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    /// Stable authenticated application identifier.
    pub fn application_id(&self) -> &str {
        &self.application_id
    }

    /// Whether this credential has been revoked.
    pub const fn is_revoked(&self) -> bool {
        matches!(self.state, CredentialState::Revoked)
    }

    /// Whether bootstrap delivery has durably activated this credential.
    pub const fn is_active(&self) -> bool {
        matches!(self.state, CredentialState::Active)
    }

    pub(crate) const fn is_pending(&self) -> bool {
        matches!(self.state, CredentialState::Pending)
    }

    pub(crate) fn is_valid(&self) -> bool {
        Uuid::parse_str(&self.credential_id)
            .is_ok_and(|credential_id| credential_id.to_string() == self.credential_id)
            && self.principal().is_ok()
    }

    pub(crate) fn mark_revoked(&mut self) {
        self.state = CredentialState::Revoked;
    }

    pub(crate) fn mark_active(&mut self) {
        self.state = CredentialState::Active;
    }

    fn principal(&self) -> ApiResult<ApplicationPrincipal> {
        ApplicationPrincipal::authenticated(
            self.application_id.clone(),
            self.credential_id.clone(),
            self.kind,
            self.scopes.iter().cloned(),
            self.allowed_roles.iter().cloned(),
            self.allowed_tools.iter().cloned(),
        )
    }
}

impl fmt::Debug for CredentialRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialRecord")
            .field("credential_id", &self.credential_id)
            .field("application_id", &self.application_id)
            .field("kind", &self.kind)
            .field("scopes", &self.scopes)
            .field("allowed_roles", &self.allowed_roles)
            .field("allowed_tools", &self.allowed_tools)
            .field("verifier", &"[REDACTED]")
            .field("state", &self.state)
            .finish()
    }
}

/// Credential repository failure.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("credential repository is unavailable")]
pub struct CredentialStoreError;

/// Persistence boundary for credential verifier records.
pub trait CredentialRepository: Send + Sync {
    /// Insert a newly issued unique record.
    fn insert(&self, record: CredentialRecord) -> Result<(), CredentialStoreError>;

    /// Insert multiple credentials as one all-or-nothing persistence operation.
    ///
    /// Repositories that cannot provide atomic multi-record insertion must fail closed.
    fn insert_batch(&self, records: Vec<CredentialRecord>) -> Result<(), CredentialStoreError> {
        if records.len() != 1 {
            return Err(CredentialStoreError);
        }
        self.insert(records.into_iter().next().ok_or(CredentialStoreError)?)
    }

    /// Load one record by exact identifier.
    fn get(&self, credential_id: &str) -> Result<Option<CredentialRecord>, CredentialStoreError>;

    /// Activate one pending credential after secret-safe delivery.
    fn activate(&self, credential_id: &str) -> Result<bool, CredentialStoreError>;

    /// Activate multiple pending credentials atomically.
    fn activate_batch(&self, credential_ids: &[String]) -> Result<bool, CredentialStoreError> {
        if credential_ids.len() != 1 {
            return Err(CredentialStoreError);
        }
        self.activate(credential_ids.first().ok_or(CredentialStoreError)?)
    }

    /// Permanently revoke one existing record.
    fn revoke(&self, credential_id: &str) -> Result<bool, CredentialStoreError>;

    /// Revoke multiple credentials atomically.
    fn revoke_batch(&self, credential_ids: &[String]) -> Result<bool, CredentialStoreError> {
        if credential_ids.len() != 1 {
            return Err(CredentialStoreError);
        }
        self.revoke(credential_ids.first().ok_or(CredentialStoreError)?)
    }
}

/// Deterministic in-memory credential repository for embedded use and tests.
#[derive(Default)]
pub struct InMemoryCredentialRepository {
    records: Mutex<BTreeMap<String, CredentialRecord>>,
}

impl CredentialRepository for InMemoryCredentialRepository {
    fn insert(&self, record: CredentialRecord) -> Result<(), CredentialStoreError> {
        let mut records = self.records.lock().map_err(|_| CredentialStoreError)?;
        if records.contains_key(record.credential_id()) {
            return Err(CredentialStoreError);
        }
        records.insert(record.credential_id.clone(), record);
        Ok(())
    }

    fn insert_batch(&self, records: Vec<CredentialRecord>) -> Result<(), CredentialStoreError> {
        if records.is_empty() || records.len() > MAX_CREDENTIAL_BATCH_SIZE {
            return Err(CredentialStoreError);
        }
        let mut stored = self.records.lock().map_err(|_| CredentialStoreError)?;
        let mut ids = BTreeSet::new();
        if records.iter().any(|record| {
            !record.is_valid()
                || !record.is_pending()
                || !ids.insert(record.credential_id().to_owned())
                || stored.contains_key(record.credential_id())
        }) {
            return Err(CredentialStoreError);
        }
        for record in records {
            stored.insert(record.credential_id.clone(), record);
        }
        Ok(())
    }

    fn get(&self, credential_id: &str) -> Result<Option<CredentialRecord>, CredentialStoreError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| CredentialStoreError)?
            .get(credential_id)
            .cloned())
    }

    fn activate(&self, credential_id: &str) -> Result<bool, CredentialStoreError> {
        let mut records = self.records.lock().map_err(|_| CredentialStoreError)?;
        let Some(record) = records.get_mut(credential_id) else {
            return Ok(false);
        };
        match record.state {
            CredentialState::Pending => {
                record.mark_active();
                Ok(true)
            }
            CredentialState::Active => Ok(true),
            CredentialState::Revoked => Ok(false),
        }
    }

    fn activate_batch(&self, credential_ids: &[String]) -> Result<bool, CredentialStoreError> {
        let mut records = self.records.lock().map_err(|_| CredentialStoreError)?;
        if credential_ids.is_empty()
            || credential_ids.len() > MAX_CREDENTIAL_BATCH_SIZE
            || credential_ids.iter().collect::<BTreeSet<_>>().len() != credential_ids.len()
            || credential_ids.iter().any(|credential_id| {
                records
                    .get(credential_id)
                    .is_none_or(CredentialRecord::is_revoked)
            })
        {
            return Ok(false);
        }
        for credential_id in credential_ids {
            records
                .get_mut(credential_id)
                .ok_or(CredentialStoreError)?
                .mark_active();
        }
        Ok(true)
    }

    fn revoke(&self, credential_id: &str) -> Result<bool, CredentialStoreError> {
        let mut records = self.records.lock().map_err(|_| CredentialStoreError)?;
        let Some(record) = records.get_mut(credential_id) else {
            return Ok(false);
        };
        record.mark_revoked();
        Ok(true)
    }

    fn revoke_batch(&self, credential_ids: &[String]) -> Result<bool, CredentialStoreError> {
        let mut records = self.records.lock().map_err(|_| CredentialStoreError)?;
        if credential_ids.is_empty()
            || credential_ids.len() > MAX_CREDENTIAL_BATCH_SIZE
            || credential_ids.iter().collect::<BTreeSet<_>>().len() != credential_ids.len()
            || credential_ids
                .iter()
                .any(|credential_id| !records.contains_key(credential_id))
        {
            return Ok(false);
        }
        for credential_id in credential_ids {
            records
                .get_mut(credential_id)
                .ok_or(CredentialStoreError)?
                .mark_revoked();
        }
        Ok(true)
    }
}

/// One newly issued bearer credential.
///
/// The token is deliberately non-cloneable, redacted from debug output, and zeroized
/// when dropped. It must be delivered over an inherited bootstrap channel or written
/// directly to a platform credential store; it must never enter argv, environment
/// variables, endpoint descriptors, or logs.
pub struct IssuedCredential {
    credential_id: String,
    token: Zeroizing<String>,
}

impl IssuedCredential {
    /// Stable non-secret credential identifier.
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    /// Borrow the bearer token for secure bootstrap delivery.
    pub fn expose_token(&self) -> &str {
        self.token.as_str()
    }
}

impl fmt::Debug for IssuedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedCredential")
            .field("credential_id", &self.credential_id)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// Issues and verifies opaque application credentials.
pub struct CredentialAuthenticator {
    root: Zeroizing<[u8; 32]>,
    repository: Arc<dyn CredentialRepository>,
}

impl CredentialAuthenticator {
    /// Construct an authenticator from an independent API root and repository.
    pub fn new(root: [u8; 32], repository: Arc<dyn CredentialRepository>) -> Self {
        Self {
            root: Zeroizing::new(root),
            repository,
        }
    }

    /// Issue one pending credential and persist only its verifier.
    ///
    /// Pending credentials never authenticate. Trusted bootstrap code must deliver the
    /// bearer into its protected destination and then call [`Self::activate`].
    pub fn issue_pending(
        &self,
        grant: &ApplicationGrant,
    ) -> Result<IssuedCredential, CredentialStoreError> {
        self.issue_pending_batch(std::slice::from_ref(grant))?
            .pop()
            .ok_or(CredentialStoreError)
    }

    /// Issue multiple pending credentials in one all-or-nothing repository transaction.
    pub fn issue_pending_batch(
        &self,
        grants: &[ApplicationGrant],
    ) -> Result<Vec<IssuedCredential>, CredentialStoreError> {
        if grants.is_empty() || grants.len() > MAX_CREDENTIAL_BATCH_SIZE {
            return Err(CredentialStoreError);
        }
        let mut records = Vec::with_capacity(grants.len());
        let mut issued = Vec::with_capacity(grants.len());
        for grant in grants {
            let (record, credential) = self.pending_credential(grant)?;
            records.push(record);
            issued.push(credential);
        }
        self.repository.insert_batch(records)?;
        Ok(issued)
    }

    fn pending_credential(
        &self,
        grant: &ApplicationGrant,
    ) -> Result<(CredentialRecord, IssuedCredential), CredentialStoreError> {
        let credential_id = Uuid::now_v7().to_string();
        let mut secret = Zeroizing::new([0_u8; TOKEN_SECRET_BYTES]);
        fill(secret.as_mut()).map_err(|_| CredentialStoreError)?;
        let verifier = self.verifier(&credential_id, secret.as_ref());
        let principal = grant
            .principal(&credential_id)
            .map_err(|_| CredentialStoreError)?;
        let record = CredentialRecord {
            credential_id: credential_id.clone(),
            application_id: principal.application_id().to_owned(),
            kind: principal.kind(),
            scopes: grant.scopes.iter().cloned().collect(),
            allowed_roles: grant.allowed_roles.iter().cloned().collect(),
            allowed_tools: grant.allowed_tools.iter().cloned().collect(),
            verifier,
            state: CredentialState::Pending,
        };
        let encoded = URL_SAFE_NO_PAD.encode(secret.as_ref());
        let credential = IssuedCredential {
            credential_id: credential_id.clone(),
            token: Zeroizing::new(format!("{TOKEN_PREFIX}.{credential_id}.{encoded}")),
        };
        Ok((record, credential))
    }

    /// Durably activate one pending credential after secret-safe delivery.
    pub fn activate(&self, credential_id: &str) -> Result<bool, CredentialStoreError> {
        self.repository.activate(credential_id)
    }

    /// Durably activate multiple pending credentials in one transaction.
    pub fn activate_batch(&self, credential_ids: &[String]) -> Result<bool, CredentialStoreError> {
        if credential_ids.is_empty() || credential_ids.len() > MAX_CREDENTIAL_BATCH_SIZE {
            return Err(CredentialStoreError);
        }
        self.repository.activate_batch(credential_ids)
    }

    /// Verify an exact HTTP Authorization header.
    pub fn authenticate_authorization(
        &self,
        authorization: &str,
    ) -> Result<ApplicationPrincipal, AuthenticationError> {
        if authorization.len() > MAX_AUTHORIZATION_BYTES {
            return Err(AuthenticationError);
        }
        let token = authorization
            .strip_prefix("Bearer ")
            .ok_or(AuthenticationError)?;
        if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(AuthenticationError);
        }
        let mut parts = token.split('.');
        let prefix = parts.next().ok_or(AuthenticationError)?;
        let credential_id = parts.next().ok_or(AuthenticationError)?;
        let encoded_secret = parts.next().ok_or(AuthenticationError)?;
        if prefix != TOKEN_PREFIX || parts.next().is_some() {
            return Err(AuthenticationError);
        }
        Uuid::parse_str(credential_id).map_err(|_| AuthenticationError)?;
        let secret = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(encoded_secret)
                .map_err(|_| AuthenticationError)?,
        );
        if secret.len() != TOKEN_SECRET_BYTES {
            return Err(AuthenticationError);
        }
        let record = self
            .repository
            .get(credential_id)
            .map_err(|_| AuthenticationError)?
            .ok_or(AuthenticationError)?;
        let verifier = self.verifier(credential_id, secret.as_ref());
        if !record.is_active() || !constant_time_verify(&record.verifier, &verifier) {
            return Err(AuthenticationError);
        }
        record.principal().map_err(|_| AuthenticationError)
    }

    /// Revoke a credential by its non-secret identifier.
    pub fn revoke(&self, credential_id: &str) -> Result<bool, CredentialStoreError> {
        self.repository.revoke(credential_id)
    }

    /// Durably revoke multiple credentials in one transaction.
    pub fn revoke_batch(&self, credential_ids: &[String]) -> Result<bool, CredentialStoreError> {
        if credential_ids.is_empty() || credential_ids.len() > MAX_CREDENTIAL_BATCH_SIZE {
            return Err(CredentialStoreError);
        }
        self.repository.revoke_batch(credential_ids)
    }

    fn verifier(&self, credential_id: &str, secret: &[u8]) -> [u8; 32] {
        let mut mac =
            HmacSha256::new_from_slice(self.root.as_ref()).expect("HMAC accepts a 32-byte key");
        mac.update(CREDENTIAL_DOMAIN);
        mac.update(credential_id.as_bytes());
        mac.update(&[0]);
        mac.update(secret);
        mac.finalize().into_bytes().into()
    }
}

impl fmt::Debug for CredentialAuthenticator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialAuthenticator")
            .field("root", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

fn constant_time_verify(expected: &[u8; 32], actual: &[u8; 32]) -> bool {
    expected.ct_eq(actual).into()
}

/// Intentionally indistinguishable authentication failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("authentication failed")]
pub struct AuthenticationError;

/// Tonic interceptor that derives caller identity exclusively from bearer verification.
#[derive(Clone)]
pub struct AuthenticationInterceptor {
    authenticator: Arc<CredentialAuthenticator>,
    decode_slots: Arc<Semaphore>,
    application_decode_slots: Arc<Mutex<BTreeMap<String, Weak<Semaphore>>>>,
}

struct AuthenticatedDecodePermit {
    _global: OwnedSemaphorePermit,
    _application: OwnedSemaphorePermit,
}

impl AuthenticationInterceptor {
    /// Construct an interceptor for one server authenticator.
    pub fn new(authenticator: Arc<CredentialAuthenticator>) -> Self {
        Self {
            authenticator,
            decode_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_AUTHENTICATED_DECODES)),
            application_decode_slots: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn acquire_decode_permit(
        &self,
        application_id: &str,
    ) -> Result<Arc<AuthenticatedDecodePermit>, Status> {
        let global = Arc::clone(&self.decode_slots)
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("authenticated request capacity reached"))?;
        let application_slots = {
            let mut slots_by_application = self
                .application_decode_slots
                .lock()
                .map_err(|_| Status::unavailable("request admission is unavailable"))?;
            slots_by_application.retain(|_, slots| slots.strong_count() > 0);
            slots_by_application
                .get(application_id)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let application_slots = Arc::new(Semaphore::new(
                        MAX_CONCURRENT_AUTHENTICATED_DECODES_PER_APPLICATION,
                    ));
                    slots_by_application
                        .insert(application_id.into(), Arc::downgrade(&application_slots));
                    application_slots
                })
        };
        let application = application_slots
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("application request capacity reached"))?;
        Ok(Arc::new(AuthenticatedDecodePermit {
            _global: global,
            _application: application,
        }))
    }
}

impl Interceptor for AuthenticationInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let authorization = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| Status::unauthenticated("application authentication is required"))?;
        let principal = self
            .authenticator
            .authenticate_authorization(authorization)
            .map_err(|_| Status::unauthenticated("application authentication failed"))?;
        let decode_permit = self.acquire_decode_permit(principal.application_id())?;
        // Downstream handlers receive only the derived caller context. Keeping the
        // bearer header attached would unnecessarily widen its lifetime and make an
        // otherwise harmless request-metadata diagnostic capable of disclosing it.
        request.metadata_mut().remove("authorization");
        let request_id = RequestId::new(Uuid::now_v7().to_string())
            .map_err(|_| Status::internal("request correlation failed"))?;
        request
            .extensions_mut()
            .insert(CallerContext::authenticated(principal, request_id));
        // Tonic preserves request extensions while decoding. Holding both permits in
        // an Arc therefore bounds authenticated Prost allocations before a handler can
        // inspect repeated-field cardinality; `into_inner` releases them before the
        // potentially long-running application operation.
        request.extensions_mut().insert(decode_permit);
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_api::scopes;

    fn grant() -> ApplicationGrant {
        ApplicationGrant::new(
            "app:test-ui",
            ApplicationKind::Enrolled,
            [
                ApiScope::new(scopes::RUNS_EXECUTE).expect("scope"),
                ApiScope::new(scopes::RUNS_READ).expect("scope"),
            ],
            ["primary".into()],
            ["session.list".into()],
        )
        .expect("grant")
    }

    #[test]
    fn issued_secret_is_not_persisted_or_debugged() {
        let repository = Arc::new(InMemoryCredentialRepository::default());
        let authenticator = CredentialAuthenticator::new([7_u8; 32], repository.clone());
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issued pending");
        let record = repository
            .get(issued.credential_id())
            .expect("repository")
            .expect("record");
        let token = issued.expose_token().to_owned();
        assert!(!format!("{issued:?}").contains(&token));
        assert!(!format!("{record:?}").contains(&token));
        assert!(
            authenticator
                .authenticate_authorization(&format!("Bearer {token}"))
                .is_err(),
            "pending credentials never authenticate"
        );
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("activate")
        );
        assert_eq!(
            authenticator
                .authenticate_authorization(&format!("Bearer {token}"))
                .expect("authenticated")
                .application_id(),
            "app:test-ui"
        );
    }

    #[test]
    fn credential_batches_activate_and_revoke_all_or_none() {
        let repository = Arc::new(InMemoryCredentialRepository::default());
        let authenticator = CredentialAuthenticator::new([19_u8; 32], repository);
        assert!(
            authenticator
                .issue_pending_batch(&[grant(), grant(), grant()])
                .is_err(),
            "credential batches are bounded before allocation or random generation"
        );
        let issued = authenticator
            .issue_pending_batch(&[grant(), grant()])
            .expect("issue pair");
        let authorizations = issued
            .iter()
            .map(|credential| format!("Bearer {}", credential.expose_token()))
            .collect::<Vec<_>>();
        let mut invalid_ids = vec![
            issued[0].credential_id().to_owned(),
            Uuid::now_v7().to_string(),
        ];
        assert!(
            !authenticator
                .activate_batch(&invalid_ids)
                .expect("reject pair")
        );
        assert!(
            authenticator
                .authenticate_authorization(&authorizations[0])
                .is_err(),
            "a failed pair activation must not activate its valid member"
        );

        let credential_ids = issued
            .iter()
            .map(|credential| credential.credential_id().to_owned())
            .collect::<Vec<_>>();
        let oversized_ids = vec![
            credential_ids[0].clone(),
            credential_ids[1].clone(),
            Uuid::now_v7().to_string(),
        ];
        assert!(authenticator.activate_batch(&oversized_ids).is_err());
        assert!(authenticator.revoke_batch(&oversized_ids).is_err());
        assert!(
            authenticator
                .activate_batch(&credential_ids)
                .expect("activate pair")
        );
        assert!(authorizations.iter().all(|authorization| {
            authenticator
                .authenticate_authorization(authorization)
                .is_ok()
        }));

        invalid_ids[0] = credential_ids[0].clone();
        assert!(
            !authenticator
                .revoke_batch(&invalid_ids)
                .expect("reject revoke")
        );
        assert!(
            authenticator
                .authenticate_authorization(&authorizations[0])
                .is_ok(),
            "a failed pair revocation must not revoke its valid member"
        );
        assert!(
            authenticator
                .revoke_batch(&credential_ids)
                .expect("revoke pair")
        );
        assert!(authorizations.iter().all(|authorization| {
            authenticator
                .authenticate_authorization(authorization)
                .is_err()
        }));
    }

    #[test]
    fn tampering_revocation_and_another_root_fail_closed() {
        let repository = Arc::new(InMemoryCredentialRepository::default());
        let authenticator = CredentialAuthenticator::new([9_u8; 32], repository.clone());
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issued pending");
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("activate")
        );
        let authorization = format!("Bearer {}", issued.expose_token());
        let mut tampered = authorization.clone();
        tampered.push('A');
        assert!(authenticator.authenticate_authorization(&tampered).is_err());
        assert!(
            CredentialAuthenticator::new([8_u8; 32], repository.clone())
                .authenticate_authorization(&authorization)
                .is_err()
        );
        assert!(
            authenticator
                .revoke(issued.credential_id())
                .expect("revoke")
        );
        assert!(
            authenticator
                .authenticate_authorization(&authorization)
                .is_err()
        );
    }

    #[test]
    fn malformed_authorization_never_authenticates() {
        let authenticator = CredentialAuthenticator::new(
            [4_u8; 32],
            Arc::new(InMemoryCredentialRepository::default()),
        );
        for value in [
            "",
            "bearer x",
            "Bearer ",
            "Bearer cls_v1.only-two",
            "Bearer cls_v1.not-a-uuid.AAAA",
            "Bearer cls_v1.00000000-0000-0000-0000-000000000000.A A",
        ] {
            assert!(
                authenticator.authenticate_authorization(value).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn interceptor_consumes_bearer_metadata_after_deriving_identity() {
        let authenticator = Arc::new(CredentialAuthenticator::new(
            [5_u8; 32],
            Arc::new(InMemoryCredentialRepository::default()),
        ));
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issued pending");
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("activate")
        );
        let authorization = format!("Bearer {}", issued.expose_token())
            .parse()
            .expect("authorization metadata");
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", authorization);

        let accepted = AuthenticationInterceptor::new(authenticator)
            .call(request)
            .expect("authenticated");
        assert!(accepted.metadata().get("authorization").is_none());
        assert_eq!(
            accepted
                .extensions()
                .get::<CallerContext>()
                .expect("caller context")
                .principal()
                .application_id(),
            "app:test-ui"
        );
    }

    #[test]
    fn interceptor_bounds_authenticated_decodes_per_application() {
        let authenticator = Arc::new(CredentialAuthenticator::new(
            [6_u8; 32],
            Arc::new(InMemoryCredentialRepository::default()),
        ));
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issued pending");
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("activate")
        );
        let authorization = format!("Bearer {}", issued.expose_token());
        let mut interceptor = AuthenticationInterceptor::new(authenticator);
        let request = || {
            let mut request = Request::new(());
            request.metadata_mut().insert(
                "authorization",
                authorization.parse().expect("authorization metadata"),
            );
            request
        };

        let first = interceptor.call(request()).expect("first decode slot");
        let second = interceptor.call(request()).expect("second decode slot");
        let error = interceptor
            .call(request())
            .expect_err("third same-application decode must be rejected");
        assert_eq!(error.code(), tonic::Code::ResourceExhausted);
        drop(first);
        assert!(
            interceptor.call(request()).is_ok(),
            "dropping a decoded request releases its admission permits"
        );
        drop(second);
    }
}
