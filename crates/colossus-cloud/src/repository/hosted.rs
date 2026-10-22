//! Trusted Hosted placement. No enrollment tokens, certificates or transport identity.
use super::*;
use crate::ExecutionKind;
use std::collections::BTreeSet;

impl CloudRepository {
    /// Register the caller-bound Hosted target, preserving its historical Chat identity.
    /// Only trusted service composition calls this operation; it is not an HTTP control.
    /// A legacy self-enrolled Chat target is converted only under its exact owner and
    /// instance binding. Its conversations, tasks and runtime namespace remain unchanged.
    pub async fn register_hosted_target(
        &self,
        administrator: &CloudCaller,
        owner: &str,
        roles: BTreeSet<String>,
    ) -> CloudResult<CloudNode> {
        administrator.require(CloudPermission::Administer)?;
        crate::validate_identifier(owner)?;
        if roles.is_empty() || roles.len() > 32 {
            return Err(CloudError::InvalidArgument);
        }
        for role in &roles {
            crate::validate_identifier(role)?;
        }
        let identity = crate::validation::fingerprint(
            &serde_json::to_vec(&("cloud-chat-v1", administrator.project_id(), owner))
                .map_err(|_| CloudError::InvalidArgument)?,
        );
        let node_id = format!("chat-{identity}");
        let instance_id = format!("hosted-chat-{identity}");
        let legacy_workspace = format!(
            "workspace-{}",
            crate::validation::fingerprint(format!("workspace:{instance_id}").as_bytes())
        );
        for _ in 0..8 {
            let existing = match self.node(administrator.project_id(), &node_id).await {
                Ok(node) => Some(node),
                Err(CloudError::NotFound) => None,
                Err(error) => return Err(error),
            };
            if let Some(node) = &existing {
                if node.owner_user_id.as_deref() != Some(owner)
                    || node.instance_id != instance_id
                    || node
                        .workspace_id
                        .as_ref()
                        .is_some_and(|workspace| workspace != &legacy_workspace)
                    || node.revoked
                {
                    return Err(CloudError::PermissionDenied);
                }
                if node.execution_kind == ExecutionKind::Hosted && node.roles == roles {
                    return Ok(node.clone());
                }
            }
            let revision = existing.as_ref().map_or(0, |node| node.revision);
            let node = CloudNode {
                execution_kind: ExecutionKind::Hosted,
                owner_user_id: Some(owner.into()),
                node_id: node_id.clone(),
                project_id: administrator.project_id().into(),
                instance_id: instance_id.clone(),
                label: "Colossus Hosted".into(),
                certificate_sha256: String::new(),
                roles: roles.clone(),
                revoked: false,
                // Retain historical grouping for ownership/history joins. Fleet
                // projections exclude this target by its execution kind.
                host_id: existing.as_ref().and_then(|node| node.host_id.clone()),
                workspace_id: existing.as_ref().and_then(|node| node.workspace_id.clone()),
                workspace_label: existing
                    .as_ref()
                    .and_then(|node| node.workspace_label.clone()),
                runtime_ready: true,
                policy: None,
                policy_observed_at: None,
                revision: revision.checked_add(1).ok_or(CloudError::Conflict)?,
            };
            match self
                .append(
                    administrator.subject(),
                    format!("cloud.node:{}:{node_id}", administrator.project_id()),
                    revision,
                    "cloud.hosted.registered.v1",
                    &node,
                )
                .await
            {
                Ok(()) => return Ok(node),
                Err(CloudError::Conflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(CloudError::Conflict)
    }

    /// List only Fleet targets, filtering at storage before pagination.
    pub async fn list_connected_nodes(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudNode>> {
        if let Some(after) = after {
            crate::validate_identifier(after)?;
        }
        self.query_entities(
            caller,
            EntityQuery {
                kind: EntityKind::Node,
                after: after.map(str::to_owned),
                status: Some("connected".into()),
                limit,
                ..Default::default()
            },
        )
        .await?
        .into_iter()
        .map(|record| record.value.try_into())
        .collect()
    }
}
