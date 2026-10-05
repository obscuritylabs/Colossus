use crate::{
    CloudCaller, CloudError, CloudNode, CloudPermission, CloudResult, CloudTask, PendingCommand,
};
use colossus_cloud_protocol::{CloudReply, Command};
use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
use colossus_ports::EventJournal;
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

mod delivery;
mod enrollment;
mod nodes;
mod renewal;
mod tasks;

/// Durable cloud application operations; storage adapters remain replaceable.
#[derive(Clone)]
pub struct CloudRepository {
    pub(super) journal: Arc<dyn EventJournal>,
}

impl CloudRepository {
    /// Probe canonical storage without scanning or releasing project payloads.
    pub fn check_readiness(&self) -> CloudResult<()> {
        if self.journal.is_recovery_mode() {
            return Err(CloudError::Storage);
        }
        self.journal.list_stream_ids("cloud.node:", None, 1)?;
        Ok(())
    }
    /// Bind a cloud-specific canonical journal. Never share a worker's writer identity.
    pub fn new(journal: Arc<dyn EventJournal>) -> CloudResult<Self> {
        if journal.is_recovery_mode() {
            return Err(CloudError::Storage);
        }
        Ok(Self { journal })
    }

    pub(super) fn read<T: DeserializeOwned>(&self, stream: &str) -> CloudResult<(T, u64)> {
        let events = self.journal.read_stream_backwards(stream, None, 1)?;
        let event = events.first().ok_or(CloudError::NotFound)?;
        if event.event_version != 1 || !event.event_type.starts_with("cloud.") {
            return Err(CloudError::Storage);
        }
        let value = self.journal.decrypt_payload(event)?;
        Ok((
            serde_json::from_value(value).map_err(|_| CloudError::Storage)?,
            event.stream_version,
        ))
    }

    pub(super) fn event<T: Serialize>(
        &self,
        subject: &str,
        stream: String,
        version: u64,
        kind: &str,
        value: &T,
    ) -> CloudResult<NewEvent> {
        Ok(NewEvent {
            event_version: 1,
            stream_id: stream,
            expected_stream_version: version,
            classification: EventClassification::Domain,
            event_type: kind.into(),
            actor: Actor {
                actor_type: ActorType::Application,
                id: subject.into(),
            },
            context: ExecutionContext::default(),
            payload: serde_json::to_value(value).map_err(|_| CloudError::InvalidArgument)?,
        })
    }

    pub(super) fn append<T: Serialize>(
        &self,
        subject: &str,
        stream: String,
        version: u64,
        kind: &str,
        value: &T,
    ) -> CloudResult<()> {
        self.commit(vec![self.event(subject, stream, version, kind, value)?])?;
        Ok(())
    }
    pub(super) fn commit(&self, events: Vec<NewEvent>) -> Result<(), colossus_ports::StoreError> {
        self.journal.append_batch(events)?;
        self.journal.checkpoint()?;
        Ok(())
    }
}
