use crate::service::CommunicationService;
use colossus_contracts::{
    Actor, AgentParticipant, EventClassification, ExecutionContext, NewEvent,
};
use colossus_ports::StoreError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingInput {
    pub id: String,
    pub bytes: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParticipantState {
    pub participant: AgentParticipant,
    pub pending: Vec<PendingInput>,
    pub accepted: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_origin: Option<colossus_contracts::AgentMessageOrigin>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScopeState {
    pub root_run_id: String,
    pub owner: Actor,
    pub participants: Vec<String>,
    pub accepted: u64,
    pub pending_bytes: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttemptIndex {
    pub participant_id: String,
    pub generation: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageClaim {
    pub fingerprint: String,
    pub message_id: String,
}

pub(crate) fn hash(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

pub(crate) fn root_stream(id: &str) -> String {
    format!("agent-scope:{}", hash(id.as_bytes()))
}
pub(crate) fn participant_stream(id: &str) -> String {
    format!("agent-participant:{id}")
}
pub(crate) fn message_stream(id: &str) -> String {
    format!("agent-message:{id}")
}
pub(crate) fn inbox_stream(id: &str) -> String {
    format!("agent-inbox:{id}")
}
pub(crate) fn run_stream(id: &str) -> String {
    format!("agent-run:{}", hash(id.as_bytes()))
}
pub(crate) fn job_stream(id: &str) -> String {
    format!("agent-job:{}", hash(id.as_bytes()))
}
pub(crate) fn feed_stream(id: &str) -> String {
    format!("agent-feed:{}", hash(id.as_bytes()))
}

pub(crate) fn timestamp() -> Result<String, StoreError> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| StoreError::Adapter("communication timestamp unavailable".into()))
}

pub(crate) fn event<T: Serialize>(
    stream: String,
    version: u64,
    kind: &str,
    owner: &Actor,
    context: &ExecutionContext,
    record: &T,
) -> Result<NewEvent, StoreError> {
    Ok(NewEvent {
        event_version: 1,
        stream_id: stream,
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: format!("agent.communication.{kind}.v1"),
        actor: owner.clone(),
        context: context.clone(),
        payload: serde_json::to_value(record)
            .map_err(|_| StoreError::Adapter("communication encoding failed".into()))?,
    })
}

impl CommunicationService {
    pub(crate) fn load<T: DeserializeOwned>(
        &self,
        stream: &str,
    ) -> Result<Option<(T, u64)>, StoreError> {
        self.journal
            .read_stream_backwards(stream, None, 1)?
            .first()
            .map(|envelope| {
                if envelope.event_version != 1
                    || !envelope.event_type.starts_with("agent.communication.")
                {
                    return Err(StoreError::Verification(
                        "invalid communication event".into(),
                    ));
                }
                let value = self.journal.decrypt_payload(envelope)?;
                let record = serde_json::from_value(value)
                    .map_err(|_| StoreError::Verification("invalid communication record".into()))?;
                Ok((record, envelope.stream_version))
            })
            .transpose()
    }

    pub(crate) fn participant(&self, id: &str) -> Result<(ParticipantState, u64), StoreError> {
        self.load(&participant_stream(id))?
            .ok_or_else(|| StoreError::NotFound("agent participant".into()))
    }

    pub(crate) fn participant_for_run(
        &self,
        id: &str,
    ) -> Result<Option<(ParticipantState, u64)>, StoreError> {
        self.load::<AttemptIndex>(&run_stream(id))?
            .map(|(index, _)| self.participant(&index.participant_id))
            .transpose()
    }

    pub(crate) fn scope(&self, id: &str) -> Result<(ScopeState, u64), StoreError> {
        self.load(&root_stream(id))?
            .ok_or_else(|| StoreError::NotFound("agent communication scope".into()))
    }

    pub(crate) fn version(&self, stream: &str) -> Result<u64, StoreError> {
        Ok(self
            .journal
            .read_stream_backwards(stream, None, 1)?
            .first()
            .map_or(0, |event| event.stream_version))
    }

    pub(crate) fn wake(&self) {
        self.changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}
