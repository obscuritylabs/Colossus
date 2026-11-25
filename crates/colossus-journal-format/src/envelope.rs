use colossus_contracts::{
    Actor, EncryptedPayload, EventClassification, EventEnvelope, ExecutionContext, NewEvent,
};
use colossus_ports::{JournalPayloadProtection, KeyProvider, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::protection::protect_payload;

/// The previous record hash for a new journal.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Serialize)]
struct AssociatedData<'a, A: ?Sized, C: ?Sized> {
    schema_version: u16,
    event_version: u16,
    event_id: &'a str,
    global_sequence: u64,
    stream_id: &'a str,
    stream_version: u64,
    classification: &'a EventClassification,
    event_type: &'a str,
    actor: &'a A,
    context: &'a C,
    occurred_at: &'a str,
}

#[derive(Serialize)]
struct RecordHashInput<'a, A: ?Sized, C: ?Sized, P: ?Sized> {
    associated_data: AssociatedData<'a, A, C>,
    payload: &'a P,
    previous_hash: &'a str,
}

/// An envelope that retains the exact nested JSON covered by its hash and encryption.
///
/// Reconstructing actor or context through newer typed contracts can add defaulted
/// fields and invalidate historical evidence.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedEnvelope<'a> {
    schema_version: u16,
    event_version: u16,
    event_id: String,
    global_sequence: u64,
    stream_id: String,
    stream_version: u64,
    classification: EventClassification,
    event_type: String,
    #[serde(borrow)]
    actor: &'a RawValue,
    #[serde(borrow)]
    context: &'a RawValue,
    occurred_at: String,
    #[serde(borrow)]
    payload: &'a RawValue,
    previous_hash: String,
    pub(crate) record_hash: String,
}

fn associated_data(envelope: &EventEnvelope) -> AssociatedData<'_, Actor, ExecutionContext> {
    AssociatedData {
        schema_version: envelope.schema_version,
        event_version: envelope.event_version,
        event_id: &envelope.event_id,
        global_sequence: envelope.global_sequence,
        stream_id: &envelope.stream_id,
        stream_version: envelope.stream_version,
        classification: &envelope.classification,
        event_type: &envelope.event_type,
        actor: &envelope.actor,
        context: &envelope.context,
        occurred_at: &envelope.occurred_at,
    }
}

impl PersistedEnvelope<'_> {
    fn associated_data(&self) -> AssociatedData<'_, RawValue, RawValue> {
        AssociatedData {
            schema_version: self.schema_version,
            event_version: self.event_version,
            event_id: &self.event_id,
            global_sequence: self.global_sequence,
            stream_id: &self.stream_id,
            stream_version: self.stream_version,
            classification: &self.classification,
            event_type: &self.event_type,
            actor: self.actor,
            context: self.context,
            occurred_at: &self.occurred_at,
        }
    }

    /// Encode the authenticated metadata using the actor and context as originally stored.
    pub fn associated_data_bytes(&self) -> Result<Vec<u8>, StoreError> {
        encode(&self.associated_data())
    }

    /// Recompute the hash of the complete canonical record.
    pub fn compute_record_hash(&self) -> Result<String, StoreError> {
        let input = RecordHashInput {
            associated_data: self.associated_data(),
            payload: self.payload,
            previous_hash: &self.previous_hash,
        };
        Ok(sha256_hex(&encode(&input)?))
    }
}

/// Assign durable identity, protect the payload and hash an event into the journal chain.
pub fn seal_event(
    event: NewEvent,
    sequence: u64,
    stream_version: u64,
    previous_hash: &str,
    protection: JournalPayloadProtection,
    keys: &dyn KeyProvider,
) -> Result<EventEnvelope, StoreError> {
    let mut envelope = EventEnvelope {
        schema_version: 1,
        event_version: event.event_version,
        event_id: Uuid::now_v7().to_string(),
        global_sequence: sequence,
        stream_id: event.stream_id,
        stream_version,
        classification: event.classification,
        event_type: event.event_type,
        actor: event.actor,
        context: event.context,
        occurred_at: utc_now()?,
        payload: EncryptedPayload {
            key_id: String::new(),
            algorithm: String::new(),
            nonce: String::new(),
            ciphertext: String::new(),
            plaintext_hash: String::new(),
        },
        previous_hash: previous_hash.to_owned(),
        record_hash: String::new(),
    };
    let plaintext = encode(&event.payload)?;
    let associated_data = associated_data(&envelope);
    let aad = if protection == JournalPayloadProtection::Encrypted {
        encode(&associated_data)?
    } else {
        Vec::new()
    };
    let payload = protect_payload(protection, keys, &plaintext, &aad)?;
    envelope.record_hash = sha256_hex(&encode(&RecordHashInput {
        associated_data,
        payload: &payload,
        previous_hash,
    })?);
    envelope.payload = payload;
    Ok(envelope)
}

/// The version-one message covered by a journal checkpoint signature.
pub fn checkpoint_message(sequence: u64, hash: &str) -> Vec<u8> {
    format!("colossus-checkpoint-v1\n{sequence}\n{hash}\n").into_bytes()
}

/// Timestamp used by the journal's durable records and checkpoints.
pub fn utc_now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| StoreError::Adapter("cannot format journal timestamp".into()))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value)
        .map_err(|_| StoreError::Adapter("cannot encode canonical journal JSON".into()))
}
