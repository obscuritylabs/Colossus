use super::*;
use crate::{validate_identifier, validation::bounded_fingerprint};

fn stream(project: &str, node: &str) -> String {
    format!("cloud.node:{project}:{node}")
}

impl CloudRepository {
    /// Enroll one node under explicit project administration, with no implicit runtime grant.
    pub fn register_node(
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
        )?;
        Ok(node)
    }

    /// Read one exact project-owned node.
    pub fn get_node(&self, caller: &CloudCaller, node_id: &str) -> CloudResult<CloudNode> {
        caller.require(CloudPermission::Read)?;
        self.node(caller.project_id(), node_id)
    }

    /// List one bounded lexical node page, exclusively after a validated node identity.
    pub fn list_nodes(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudNode>> {
        caller.require(CloudPermission::Read)?;
        if let Some(after) = after {
            validate_identifier(after)?;
        }
        let prefix = format!("cloud.node:{}:", caller.project_id());
        let cursor = after.map(|id| format!("{prefix}{id}"));
        self.journal
            .list_stream_ids(&prefix, cursor.as_deref(), limit.min(100))?
            .into_iter()
            .map(|stream| self.read::<CloudNode>(&stream).map(|(node, _)| node))
            .collect()
    }

    /// Durably revoke an exact revision. Existing authenticated streams must recheck this state.
    pub fn revoke_node(
        &self,
        caller: &CloudCaller,
        node_id: &str,
        revision: u64,
    ) -> CloudResult<CloudNode> {
        caller.require(CloudPermission::Administer)?;
        let mut node = self.node(caller.project_id(), node_id)?;
        if node.revoked {
            self.journal.checkpoint()?;
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
        )?;
        Ok(node)
    }

    /// Verify a TLS-authenticated client leaf against durable project enrollment and local instance.
    pub fn authenticate_node(
        &self,
        project: &str,
        node_id: &str,
        fingerprint: &str,
        instance_id: &str,
    ) -> CloudResult<CloudNode> {
        let node = self.node(project, node_id)?;
        if node.revoked || node.certificate_sha256 != fingerprint || node.instance_id != instance_id
        {
            return Err(CloudError::PermissionDenied);
        }
        Ok(node)
    }

    /// A TLS-authenticated enrollment can revoke only its exact fixed node.
    /// Repeated calls remain idempotent after an acknowledgement is lost.
    pub fn revoke_own_node(
        &self,
        project: &str,
        node_id: &str,
        fingerprint: &str,
        instance_id: &str,
    ) -> CloudResult<()> {
        let mut node = self.node(project, node_id)?;
        if node.certificate_sha256 != fingerprint || node.instance_id != instance_id {
            return Err(CloudError::PermissionDenied);
        }
        if node.revoked {
            self.journal.checkpoint()?;
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
    }

    pub(super) fn node(&self, project: &str, node_id: &str) -> CloudResult<CloudNode> {
        validate_identifier(project)?;
        validate_identifier(node_id)?;
        let (node, revision) = self.read::<CloudNode>(&stream(project, node_id))?;
        if node.project_id != project || node.node_id != node_id || node.revision != revision {
            return Err(CloudError::Storage);
        }
        Ok(node)
    }

    pub(super) fn live_node(&self, node: &CloudNode) -> CloudResult<CloudNode> {
        self.authenticate_node(
            &node.project_id,
            &node.node_id,
            &node.certificate_sha256,
            &node.instance_id,
        )
    }
}
