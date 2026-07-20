use crate::auth::{CredentialRecord, CredentialRepository, CredentialStoreError};
use colossus_contracts::{
    Actor, ActorType, EventClassification, EventEnvelope, ExecutionContext, NewEvent,
};
use colossus_ports::{EventJournal, StoreError};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use uuid::Uuid;

const STREAM_PREFIX: &str = "public-api-credential:";
const ISSUED_EVENT: &str = "public_api.credential_issued.v1";
const ACTIVATED_EVENT: &str = "public_api.credential_activated.v1";
const REVOKED_EVENT: &str = "public_api.credential_revoked.v1";
const STORE_ACTOR_ID: &str = "public-api-credential-store";
const EVENT_VERSION: u16 = 1;
const ENVELOPE_SCHEMA_VERSION: u16 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialIssued {
    credential: CredentialRecord,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialRevoked {
    credential_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialActivated {
    credential_id: String,
}

struct ReconstructedRecord {
    record: CredentialRecord,
    stream_version: u64,
}

/// Credential verifier repository backed by the authoritative encrypted journal.
///
/// Only the keyed verifier and the application's bounded authorization grant are
/// journaled. Bearer secrets never enter this adapter.
pub struct JournalCredentialRepository {
    journal: Arc<dyn EventJournal>,
}

impl JournalCredentialRepository {
    /// Bind credential records to the runtime's authoritative encrypted journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(credential_id: &str) -> Result<String, CredentialStoreError> {
        validate_credential_id(credential_id)?;
        Ok(format!("{STREAM_PREFIX}{credential_id}"))
    }

    fn load(
        &self,
        credential_id: &str,
    ) -> Result<Option<ReconstructedRecord>, CredentialStoreError> {
        let stream_id = Self::stream(credential_id)?;
        let events = self
            .journal
            .read_stream(&stream_id)
            .map_err(store_failure)?;
        if events.is_empty() {
            return Ok(None);
        }
        reconstruct(self.journal.as_ref(), credential_id, &stream_id, &events).map(Some)
    }

    fn event(
        credential_id: &str,
        expected_stream_version: u64,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<NewEvent, CredentialStoreError> {
        Ok(NewEvent {
            event_version: EVENT_VERSION,
            stream_id: Self::stream(credential_id)?,
            expected_stream_version,
            classification: EventClassification::System,
            event_type: event_type.into(),
            actor: store_actor(),
            context: store_context(credential_id),
            payload,
        })
    }
}

impl fmt::Debug for JournalCredentialRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalCredentialRepository")
            .field("journal", &"[REDACTED]")
            .finish()
    }
}

impl CredentialRepository for JournalCredentialRepository {
    fn insert(&self, record: CredentialRecord) -> Result<(), CredentialStoreError> {
        if !record.is_valid() || !record.is_pending() {
            return Err(CredentialStoreError);
        }
        let credential_id = record.credential_id().to_owned();
        if self.load(&credential_id)?.is_some() {
            return Err(CredentialStoreError);
        }
        let payload = serde_json::to_value(CredentialIssued { credential: record })
            .map_err(redacted_failure)?;
        self.journal
            .append(Self::event(&credential_id, 0, ISSUED_EVENT, payload)?)
            .map_err(store_failure)?;
        Ok(())
    }

    fn get(&self, credential_id: &str) -> Result<Option<CredentialRecord>, CredentialStoreError> {
        Ok(self.load(credential_id)?.map(|state| state.record))
    }

    fn activate(&self, credential_id: &str) -> Result<bool, CredentialStoreError> {
        let Some(state) = self.load(credential_id)? else {
            return Ok(false);
        };
        if state.record.is_active() {
            return Ok(true);
        }
        if state.record.is_revoked() || !state.record.is_pending() {
            return Ok(false);
        }
        let payload = serde_json::to_value(CredentialActivated {
            credential_id: credential_id.into(),
        })
        .map_err(redacted_failure)?;
        let event = Self::event(
            credential_id,
            state.stream_version,
            ACTIVATED_EVENT,
            payload,
        )?;
        match self.journal.append(event) {
            Ok(_) => Ok(true),
            Err(StoreError::Conflict { stream_id, .. })
                if stream_id == Self::stream(credential_id)? =>
            {
                self.load(credential_id)?
                    .map(|record| record.record.is_active())
                    .ok_or(CredentialStoreError)
            }
            Err(error) => Err(store_failure(error)),
        }
    }

