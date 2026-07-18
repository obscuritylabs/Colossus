use super::*;

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

// Preserve the exact nested JSON used for historical authenticated data and hashes.
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

pub(super) fn record_hash(envelope: &EventEnvelope) -> Result<String, StoreError> {
    Ok(sha256_hex(
        &serde_json::to_vec(&RecordHashInput {
            associated_data: associated_data(envelope),
            payload: &envelope.payload,
            previous_hash: &envelope.previous_hash,
        })
        .map_err(adapter_error)?,
    ))
}

pub(super) fn persisted_record_hash(
    envelope: &PersistedEventEnvelope,
) -> Result<String, StoreError> {
    Ok(sha256_hex(
        &serde_json::to_vec(&PersistedRecordHashInput {
            associated_data: persisted_associated_data(envelope),
            payload: &envelope.payload,
            previous_hash: &envelope.previous_hash,
        })
        .map_err(adapter_error)?,
    ))
}

pub(super) fn checkpoint_message(sequence: u64, hash: &str) -> Vec<u8> {
    format!("colossus-checkpoint-v1\n{sequence}\n{hash}\n").into_bytes()
}

#[cfg(test)]
pub(super) fn crash_at_test_fault(point: &str) {
    if std::env::var("COLOSSUS_POSTGRES_TEST_CRASH_POINT").as_deref() == Ok(point) {
        std::process::abort();
    }
}
