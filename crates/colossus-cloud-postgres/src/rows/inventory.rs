//! Native host, runtime and workspace rows.
use super::{DomainRow, Metadata, decode_payload, mismatch, payload, set, signed, unsigned};
use colossus_cloud::{CloudHost, CloudNode, CloudResult, CloudWorkspace, storage::EntityValue};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName,
    sql_types::{Array, BigInt, Bool, Jsonb, Nullable, Text},
};
use serde_json::Value;

row!(HostRow {
    host_name: String => Text,
    platform: String => Text,
    deployment_kind: String => Text,
    last_seen_at: i64 => BigInt,
});
impl DomainRow for HostRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Host(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            host_name: value.label.clone(),
            platform: value.platform.clone(),
            deployment_kind: value.deployment_kind.clone(),
            last_seen_at: signed(value.last_seen_at)?,
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudHost {
            revision: self.metadata.revision()?,
            host_id: self.metadata.id,
            project_id: self.metadata.project_id,
            label: self.host_name,
            platform: self.platform,
            deployment_kind: self.deployment_kind,
            last_seen_at: unsigned(self.last_seen_at)?,
        }
        .into())
    }
}

row!(NodeRow {
    instance_id: String => Text,
    label: String => Text,
    certificate_sha256: String => Text,
    roles: Vec<String> => Array<Text>,
    revoked: bool => Bool,
    host_id: Option<String> => Nullable<Text>,
    workspace_id: Option<String> => Nullable<Text>,
    workspace_label: Option<String> => Nullable<Text>,
    runtime_ready: bool => Bool,
    policy: Option<Value> => Nullable<Jsonb>,
    policy_observed_at: Option<i64> => Nullable<BigInt>,
});
impl DomainRow for NodeRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Node(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            instance_id: value.instance_id.clone(),
            label: value.label.clone(),
            certificate_sha256: value.certificate_sha256.clone(),
            roles: value.roles.iter().cloned().collect(),
            revoked: value.revoked,
            host_id: value.host_id.clone(),
            workspace_id: value.workspace_id.clone(),
            workspace_label: value.workspace_label.clone(),
            runtime_ready: value.runtime_ready,
            policy: value.policy.as_ref().map(payload).transpose()?,
            policy_observed_at: value.policy_observed_at.map(signed).transpose()?,
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudNode {
            revision: self.metadata.revision()?,
            node_id: self.metadata.id,
            project_id: self.metadata.project_id,
            instance_id: self.instance_id,
            label: self.label,
            certificate_sha256: self.certificate_sha256,
            roles: set(self.roles)?,
            revoked: self.revoked,
            host_id: self.host_id,
            workspace_id: self.workspace_id,
            workspace_label: self.workspace_label,
            runtime_ready: self.runtime_ready,
            policy: self.policy.map(decode_payload).transpose()?,
            policy_observed_at: self.policy_observed_at.map(unsigned).transpose()?,
        }
        .into())
    }
}

row!(WorkspaceRow {
    host_id: String => Text,
    node_id: String => Text,
    workspace_name: String => Text,
    sharing_mode: String => Text,
});
impl DomainRow for WorkspaceRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Workspace(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            host_id: value.host_id.clone(),
            node_id: value.node_id.clone(),
            workspace_name: value.label.clone(),
            sharing_mode: value.sharing.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudWorkspace {
            revision: self.metadata.revision()?,
            workspace_id: self.metadata.id,
            project_id: self.metadata.project_id,
            host_id: self.host_id,
            node_id: self.node_id,
            label: self.workspace_name,
            sharing: self.sharing_mode,
        }
        .into())
    }
}