    fn revoke(&self, credential_id: &str) -> Result<bool, CredentialStoreError> {
        for _ in 0..3 {
            let Some(state) = self.load(credential_id)? else {
                return Ok(false);
            };
            if state.record.is_revoked() {
                return Ok(true);
            }
            let payload = serde_json::to_value(CredentialRevoked {
                credential_id: credential_id.into(),
            })
            .map_err(redacted_failure)?;
            let event = Self::event(credential_id, state.stream_version, REVOKED_EVENT, payload)?;
            match self.journal.append(event) {
                Ok(_) => return Ok(true),
                Err(StoreError::Conflict { stream_id, .. })
                    if stream_id == Self::stream(credential_id)? =>
                {
                    // Retry from strict replay so revocation wins an activation race.
                    continue;
                }
                Err(error) => return Err(store_failure(error)),
            }
        }
        Err(CredentialStoreError)
    }
}

fn reconstruct(
    journal: &dyn EventJournal,
    credential_id: &str,
    stream_id: &str,
    events: &[EventEnvelope],
) -> Result<ReconstructedRecord, CredentialStoreError> {
    let mut record = None;
    for (index, event) in events.iter().enumerate() {
        let expected_version = u64::try_from(index)
            .map_err(redacted_failure)?
            .saturating_add(1);
        validate_envelope(event, credential_id, stream_id, expected_version)?;
        match event.event_type.as_str() {
            ISSUED_EVENT if index == 0 && record.is_none() => {
                let issued: CredentialIssued =
                    serde_json::from_value(journal.decrypt_payload(event).map_err(store_failure)?)
                        .map_err(redacted_failure)?;
                if issued.credential.credential_id() != credential_id
                    || !issued.credential.is_pending()
                    || !issued.credential.is_valid()
                {
                    return Err(CredentialStoreError);
                }
                record = Some(issued.credential);
            }
            ACTIVATED_EVENT => {
                let activated: CredentialActivated =
                    serde_json::from_value(journal.decrypt_payload(event).map_err(store_failure)?)
                        .map_err(redacted_failure)?;
                if activated.credential_id != credential_id {
                    return Err(CredentialStoreError);
                }
                let record = record.as_mut().ok_or(CredentialStoreError)?;
                if !record.is_pending() {
                    return Err(CredentialStoreError);
                }
                record.mark_active();
            }
            REVOKED_EVENT => {
                let revoked: CredentialRevoked =
                    serde_json::from_value(journal.decrypt_payload(event).map_err(store_failure)?)
                        .map_err(redacted_failure)?;
                if revoked.credential_id != credential_id {
                    return Err(CredentialStoreError);
                }
                let record = record.as_mut().ok_or(CredentialStoreError)?;
                if record.is_revoked() {
                    return Err(CredentialStoreError);
                }
                record.mark_revoked();
            }
            _ => return Err(CredentialStoreError),
        }
    }
    Ok(ReconstructedRecord {
        record: record.ok_or(CredentialStoreError)?,
        stream_version: events
            .last()
            .map(|event| event.stream_version)
            .ok_or(CredentialStoreError)?,
    })
}

fn validate_envelope(
    event: &EventEnvelope,
    credential_id: &str,
    stream_id: &str,
    expected_stream_version: u64,
) -> Result<(), CredentialStoreError> {
    if event.schema_version != ENVELOPE_SCHEMA_VERSION
        || event.event_version != EVENT_VERSION
        || event.stream_id != stream_id
        || event.stream_version != expected_stream_version
        || event.classification != EventClassification::System
        || event.actor != store_actor()
        || event.context != store_context(credential_id)
    {
        return Err(CredentialStoreError);
    }
    Ok(())
}

fn validate_credential_id(credential_id: &str) -> Result<(), CredentialStoreError> {
    let parsed = Uuid::parse_str(credential_id).map_err(redacted_failure)?;
    if parsed.to_string() != credential_id {
        return Err(CredentialStoreError);
    }
    Ok(())
}

fn store_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: STORE_ACTOR_ID.into(),
    }
}

fn store_context(credential_id: &str) -> ExecutionContext {
    ExecutionContext {
        correlation_id: credential_id.into(),
        ..ExecutionContext::default()
    }
}

fn store_failure(_: StoreError) -> CredentialStoreError {
    CredentialStoreError
}

