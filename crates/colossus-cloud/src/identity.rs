use crate::{CloudError, CloudResult, validate_identifier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Cloud permission independent of the runtime's local role/tool/scope ceiling.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudPermission {
    /// Read the project's registered runtimes and released tasks.
    Read,
    /// Submit agent tasks beneath the selected node's grant.
    Execute,
    /// Cooperatively cancel project tasks and answer ordinary prompts.
    Control,
    /// Answer exact runtime-bound effect approvals.
    Approve,
    /// Enroll and revoke project nodes.
    Administer,
}

/// Project authority constructed only after the host authenticates an identity.
///
/// Browser request bodies never supply this context. Verified identity and persisted
/// project membership are resolved by host authentication, never by request fields.
#[derive(Clone, Debug)]
pub struct CloudCaller {
    pub(crate) subject: String,
    pub(crate) project_id: String,
    permissions: BTreeSet<CloudPermission>,
}

impl CloudCaller {
    /// Bind one verified subject to an explicit project membership and permission set.
    pub fn new(
        subject: String,
        project_id: String,
        permissions: BTreeSet<CloudPermission>,
    ) -> CloudResult<Self> {
        validate_identifier(&project_id)?;
        if subject.is_empty() || subject.len() > 256 || subject.chars().any(char::is_control) {
            return Err(CloudError::InvalidArgument);
        }
        Ok(Self {
            subject,
            project_id,
            permissions,
        })
    }

    /// Authenticated subject, suitable for bounded audit correlation.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Exact project selected from authenticated membership.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Enforce one exact cloud permission. Administer is not implicit execution.
    pub fn require(&self, permission: CloudPermission) -> CloudResult<()> {
        self.permissions
            .contains(&permission)
            .then_some(())
            .ok_or(CloudError::PermissionDenied)
    }
}
