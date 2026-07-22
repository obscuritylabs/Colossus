use colossus_grpc::{CredentialAuthenticator, JournalCredentialRepository};
use colossus_ports::EventJournal;
use std::{fmt, sync::Arc};
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_ROTATION_BEARER_BYTES: usize = 761;

pub use colossus_grpc::{
    ApplicationGrant, CredentialStoreError as PublicApiCredentialError, IssuedCredential,
};

/// Independent 256-bit authentication root for the public application API.
///
/// Trusted composition code must load this value from an API-specific platform
/// secret. It must be stable across worker restarts and must never be reused as the
/// worker IPC key, journal encryption key, checkpoint signing key, TLS seed, or a
/// provider credential. Constructing this wrapper consumes the byte array, and every
/// retained copy owned by this API is zeroized on drop.
pub struct PublicApiAuthenticationKey(Zeroizing<[u8; 32]>);

impl PublicApiAuthenticationKey {
    /// Wrap an exact 256-bit public API authentication root.
    pub fn new(key: [u8; 32]) -> Self {
        Self(Zeroizing::new(key))
    }
}

impl fmt::Debug for PublicApiAuthenticationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PublicApiAuthenticationKey")
            .field(&"[REDACTED]")
            .finish()
    }
}

/// A stored rotation source failed authentication or belongs to another application.
///
/// This error is intentionally indistinguishable and carries no bearer material.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("public API rotation source credential is invalid")]
pub struct PublicApiRotationSourceError;

/// Journal-bound application credential issuer, activator, and revoker.
///
/// This manager is created only through
/// [`WorkerServer::public_api_credential_manager`](crate::WorkerServer::public_api_credential_manager),
/// which binds it to that worker's authoritative encrypted journal. Issuance persists
/// only a keyed verifier and the bounded [`ApplicationGrant`]. The returned
/// [`IssuedCredential`] contains the bearer token exactly once: it is non-cloneable,
/// zeroized on drop, and redacted from `Debug`.
///
/// Trusted composition code must transfer the bearer directly over an inherited
/// bootstrap channel or into an OS credential store. It must never put the bearer in
/// files, endpoint descriptors, command-line arguments, environment variables, logs,
/// error messages, or WebView/renderer state. Revocation accepts only the stable,
/// non-secret credential identifier.
pub struct PublicApiCredentialManager {
    authenticator: Arc<CredentialAuthenticator>,
    journal: Arc<dyn EventJournal>,
}

impl PublicApiCredentialManager {
    pub(crate) fn bind(
        journal: Arc<dyn EventJournal>,
        authentication_key: PublicApiAuthenticationKey,
    ) -> Self {
        let repository = Arc::new(JournalCredentialRepository::new(Arc::clone(&journal)));
        let authenticator = Arc::new(CredentialAuthenticator::new(
            *authentication_key.0,
            repository,
        ));
        Self {
            authenticator,
            journal,
        }
    }

    /// Issue one pending bearer credential and durably persist only its verifier and grant.
    ///
    /// Pending credentials cannot authenticate. The caller must deliver the returned
    /// bearer using a secret-safe channel and call [`Self::activate`] only after that
    /// delivery succeeds. Colossus intentionally provides no file, argv, environment,
    /// or descriptor serialization helper for bearer credentials.
    pub fn issue_pending(
        &self,
        grant: &ApplicationGrant,
    ) -> Result<IssuedCredential, PublicApiCredentialError> {
        self.authenticator.issue_pending(grant)
    }

    /// Issue a bounded credential set in one durable all-or-nothing transaction.
    pub fn issue_pending_batch(
        &self,
        grants: &[ApplicationGrant],
    ) -> Result<Vec<IssuedCredential>, PublicApiCredentialError> {
        self.authenticator.issue_pending_batch(grants)
    }

    /// Activate one pending credential after confirmed secret-safe delivery.
    ///
    /// Returns `false` for an absent or already revoked credential. Activation is
    /// durable and idempotent.
    pub fn activate(&self, credential_id: &str) -> Result<bool, PublicApiCredentialError> {
        self.authenticator.activate(credential_id)
    }

    /// Activate a delivered credential set in one durable transaction.
    pub fn activate_batch(
        &self,
        credential_ids: &[String],
    ) -> Result<bool, PublicApiCredentialError> {
        self.authenticator.activate_batch(credential_ids)
    }