fn redacted_failure<T>(_: T) -> CredentialStoreError {
    CredentialStoreError
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{ApplicationGrant, CredentialAuthenticator, InMemoryCredentialRepository};
    use colossus_api::{ApiScope, ApplicationKind, scopes};
    use colossus_contracts::{ProjectionWorkItem, SignedCheckpoint};
    use colossus_ports::VerificationReport;
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::Value;
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    struct RevocationRaceJournal {
        inner: InMemoryEventJournal,
        revocation_barrier: Barrier,
        transition_attempts: AtomicUsize,
    }

    impl Default for RevocationRaceJournal {
        fn default() -> Self {
            Self {
                inner: InMemoryEventJournal::default(),
                revocation_barrier: Barrier::new(2),
                transition_attempts: AtomicUsize::new(0),
            }
        }
    }

    impl EventJournal for RevocationRaceJournal {
        fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
            if matches!(event.event_type.as_str(), ACTIVATED_EVENT | REVOKED_EVENT)
                && self.transition_attempts.fetch_add(1, Ordering::AcqRel) < 2
            {
                self.revocation_barrier.wait();
            }
            self.inner.append(event)
        }

        fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
            self.inner.append_batch(events)
        }

        fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
            self.inner.read_stream(stream_id)
        }

        fn read_stream_from(
            &self,
            stream_id: &str,
            after_version: u64,
            limit: usize,
        ) -> Result<Vec<EventEnvelope>, StoreError> {
            self.inner.read_stream_from(stream_id, after_version, limit)
        }

        fn read_stream_backwards(
            &self,
            stream_id: &str,
            before_version: Option<u64>,
            limit: usize,
        ) -> Result<Vec<EventEnvelope>, StoreError> {
            self.inner
                .read_stream_backwards(stream_id, before_version, limit)
        }

        fn read_global(
            &self,
            from_sequence: u64,
            limit: usize,
        ) -> Result<Vec<EventEnvelope>, StoreError> {
            self.inner.read_global(from_sequence, limit)
        }

        fn read_projection_work(
            &self,
            from_sequence: u64,
            limit: usize,
        ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
            self.inner.read_projection_work(from_sequence, limit)
        }

        fn head(&self) -> Result<(u64, String), StoreError> {
            self.inner.head()
        }

        fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
            self.inner.decrypt_payload(event)
        }

        fn verify(&self) -> Result<VerificationReport, StoreError> {
            self.inner.verify()
        }

        fn is_recovery_mode(&self) -> bool {
            self.inner.is_recovery_mode()
        }

        fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
            self.inner.checkpoint()
        }
    }

    fn grant() -> ApplicationGrant {
        ApplicationGrant::new(
            "app:journal-test",
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

    fn record() -> CredentialRecord {
        let repository = Arc::new(InMemoryCredentialRepository::default());
        let authenticator = CredentialAuthenticator::new([3_u8; 32], repository.clone());
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issue pending");
        repository
            .get(issued.credential_id())
            .expect("load record")
            .expect("record exists")
    }

    #[test]
    fn verifier_and_grant_survive_reconstruction_without_storing_bearer_secret() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository = Arc::new(JournalCredentialRepository::new(journal.clone()));
        let authenticator = CredentialAuthenticator::new([7_u8; 32], repository);
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issue pending");
        let authorization = format!("Bearer {}", issued.expose_token());

        let events = journal
            .read_stream(&format!("{STREAM_PREFIX}{}", issued.credential_id()))
            .expect("read");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ISSUED_EVENT);
        assert_eq!(events[0].actor.actor_type, ActorType::System);
        let encrypted_envelope = serde_json::to_string(&events[0]).expect("encode envelope");
        let persisted_record =
            serde_json::to_string(&journal.decrypt_payload(&events[0]).expect("decrypt"))
                .expect("encode persisted record");
        assert!(!encrypted_envelope.contains(issued.expose_token()));
        assert!(!persisted_record.contains(issued.expose_token()));
        assert!(!format!("{authenticator:?}").contains(issued.expose_token()));
        assert!(
            authenticator
                .authenticate_authorization(&authorization)
                .is_err()
        );
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("activate")
        );
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("idempotent activate")
        );

        let reopened = CredentialAuthenticator::new(
            [7_u8; 32],
            Arc::new(JournalCredentialRepository::new(journal)),
        );
        let principal = reopened
            .authenticate_authorization(&authorization)
            .expect("authenticate after reconstruction");
        assert_eq!(principal.application_id(), "app:journal-test");
    }

    #[test]
    fn revocation_is_durable_idempotent_and_race_safe() {
        let journal = Arc::new(RevocationRaceJournal::default());
        let repository = Arc::new(JournalCredentialRepository::new(journal.clone()));
        let authenticator = CredentialAuthenticator::new([9_u8; 32], repository.clone());
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issue pending");
        let credential_id = issued.credential_id().to_owned();

        let handles = (0..2)
            .map(|_| {
                let repository = repository.clone();
                let credential_id = credential_id.clone();
                std::thread::spawn(move || repository.revoke(&credential_id))
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert!(handle.join().expect("thread").expect("revoke"));
        }
        assert!(
            repository
                .revoke(&credential_id)
                .expect("idempotent revoke")
        );
        assert!(
            authenticator
                .authenticate_authorization(&format!("Bearer {}", issued.expose_token()))
                .is_err()
        );
        assert_eq!(
            journal
                .read_stream(&format!("{STREAM_PREFIX}{credential_id}"))
                .expect("read")
                .len(),
            2
        );
    }

    #[test]
    fn activation_is_durable_idempotent_and_race_safe() {
        let journal = Arc::new(RevocationRaceJournal::default());
        let repository = Arc::new(JournalCredentialRepository::new(journal.clone()));
        let authenticator = CredentialAuthenticator::new([10_u8; 32], repository.clone());
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issue pending");
        let credential_id = issued.credential_id().to_owned();

        let handles = (0..2)
            .map(|_| {
                let repository = repository.clone();
                let credential_id = credential_id.clone();
                std::thread::spawn(move || repository.activate(&credential_id))
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert!(handle.join().expect("thread").expect("activate"));
        }
        assert!(
            repository
                .activate(&credential_id)
                .expect("idempotent activate")
        );
        assert!(
            authenticator
                .authenticate_authorization(&format!("Bearer {}", issued.expose_token()))
                .is_ok()
        );
        assert_eq!(
            journal
                .read_stream(&format!("{STREAM_PREFIX}{credential_id}"))
                .expect("read")
                .len(),
            2
        );
    }

    #[test]
    fn revocation_wins_a_pending_activation_race() {
        let journal = Arc::new(RevocationRaceJournal::default());
        let repository = Arc::new(JournalCredentialRepository::new(journal));
        let authenticator = CredentialAuthenticator::new([11_u8; 32], repository.clone());
        let issued = authenticator
            .issue_pending(&grant())
            .expect("issue pending");
        let credential_id = issued.credential_id().to_owned();

        let activating = {
            let repository = repository.clone();
            let credential_id = credential_id.clone();
            std::thread::spawn(move || repository.activate(&credential_id))
        };
        let revoking = {
            let repository = repository.clone();
            let credential_id = credential_id.clone();
            std::thread::spawn(move || repository.revoke(&credential_id))
        };
        let _ = activating.join().expect("activation thread");
        assert!(revoking.join().expect("revocation thread").expect("revoke"));
        assert!(
            repository
                .get(&credential_id)
                .expect("load")
                .expect("record")
                .is_revoked()
        );
        assert!(
            authenticator
                .authenticate_authorization(&format!("Bearer {}", issued.expose_token()))
                .is_err()
        );
    }

    #[test]
    fn malformed_history_fails_closed_without_diagnostics() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository = JournalCredentialRepository::new(journal.clone());
        let malformed_history_record = record();
        let credential_id = malformed_history_record.credential_id().to_owned();
        repository.insert(malformed_history_record).expect("insert");
        journal
            .append(
                JournalCredentialRepository::event(
                    &credential_id,
                    1,
                    "public_api.credential_unknown.v1",
                    serde_json::json!({"credential_id": credential_id}),
                )
                .expect("event"),
            )
            .expect("append malformed");

        let error = repository
            .get(&credential_id)
            .expect_err("unknown event must fail");
        assert_eq!(error.to_string(), "credential repository is unavailable");
        assert_eq!(
            format!("{error:?}"),
            "CredentialStoreError",
            "debug output remains generic"
        );
    }

    #[test]
    fn application_actor_or_mismatched_payload_is_rejected() {
        let journal = Arc::new(InMemoryEventJournal::default());
        let repository = JournalCredentialRepository::new(journal.clone());
        let malformed_actor_record = record();
        let credential_id = malformed_actor_record.credential_id().to_owned();
        let mut event = JournalCredentialRepository::event(
            &credential_id,
            0,
            ISSUED_EVENT,
            serde_json::to_value(CredentialIssued {
                credential: malformed_actor_record,
            })
            .expect("payload"),
        )
        .expect("event");
        event.actor = Actor {
            actor_type: ActorType::Application,
            id: "app:journal-test".into(),
        };
        journal.append(event).expect("append malformed actor");
        assert!(repository.get(&credential_id).is_err());

        let other_id = Uuid::now_v7().to_string();
        let other_journal = Arc::new(InMemoryEventJournal::default());
        let other_repository = JournalCredentialRepository::new(other_journal.clone());
        let mismatched_record = record();
        other_journal
            .append(
                JournalCredentialRepository::event(
                    &other_id,
                    0,
                    ISSUED_EVENT,
                    serde_json::to_value(CredentialIssued {
                        credential: mismatched_record,
                    })
                    .expect("payload"),
                )
                .expect("event"),
            )
            .expect("append mismatched record");
        assert!(other_repository.get(&other_id).is_err());
    }

    #[test]
    fn noncanonical_or_injected_ids_are_rejected_before_journal_access() {
        let repository =
            JournalCredentialRepository::new(Arc::new(InMemoryEventJournal::default()));
        for credential_id in [
            "not-a-uuid",
            "00000000-0000-0000-0000-000000000000/other",
            "019f7837-0BB0-7821-9e06-874534164de5",
        ] {
            assert!(repository.get(credential_id).is_err());
            assert!(repository.revoke(credential_id).is_err());
        }
    }
}
