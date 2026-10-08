//! Conversation metadata and native task/command columns with SDK payloads.
use super::{DomainRow, Metadata, decode_payload, mismatch, payload, signed, unsigned};
use colossus_cloud::{
    CloudError, CloudMessage, CloudResult, CloudTask, CloudThread, PendingCommand,
    storage::EntityValue,
};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName,
    sql_types::{Array, BigInt, Bool, Jsonb, Nullable, Text},
};
use serde_json::Value;

row!(ThreadRow {
    node_id: String => Text,
    host_id: Option<String> => Nullable<Text>,
    workspace_id: Option<String> => Nullable<Text>,
    title: String => Text,
    created_at_text: String => Text,
    updated_at_text: String => Text,
    archived: bool => Bool,
    local_session_id: Option<String> => Nullable<Text>,
    sync_status: String => Text,
    source: String => Text,
    can_continue: bool => Bool,
    active_task_id: Option<String> => Nullable<Text>,
    queued_task_ids: Vec<String> => Array<Text>,
});
impl DomainRow for ThreadRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Thread(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            node_id: value.node_id.clone(),
            host_id: value.host_id.clone(),
            workspace_id: value.workspace_id.clone(),
            title: value.title.clone(),
            created_at_text: value.created_at.clone(),
            updated_at_text: value.updated_at.clone(),
            archived: value.archived,
            local_session_id: value.session_id.clone(),
            sync_status: value.sync_status.clone(),
            source: value.source.clone(),
            can_continue: value.can_continue,
            active_task_id: value.active_task_id.clone(),
            queued_task_ids: value.queued_task_ids.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudThread {
            revision: self.metadata.revision()?,
            thread_id: self.metadata.id,
            project_id: self.metadata.project_id,
            node_id: self.node_id,
            host_id: self.host_id,
            workspace_id: self.workspace_id,
            title: self.title,
            created_at: self.created_at_text,
            updated_at: self.updated_at_text,
            archived: self.archived,
            session_id: self.local_session_id,
            sync_status: self.sync_status,
            source: self.source,
            can_continue: self.can_continue,
            active_task_id: self.active_task_id,
            queued_task_ids: self.queued_task_ids,
        }
        .into())
    }
}

row!(MessageRow {
    role: String => Text,
    message_text: String => Text,
    created_at_text: String => Text,
    task_id: String => Text,
});
impl DomainRow for MessageRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::ThreadMessage(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            role: value.role.clone(),
            message_text: value.text.clone(),
            created_at_text: value.created_at.clone(),
            task_id: value.task_id.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudMessage {
            revision: self.metadata.revision()?,
            message_id: self.metadata.id,
            thread_id: self.metadata.parent_id,
            project_id: self.metadata.project_id,
            role: self.role,
            text: self.message_text,
            created_at: self.created_at_text,
            task_id: self.task_id,
        }
        .into())
    }
}

row!(TaskRow {
    node_id: String => Text,
    subject: String => Text,
    created_at_text: String => Text,
    updated_at_text: String => Text,
    request: Value => Jsonb,
    thread_id: Option<String> => Nullable<Text>,
    source_read_only: bool => Bool,
    history_complete: bool => Bool,
    history_bounded: bool => Bool,
    run_id: Option<String> => Nullable<Text>,
    snapshot: Option<Value> => Nullable<Jsonb>,
    dispatch_error: Option<Value> => Nullable<Jsonb>,
    last_sequence: i64 => BigInt,
    released_bytes: i64 => BigInt,
    output_limited: bool => Bool,
});
impl DomainRow for TaskRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Task(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            node_id: value.node_id.clone(),
            subject: value.subject.clone(),
            created_at_text: value.created_at.clone(),
            updated_at_text: value.updated_at.clone(),
            request: payload(&value.request)?,
            thread_id: value.thread_id.clone(),
            source_read_only: value.source_read_only,
            history_complete: value.history_complete,
            history_bounded: value.history_bounded,
            run_id: value.run_id.clone(),
            snapshot: value.snapshot.as_ref().map(payload).transpose()?,
            dispatch_error: value.dispatch_error.as_ref().map(payload).transpose()?,
            last_sequence: signed(value.last_sequence)?,
            released_bytes: i64::try_from(value.released_bytes)
                .map_err(|_| StoreError::Adapter("cloud numeric bound exceeded".into()))?,
            output_limited: value.output_limited,
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudTask {
            revision: self.metadata.revision()?,
            task_id: self.metadata.id,
            project_id: self.metadata.project_id,
            node_id: self.node_id,
            subject: self.subject,
            created_at: self.created_at_text,
            updated_at: self.updated_at_text,
            request: decode_payload(self.request)?,
            thread_id: self.thread_id,
            source_read_only: self.source_read_only,
            history_complete: self.history_complete,
            history_bounded: self.history_bounded,
            run_id: self.run_id,
            snapshot: self.snapshot.map(decode_payload).transpose()?,
            dispatch_error: self.dispatch_error.map(decode_payload).transpose()?,
            last_sequence: unsigned(self.last_sequence)?,
            released_bytes: usize::try_from(self.released_bytes)
                .map_err(|_| CloudError::Storage)?,
            output_limited: self.output_limited,
        }
        .into())
    }
}

row!(CommandRow {
    task_id: String => Text,
    operation: Value => Jsonb,
    reply: Option<Value> => Nullable<Jsonb>,
});
impl DomainRow for CommandRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Command(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            task_id: value.task_id.clone(),
            operation: payload(&value.command)?,
            reply: value.reply.as_ref().map(payload).transpose()?,
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(PendingCommand {
            revision: self.metadata.revision()?,
            command_id: self.metadata.id,
            node_id: self.metadata.parent_id,
            task_id: self.task_id,
            command: decode_payload(self.operation)?,
            reply: self.reply.map(decode_payload).transpose()?,
        }
        .into())
    }
}
