use super::*;

/// Checkpoint signer placeholder for journal modes where checkpoints are disabled.
#[derive(Default)]
pub struct DisabledCheckpointSigner;

impl CheckpointSigner for DisabledCheckpointSigner {
    fn key_id(&self) -> &str {
        "none"
    }

    fn sign(&self, _message: &[u8]) -> Result<Vec<u8>, StoreError> {
        Err(StoreError::Adapter(
            "checkpoint signing is disabled for plaintext storage".into(),
        ))
    }

    fn verify(&self, _message: &[u8], _signature: &[u8]) -> Result<(), StoreError> {
        Err(StoreError::Verification(
            "plaintext storage cannot contain signed checkpoints".into(),
        ))
    }
}

/// Ed25519 checkpoint signer created from explicit secret key bytes.
pub struct Ed25519CheckpointSigner {
    key_id: String,
    signing_key: SigningKey,
}

impl Ed25519CheckpointSigner {
    /// Construct a signer from a 32-byte Ed25519 secret key.
    pub fn new(key_id: impl Into<String>, secret: [u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            signing_key: SigningKey::from_bytes(&secret),
        }
    }

    /// Public verification key bytes for external anchor verification.
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}

impl CheckpointSigner for Ed25519CheckpointSigner {
    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, StoreError> {
        Ok(self.signing_key.sign(message).to_bytes().to_vec())
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), StoreError> {
        let signature = Signature::from_slice(signature)
            .map_err(|error| StoreError::Verification(error.to_string()))?;
        let key: VerifyingKey = self.signing_key.verifying_key();
        key.verify(message, &signature)
            .map_err(|error| StoreError::Verification(error.to_string()))
    }
}

#[derive(Serialize)]
pub(super) struct AssociatedData<'a> {
    pub(super) schema_version: u16,
    pub(super) event_version: u16,
    pub(super) event_id: &'a str,
    pub(super) global_sequence: u64,
    pub(super) stream_id: &'a str,
    pub(super) stream_version: u64,
    pub(super) classification: &'a colossus_contracts::EventClassification,
    pub(super) event_type: &'a str,
    pub(super) actor: &'a colossus_contracts::Actor,
    pub(super) context: &'a colossus_contracts::ExecutionContext,
    pub(super) occurred_at: &'a str,
}

#[derive(Serialize)]
pub(super) struct RecordHashInput<'a> {
    pub(super) associated_data: AssociatedData<'a>,
    pub(super) payload: &'a EncryptedPayload,
    pub(super) previous_hash: &'a str,
}

// Preserve the exact nested JSON used when a record was encrypted and hashed. Reconstructing
// these values through evolving typed contracts can add defaulted fields and invalidate valid
// historical evidence.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PersistedEventEnvelope {
    pub(super) schema_version: u16,
    pub(super) event_version: u16,
    pub(super) event_id: String,
    pub(super) global_sequence: u64,
    pub(super) stream_id: String,
    pub(super) stream_version: u64,
    pub(super) classification: colossus_contracts::EventClassification,
    pub(super) event_type: String,
    pub(super) actor: Box<RawValue>,
    pub(super) context: Box<RawValue>,
    pub(super) occurred_at: String,
    pub(super) payload: Box<RawValue>,
    pub(super) previous_hash: String,
    pub(super) record_hash: String,
}

#[derive(Serialize)]
pub(super) struct PersistedAssociatedData<'a> {
    pub(super) schema_version: u16,
    pub(super) event_version: u16,
    pub(super) event_id: &'a str,
    pub(super) global_sequence: u64,
    pub(super) stream_id: &'a str,
    pub(super) stream_version: u64,
    pub(super) classification: &'a colossus_contracts::EventClassification,
    pub(super) event_type: &'a str,
    pub(super) actor: &'a RawValue,
    pub(super) context: &'a RawValue,
    pub(super) occurred_at: &'a str,
}

#[derive(Serialize)]
pub(super) struct PersistedRecordHashInput<'a> {
    pub(super) associated_data: PersistedAssociatedData<'a>,
    pub(super) payload: &'a RawValue,
    pub(super) previous_hash: &'a str,
}

pub(super) fn associated_data(envelope: &EventEnvelope) -> AssociatedData<'_> {
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

pub(super) fn record_hash(envelope: &EventEnvelope) -> Result<String, StoreError> {
    let input = RecordHashInput {
        associated_data: associated_data(envelope),
        payload: &envelope.payload,
        previous_hash: &envelope.previous_hash,
    };
    Ok(sha256_hex(
        &serde_json::to_vec(&input).map_err(adapter_error)?,
    ))
}

pub(super) fn persisted_associated_data(
    envelope: &PersistedEventEnvelope,
) -> PersistedAssociatedData<'_> {
    PersistedAssociatedData {
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

pub(super) fn persisted_record_hash(
    envelope: &PersistedEventEnvelope,
) -> Result<String, StoreError> {
    let input = PersistedRecordHashInput {
        associated_data: persisted_associated_data(envelope),
        payload: &envelope.payload,
        previous_hash: &envelope.previous_hash,
    };
    Ok(sha256_hex(
        &serde_json::to_vec(&input).map_err(adapter_error)?,
    ))
}

pub(super) fn checkpoint_message(sequence: u64, hash: &str) -> Vec<u8> {
    format!("colossus-checkpoint-v1\n{sequence}\n{hash}\n").into_bytes()
}

#[cfg(test)]
pub(super) fn crash_at_test_fault(point: &str) {
    if std::env::var("COLOSSUS_REDB_TEST_CRASH_POINT").as_deref() == Ok(point) {
        std::process::abort();
    }
}