    /// Permanently revoke an issued credential by its non-secret identifier.
    ///
    /// Returns `false` when no such credential exists. A successful revocation is
    /// durable and idempotent.
    pub fn revoke(&self, credential_id: &str) -> Result<bool, PublicApiCredentialError> {
        self.authenticator.revoke(credential_id)
    }

    /// Revoke a credential set in one durable transaction.
    pub fn revoke_batch(
        &self,
        credential_ids: &[String],
    ) -> Result<bool, PublicApiCredentialError> {
        self.authenticator.revoke_batch(credential_ids)
    }

    /// Authenticate a bearer loaded from a trusted credential store for rotation.
    ///
    /// This narrow administration helper must receive the token directly from a
    /// platform credential store. It verifies the exact API authentication root,
    /// durable verifier, revocation state, and expected application identity before
    /// returning only the non-secret credential identifier. It must never be wired to
    /// argv, environment, files, logs, renderer input, or a public request.
    pub fn validate_rotation_source(
        &self,
        bearer: &str,
        expected_application_id: &str,
    ) -> Result<String, PublicApiRotationSourceError> {
        if bearer.is_empty() || bearer.len() > MAX_ROTATION_BEARER_BYTES {
            return Err(PublicApiRotationSourceError);
        }
        let mut authorization = Zeroizing::new(String::with_capacity(
            "Bearer ".len().saturating_add(bearer.len()),
        ));
        authorization.push_str("Bearer ");
        authorization.push_str(bearer);
        let principal = self
            .authenticator
            .authenticate_authorization(&authorization)
            .map_err(|_| PublicApiRotationSourceError)?;
        if principal.application_id() != expected_application_id {
            return Err(PublicApiRotationSourceError);
        }
        Ok(principal.credential_id().to_owned())
    }

    pub(crate) fn authenticator(&self) -> Arc<CredentialAuthenticator> {
        Arc::clone(&self.authenticator)
    }

    pub(crate) fn journal(&self) -> Arc<dyn EventJournal> {
        Arc::clone(&self.journal)
    }
}

