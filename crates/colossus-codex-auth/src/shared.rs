//! Opaque CAS custody with durable, pre-exchange rotating-token arbitration.
use super::*;

const MAX_SHARED_BYTES: usize = 257 * 1024;

/// One zeroizing opaque shared-account record. Revision zero denotes no record and
/// is never a usable authorization. Adapters must not log or debug these bytes.
pub struct CodexAuthRecord {
    /// Monotonically increasing committed CAS revision.
    pub revision: u64,
    /// Bounded private envelope, encrypted by the shared adapter at rest.
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Trusted custody for one explicitly selected purpose/project/account binding.
/// Reads must be current; CAS must atomically compare the revision and persist the
/// complete record at exactly `expected_revision + 1`. Runtime-owned writes must
/// also fence the current execution lease. Never fall back to another account.
pub trait CodexAuthStorage: Send + Sync {
    /// Read the selected record; missing custody fails closed.
    fn read(&self) -> Result<CodexAuthRecord, CodexAuthError>;
    /// Commit one exact revision, or fail without replacing a concurrent writer.
    fn compare_and_swap(&self, expected_revision: u64, bytes: &[u8])
    -> Result<u64, CodexAuthError>;
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedDocument {
    version: u32,
    binding: String,
    refresh_pending: bool,
    auth: StoredAuth,
}

#[derive(Clone)]
pub(super) struct SharedCodexStore {
    storage: Arc<dyn CodexAuthStorage>,
    binding: Arc<str>,
}

pub(super) struct SharedRefreshClaim {
    binding: Arc<str>,
    revision: u64,
}

impl SharedCodexStore {
    pub(super) fn new(
        storage: Arc<dyn CodexAuthStorage>,
        binding: String,
    ) -> Result<Self, CodexAuthError> {
        validate_binding(&binding)?;
        Ok(Self {
            storage,
            binding: binding.into(),
        })
    }

    fn read(&self) -> Result<(u64, SharedDocument), CodexAuthError> {
        let record = self.storage.read()?;
        if record.revision == 0 || record.bytes.is_empty() || record.bytes.len() > MAX_SHARED_BYTES
        {
            return Err(CodexAuthError::Unavailable(
                "shared Codex account record is unavailable".into(),
            ));
        }
        let document: SharedDocument = serde_json::from_slice(&record.bytes).map_err(|_| {
            CodexAuthError::Unavailable("shared Codex account record is invalid".into())
        })?;
        if document.version != 1 || document.binding.as_str() != self.binding.as_ref() {
            return Err(CodexAuthError::Unavailable(
                "shared Codex account binding is invalid".into(),
            ));
        }
        Ok((record.revision, document))
    }

    pub(super) fn load(&self) -> Result<CodexAuthorization, CodexAuthError> {
        let (revision, document) = self.read()?;
        require_ready(&document)?;
        self.authorization(&document, revision)
    }

    fn authorization(
        &self,
        document: &SharedDocument,
        revision: u64,
    ) -> Result<CodexAuthorization, CodexAuthError> {
        let mut authorization = validated_authorization(&document.auth)?;
        authorization.shared_revision = Some(revision);
        authorization.shared_binding = Some(self.binding.clone());
        Ok(authorization)
    }

    pub(super) fn export(&self, binding: &str) -> Result<Zeroizing<Vec<u8>>, CodexAuthError> {
        if binding != self.binding.as_ref() {
            return Err(credentials_changed());
        }
        let (_, document) = self.read()?;
        require_ready(&document)?;
        validated_authorization(&document.auth)?;
        encode(&document)
    }

    pub(super) fn begin(
        &self,
        expected: &CodexAuthorization,
    ) -> Result<SharedRefreshClaim, CodexAuthError> {
        let (revision, mut document) = self.read()?;
        require_ready(&document)?;
        self.check_expected(&document, revision, expected)?;
        document.refresh_pending = true;
        let revision = self.commit(revision, &document)?;
        Ok(SharedRefreshClaim {
            binding: self.binding.clone(),
            revision,
        })
    }

