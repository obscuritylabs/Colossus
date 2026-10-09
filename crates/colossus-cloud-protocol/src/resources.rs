//! Closed, bounded management operations over the existing authenticated connector.
//! These are online requests, never queued tasks or generic SDK/RPC forwarding.
use colossus_sdk::{
    ApiError, CreateWorkflowScheduleRequest, DeleteWorkflowScheduleRequest,
    SetWorkflowScheduleEnabledRequest, StartWorkflowRunRequest,
};
use serde::{Deserialize, Serialize};

/// Capability gates the additive envelopes; old peers cannot silently ignore mutations.
pub const RESOURCE_CAPABILITY: &str = "runtime.resources.v1";
/// Independent unary management admission, leaving run watches unaffected.
pub const MAX_RESOURCE_REQUESTS: usize = 4;
/// Definition and immutable-input request budget, including the JSON envelope.
pub const MAX_RESOURCE_REQUEST_BYTES: usize = 256 * 1024;

/// Caller-bound public resources. Runtime selection and owner identity are not arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum ResourceOperation {
    Context,
    ListWorkflows {
        after: Option<String>,
    },
    GetWorkflow {
        id: String,
    },
    ValidateWorkflow {
        yaml: String,
    },
    RegisterWorkflow {
        yaml: String,
        expected_hash: String,
        idempotency_key: String,
    },
    ListSchedules {
        after: Option<String>,
    },
    GetSchedule {
        id: String,
    },
    CreateSchedule {
        request: Box<CreateWorkflowScheduleRequest>,
    },
    SetScheduleEnabled {
        request: SetWorkflowScheduleEnabledRequest,
    },
    DeleteSchedule {
        request: DeleteWorkflowScheduleRequest,
    },
    ListWorkflowRuns {
        workflow_id: String,
        after: Option<String>,
    },
    GetWorkflowRun {
        id: String,
    },
    StartWorkflowRun {
        request: StartWorkflowRunRequest,
    },
    ListPlugins,
}
impl ResourceOperation {
    /// True only for public mutations; no automatically retried management writes.
    pub fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::RegisterWorkflow { .. }
                | Self::CreateSchedule { .. }
                | Self::SetScheduleEnabled { .. }
                | Self::DeleteSchedule { .. }
                | Self::StartWorkflowRun { .. }
        )
    }
    /// Additional cloud authority required beyond project read permission.
    pub fn permission(&self) -> ResourcePermission {
        match self {
            Self::CreateSchedule { .. } | Self::StartWorkflowRun { .. } => {
                ResourcePermission::Execute
            }
            Self::RegisterWorkflow { .. }
            | Self::SetScheduleEnabled { .. }
            | Self::DeleteSchedule { .. } => ResourcePermission::Control,
            _ => ResourcePermission::Read,
        }
    }
    /// Reject malformed/oversized strings before admission or forwarding.
    pub fn validate(&self) -> bool {
        let id = |s: &str| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control);
        let cursor = |s: &Option<String>| {
            s.as_ref().is_none_or(|v| {
                !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control)
            })
        };
        let yaml = |s: &str| !s.trim().is_empty() && s.len() <= 192 * 1024;
        match self {
            Self::Context | Self::ListPlugins => true,
            Self::ListWorkflows { after } | Self::ListSchedules { after } => cursor(after),
            Self::GetWorkflow { id: value }
            | Self::GetSchedule { id: value }
            | Self::GetWorkflowRun { id: value } => id(value),
            Self::ValidateWorkflow { yaml: value } => yaml(value),
            Self::RegisterWorkflow {
                yaml: value,
                expected_hash,
                idempotency_key,
            } => yaml(value) && id(expected_hash) && id(idempotency_key),
            Self::CreateSchedule { request } => {
                id(&request.schedule_id) && id(&request.idempotency_key)
            }
            Self::SetScheduleEnabled { request } => id(&request.schedule_id) && id(&request.etag),
            Self::DeleteSchedule { request } => id(&request.schedule_id) && id(&request.etag),
            Self::ListWorkflowRuns { workflow_id, after } => id(workflow_id) && cursor(after),
            Self::StartWorkflowRun { request } => {
                id(&request.workflow_id)
                    && id(&request.expected_hash)
                    && id(&request.idempotency_key)
            }
        }
    }
}
/// Project-level gate; the local SDK enforces its separate scopes and ownership again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum ResourcePermission {
    Read,
    Execute,
    Control,
}
/// Released resource response, or a safe public API error with outcome certainty.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum ResourceReply {
    Result { value: serde_json::Value },
    Failed { error: ApiError },
}