impl fmt::Debug for PublicApiCredentialManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicApiCredentialManager")
            .field("authenticator", &"[REDACTED]")
            .field("journal", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_api::{ApiScope, ApplicationKind, scopes};
    use colossus_contracts::ActorType;
    use colossus_testkit::InMemoryEventJournal;

    fn grant() -> ApplicationGrant {
        ApplicationGrant::new(
            "app:worker-credential-test",
            ApplicationKind::Enrolled,
            [
                ApiScope::new(scopes::RUNS_EXECUTE).expect("execute scope"),
                ApiScope::new(scopes::RUNS_READ).expect("read scope"),
            ],
            ["primary".into()],
            ["session.list".into()],
        )
        .expect("grant")
    }

    #[test]
    fn manager_persists_only_verifier_and_grant_and_reopens_with_same_key() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let journal_port: Arc<dyn EventJournal> = journal.clone();
        let manager = PublicApiCredentialManager::bind(
            journal_port,
            PublicApiAuthenticationKey::new([41_u8; 32]),
        );
        let issued = manager.issue_pending(&grant()).expect("issue pending");
        let authorization = format!("Bearer {}", issued.expose_token());

        let events = journal.read_global(0, 16).expect("read journal");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].actor.actor_type, ActorType::System);
        let envelope = serde_json::to_string(&events[0]).expect("serialize envelope");
        let payload = serde_json::to_string(
            &journal
                .decrypt_payload(&events[0])
                .expect("decrypt payload"),
        )
        .expect("serialize payload");
        assert!(!envelope.contains(issued.expose_token()));
        assert!(!payload.contains(issued.expose_token()));
        assert!(!format!("{manager:?}").contains(issued.expose_token()));
        assert!(!format!("{issued:?}").contains(issued.expose_token()));
        assert!(
            manager
                .authenticator
                .authenticate_authorization(&authorization)
                .is_err()
        );
        assert!(manager.activate(issued.credential_id()).expect("activate"));

        let reopened_journal: Arc<dyn EventJournal> = journal;
        let reopened = PublicApiCredentialManager::bind(
            reopened_journal,
            PublicApiAuthenticationKey::new([41_u8; 32]),
        );
        let principal = reopened
            .authenticator
            .authenticate_authorization(&authorization)
            .expect("authenticate durable credential");
        assert_eq!(principal.application_id(), "app:worker-credential-test");
        assert_eq!(principal.kind(), ApplicationKind::Enrolled);
        assert!(principal.has_scope(scopes::RUNS_EXECUTE));
        assert!(principal.allows_role("primary"));
        assert!(principal.allows_tool("session.list"));
    }

    #[test]
    fn manager_commits_native_credential_pairs_as_one_lifecycle() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let journal_port: Arc<dyn EventJournal> = journal.clone();
        let manager = PublicApiCredentialManager::bind(
            journal_port,
            PublicApiAuthenticationKey::new([51_u8; 32]),
        );
        let primary_grant = ApplicationGrant::new(
            "app:worker-credential-test",
            ApplicationKind::Sidecar,
            [
                ApiScope::new(scopes::RUNS_EXECUTE).expect("execute scope"),
                ApiScope::new(scopes::RUNS_READ).expect("read scope"),
                ApiScope::new(scopes::RUNS_CONTROL).expect("control scope"),
                ApiScope::new(scopes::PROMPTS_RESPOND).expect("prompt scope"),
            ],
            ["primary".into()],
            ["session.list".into()],
        )
        .expect("primary grant");
        let approval_grant = ApplicationGrant::new(
            "app:worker-credential-test",
            ApplicationKind::Sidecar,
            [ApiScope::new(scopes::APPROVALS_RESPOND).expect("approval scope")],
            ["primary".into()],
            Vec::<String>::new(),
        )
        .expect("approval grant");
        let issued = manager
            .issue_pending_batch(&[primary_grant, approval_grant])
            .expect("issue credential pair");
        let credential_ids = issued
            .iter()
            .map(|credential| credential.credential_id().to_owned())
            .collect::<Vec<_>>();
        let authorizations = issued
            .iter()
            .map(|credential| format!("Bearer {}", credential.expose_token()))
            .collect::<Vec<_>>();
        assert!(authorizations.iter().all(|authorization| {
            manager
                .authenticator
                .authenticate_authorization(authorization)
                .is_err()
        }));
        let invalid_pair = vec![credential_ids[0].clone(), uuid::Uuid::now_v7().to_string()];
        assert!(!manager.activate_batch(&invalid_pair).expect("reject pair"));
        assert!(
            manager
                .authenticator
                .authenticate_authorization(&authorizations[0])
                .is_err(),
            "journal batch activation must leave every member pending on failure"
        );
        assert!(
            manager
                .activate_batch(&credential_ids)
                .expect("activate pair")
        );
        let principals = authorizations
            .iter()
            .map(|authorization| {
                manager
                    .authenticator
                    .authenticate_authorization(authorization)
                    .expect("active credential")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            principals[0].application_id(),
            principals[1].application_id()
        );
        assert_eq!(principals[0].kind(), ApplicationKind::Sidecar);
        assert_eq!(principals[1].kind(), ApplicationKind::Sidecar);
        assert!(principals[0].has_scope(scopes::RUNS_EXECUTE));
        assert!(!principals[0].has_scope(scopes::APPROVALS_RESPOND));
        assert!(principals[1].has_scope(scopes::APPROVALS_RESPOND));
        assert!(!principals[1].has_scope(scopes::RUNS_EXECUTE));
        assert_eq!(principals[1].allowed_tools().len(), 0);

        assert!(!manager.revoke_batch(&invalid_pair).expect("reject revoke"));
        assert!(
            manager
                .authenticator
                .authenticate_authorization(&authorizations[0])
                .is_ok(),
            "journal batch revocation must leave every member active on failure"
        );
        assert!(manager.revoke_batch(&credential_ids).expect("revoke pair"));
        assert!(authorizations.iter().all(|authorization| {
            manager
                .authenticator
                .authenticate_authorization(authorization)
                .is_err()
        }));
        assert_eq!(journal.read_global(0, 16).expect("events").len(), 6);
    }

    #[test]
    fn revocation_is_durable_and_uses_only_the_non_secret_identifier() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let journal_port: Arc<dyn EventJournal> = journal.clone();
        let manager = PublicApiCredentialManager::bind(
            journal_port,
            PublicApiAuthenticationKey::new([42_u8; 32]),
        );
        let issued = manager.issue_pending(&grant()).expect("issue pending");
        let credential_id = issued.credential_id().to_owned();
        let authorization = format!("Bearer {}", issued.expose_token());

        assert!(manager.revoke(&credential_id).expect("revoke"));
        assert!(manager.revoke(&credential_id).expect("idempotent revoke"));
        assert!(
            manager
                .authenticator
                .authenticate_authorization(&authorization)
                .is_err()
        );

        let reopened_journal: Arc<dyn EventJournal> = journal.clone();
        let reopened = PublicApiCredentialManager::bind(
            reopened_journal,
            PublicApiAuthenticationKey::new([42_u8; 32]),
        );
        assert!(
            reopened
                .authenticator
                .authenticate_authorization(&authorization)
                .is_err()
        );
        let events = journal.read_global(0, 16).expect("read journal");
        assert_eq!(events.len(), 2, "idempotent revoke writes one event");
        assert!(
            events
                .iter()
                .all(|event| !event.event_type.contains(issued.expose_token()))
        );
    }

    #[test]
    fn independent_authentication_key_is_required_to_verify_durable_records() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let issuing_journal: Arc<dyn EventJournal> = journal.clone();
        let issuing = PublicApiCredentialManager::bind(
            issuing_journal,
            PublicApiAuthenticationKey::new([43_u8; 32]),
        );
        let issued = issuing.issue_pending(&grant()).expect("issue pending");
        assert!(issuing.activate(issued.credential_id()).expect("activate"));
        let authorization = format!("Bearer {}", issued.expose_token());

        let wrong_key_journal: Arc<dyn EventJournal> = journal.clone();
        let wrong_key = PublicApiCredentialManager::bind(
            wrong_key_journal,
            PublicApiAuthenticationKey::new([44_u8; 32]),
        );
        assert!(
            wrong_key
                .authenticator
                .authenticate_authorization(&authorization)
                .is_err()
        );

        let correct_key_journal: Arc<dyn EventJournal> = journal;
        let correct_key = PublicApiCredentialManager::bind(
            correct_key_journal,
            PublicApiAuthenticationKey::new([43_u8; 32]),
        );
        assert!(
            correct_key
                .authenticator
                .authenticate_authorization(&authorization)
                .is_ok()
        );
    }

    #[test]
    fn authentication_key_and_manager_debug_are_redacted() {
        let key = PublicApiAuthenticationKey::new([45_u8; 32]);
        assert_eq!(
            format!("{key:?}"),
            "PublicApiAuthenticationKey(\"[REDACTED]\")"
        );
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let manager = PublicApiCredentialManager::bind(journal, key);
        let debug = format!("{manager:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("45"));
    }

    #[test]
    fn rotation_source_must_authenticate_under_the_same_root_and_application() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let manager = PublicApiCredentialManager::bind(
            Arc::clone(&journal),
            PublicApiAuthenticationKey::new([46_u8; 32]),
        );
        let issued = manager.issue_pending(&grant()).expect("issue pending");
        assert!(manager.activate(issued.credential_id()).expect("activate"));
        assert_eq!(
            manager
                .validate_rotation_source(issued.expose_token(), "app:worker-credential-test")
                .expect("same application"),
            issued.credential_id()
        );
        assert!(
            manager
                .validate_rotation_source(issued.expose_token(), "app:another")
                .is_err()
        );
        let wrong_root =
            PublicApiCredentialManager::bind(journal, PublicApiAuthenticationKey::new([47_u8; 32]));
        assert!(
            wrong_root
                .validate_rotation_source(issued.expose_token(), "app:worker-credential-test")
                .is_err()
        );
        assert!(
            manager
                .validate_rotation_source("cls_v1.malformed.secret", "app:worker-credential-test")
                .is_err()
        );
        assert!(
            manager
                .validate_rotation_source(
                    &"x".repeat(MAX_ROTATION_BEARER_BYTES.saturating_add(1)),
                    "app:worker-credential-test"
                )
                .is_err()
        );
        assert!(!format!("{manager:?}").contains(issued.expose_token()));
    }
}