    pub(super) fn finish(
        &self,
        expected: &CodexAuthorization,
        claim: SharedRefreshClaim,
        response: &[u8],
    ) -> Result<CodexAuthorization, CodexAuthError> {
        if claim.binding.as_ref() != self.binding.as_ref()
            || expected.shared_binding.as_deref() != Some(self.binding.as_ref())
            || expected
                .shared_revision
                .and_then(|value| value.checked_add(1))
                != Some(claim.revision)
        {
            return Err(credentials_changed());
        }
        let (revision, mut document) = self.read()?;
        if revision != claim.revision || !document.refresh_pending {
            return Err(credentials_changed());
        }
        let current = validated_authorization(&document.auth)?;
        if current.account_id() != expected.account_id()
            || current.refresh_token() != expected.refresh_token()
            || current.access_token() != expected.access_token()
        {
            return Err(credentials_changed());
        }
        merge_refresh(&mut document.auth, expected, response)?;
        document.refresh_pending = false;
        let revision = self.commit(revision, &document)?;
        self.authorization(&document, revision)
    }

    fn check_expected(
        &self,
        document: &SharedDocument,
        revision: u64,
        expected: &CodexAuthorization,
    ) -> Result<(), CodexAuthError> {
        if expected.shared_revision != Some(revision)
            || expected.shared_binding.as_deref() != Some(self.binding.as_ref())
        {
            return Err(credentials_changed());
        }
        let current = validated_authorization(&document.auth)?;
        if current.account_id() != expected.account_id()
            || current.access_token() != expected.access_token()
            || current.refresh_token() != expected.refresh_token()
        {
            return Err(credentials_changed());
        }
        Ok(())
    }

    fn commit(&self, previous: u64, document: &SharedDocument) -> Result<u64, CodexAuthError> {
        let expected = previous.checked_add(1).ok_or_else(credentials_changed)?;
        let bytes = encode(document)?;
        let revision = self.storage.compare_and_swap(previous, &bytes)?;
        if revision != expected {
            return Err(credentials_changed());
        }
        Ok(revision)
    }
}

pub(super) fn export_document(
    auth: StoredAuth,
    binding: &str,
) -> Result<Zeroizing<Vec<u8>>, CodexAuthError> {
    validate_binding(binding)?;
    validated_authorization(&auth)?;
    // Keep unrelated API-key or metadata fields inside the validated private envelope;
    // no credential reaches a renderer, log, or public configuration snapshot.
    encode(&SharedDocument {
        version: 1,
        binding: binding.into(),
        refresh_pending: false,
        auth,
    })
}

fn encode(document: &SharedDocument) -> Result<Zeroizing<Vec<u8>>, CodexAuthError> {
    let bytes = Zeroizing::new(serde_json::to_vec(document).map_err(|_| {
        CodexAuthError::Storage("shared Codex account serialization failed".into())
    })?);
    if bytes.len() > MAX_SHARED_BYTES {
        return Err(CodexAuthError::Storage(
            "shared Codex account record exceeds the safety bound".into(),
        ));
    }
    Ok(bytes)
}

fn validate_binding(binding: &str) -> Result<(), CodexAuthError> {
    if binding.is_empty()
        || binding.len() > 1024
        || !binding
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || b"-_:/".contains(&byte))
    {
        return Err(CodexAuthError::Storage(
            "shared Codex account binding is invalid".into(),
        ));
    }
    Ok(())
}

fn validated_authorization(auth: &StoredAuth) -> Result<CodexAuthorization, CodexAuthError> {
    let authorization = authorization_from_file(auth)?;
    let tokens = auth.tokens.as_ref().ok_or_else(missing_tokens)?;
    if token_metadata(&tokens.id_token)?.account_id.as_deref() != Some(authorization.account_id()) {
        return Err(CodexAuthError::Unavailable(
            "shared Codex account identity is inconsistent".into(),
        ));
    }
    Ok(authorization)
}

fn require_ready(document: &SharedDocument) -> Result<(), CodexAuthError> {
    if document.refresh_pending {
        return Err(CodexAuthError::Unavailable(
            "shared Codex token refresh is pending or interrupted; reauthenticate explicitly"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
