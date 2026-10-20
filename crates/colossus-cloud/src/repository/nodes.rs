use super::*;
use crate::{validate_identifier, validation::bounded_fingerprint};

fn stream(project: &str, node: &str) -> String {
    format!("cloud.node:{project}:{node}")
}

impl CloudRepository {
    /// Enroll one node under explicit project administration, with no implicit runtime grant.
    pub async fn register_node(
        &self,
        caller: &CloudCaller,
        mut node: CloudNode,
    ) -> CloudResult<CloudNode> {
        caller.require(CloudPermission::Administer)?;
        validate_identifier(&node.node_id)?;
        validate_identifier(&node.instance_id)?;
        bounded_fingerprint(&node.certificate_sha256)?;
        if node.project_id != caller.project_id()
            || node.revoked
            || node.revision != 0
            || node.policy.is_some()
            || node.policy_observed_at.is_some()
            || node.label.is_empty()
            || node.label.len() > 128
            || node.label.chars().any(char::is_control)
            || node.roles.is_empty()
            || node.roles.len() > 32
        {
            return Err(CloudError::InvalidArgument);
        }
        for role in &node.roles {
            validate_identifier(role)?;
        }
        node.certificate_sha256.make_ascii_lowercase();
        node.revision = 1;
        self.append(
            caller.subject(),
            stream(caller.project_id(), &node.node_id),
            0,
            "cloud.node.enrolled.v1",
            &node,
        )
        .await?;
        Ok(node)
    }

    /// Read one exact project-owned node.
    pub async fn get_node(&self, caller: &CloudCaller, node_id: &str) -> CloudResult<CloudNode> {
        caller.require(CloudPermission::Read)?;
        self.node_for_caller(caller, node_id).await
    }

    /// Reconcile a hosted personal node's advertised roles after a reviewed project edit.
    ///
    /// The trusted hosted manager supplies the fixed node and owner; this operation is
    /// not a browser administration endpoint and grants no runtime tools or API scopes.
    pub async fn update_personal_node_roles(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        owner_user_id: &str,
        expected_revision: u64,
        roles: std::collections::BTreeSet<String>,
    ) -> CloudResult<CloudNode> {
        caller.require(CloudPermission::Administer)?;
        if owner_user_id.is_empty()
            || owner_user_id.len() > 256
            || owner_user_id.chars().any(char::is_control)
            || roles.is_empty()
            || roles.len() > 32
        {
            return Err(CloudError::InvalidArgument);
        }
        for role in &roles {
            validate_identifier(role)?;
        }
        let mut node = self.node(caller.project_id(), node_id).await?;
        if node.owner_user_id.as_deref() != Some(owner_user_id) {
            return Err(CloudError::NotFound);
        }
        if node.revoked {
            return Err(CloudError::PermissionDenied);
        }
        if node.revision != expected_revision {
            return Err(CloudError::Conflict);
        }
        if node.roles == roles {
            return Ok(node);
        }
        node.roles = roles;
        node.revision = node.revision.checked_add(1).ok_or(CloudError::Conflict)?;
        self.append(
            caller.subject(),
            stream(caller.project_id(), node_id),
            expected_revision,
            "cloud.node.roles.v1",
            &node,
        )
        .await?;
        Ok(node)
    }

    /// List one bounded lexical node page, exclusively after a validated node identity.
    pub async fn list_nodes(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudNode>> {
        caller.require(CloudPermission::Read)?;
        if let Some(after) = after {
            validate_identifier(after)?;
        }
        self.query_entities(
            caller,
            EntityQuery {
                kind: EntityKind::Node,
                after: after.map(str::to_owned),
                limit,
                ..Default::default()
            },
        )
        .await?
        .into_iter()
        .map(|record| record.value.try_into())
        .collect()
    }

    /// Durably revoke an exact revision. Existing authenticated streams must recheck this state.
    pub async fn revoke_node(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        revision: u64,
    ) -> CloudResult<CloudNode> {
        caller.require(CloudPermission::Administer)?;
        let mut node = self.node_for_caller(caller, node_id).await?;
        if node.revoked {
            return Ok(node);
        }
        if node.revision != revision {
            return Err(CloudError::Conflict);
        }
        node.revoked = true;
        node.revision += 1;
        self.append(
            caller.subject(),
            stream(caller.project_id(), node_id),
            revision,
            "cloud.node.revoked.v1",
            &node,
        )
        .await?;
        Ok(node)
    }

    /// Verify a TLS-authenticated client leaf against durable project enrollment and local instance.
    pub async fn authenticate_node(
        &self,
        project: &str,
        node_id: &str,
        fingerprint: &str,
        instance_id: &str,
    ) -> CloudResult<CloudNode> {
        let node = self.node(project, node_id).await?;
        if node.revoked || node.certificate_sha256 != fingerprint || node.instance_id != instance_id
        {
            return Err(CloudError::PermissionDenied);
        }
        Ok(node)
    }

    /// A TLS-authenticated enrollment can revoke only its exact fixed node.
    /// Repeated calls remain idempotent after an acknowledgement is lost.
    pub async fn revoke_own_node(
        &self,
        project: &str,
        node_id: &str,
        fingerprint: &str,
        instance_id: &str,
    ) -> CloudResult<()> {
        let mut node = self.node(project, node_id).await?;
        if node.certificate_sha256 != fingerprint || node.instance_id != instance_id {
            return Err(CloudError::PermissionDenied);
        }
        if node.revoked {
            return Ok(());
        }
        let revision = node.revision;
        node.revoked = true;
        node.revision += 1;
        self.append(
            node_id,
            stream(project, node_id),
            revision,
            "cloud.node.revoked.v1",
            &node,
        )
        .await
    }

    pub(super) async fn node_for_caller(
        &self,
        caller: &CloudCaller,
        node_id: &str,
    ) -> CloudResult<CloudNode> {
        let node = self.node(caller.project_id(), node_id).await?;
        if node
            .owner_user_id
            .as_deref()
            .is_some_and(|owner| owner != caller.subject())
        {
            return Err(CloudError::NotFound);
        }
        Ok(node)
    }
    pub(super) async fn node(&self, project: &str, node_id: &str) -> CloudResult<CloudNode> {
        validate_identifier(project)?;
        validate_identifier(node_id)?;
        let (node, revision) = self.read::<CloudNode>(&stream(project, node_id)).await?;
        if node.project_id != project || node.node_id != node_id || node.revision != revision {
            return Err(CloudError::Storage);
        }
        Ok(node)
    }

    pub(super) async fn live_node(&self, node: &CloudNode) -> CloudResult<CloudNode> {
        self.authenticate_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id,
        )
        .await
    }
}
