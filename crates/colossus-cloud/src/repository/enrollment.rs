use super::*;
use crate::{
    CertificateRedemption, Enrollment, validate_identifier, validation::bounded_fingerprint,
};

impl CloudRepository {
    /// Allocate an expiring invitation beneath explicit administration authority.
    pub async fn invite(
        &self,
        caller: &CloudCaller,
        invitation: Enrollment,
        now: u64,
    ) -> CloudResult<()> {
        if invitation.owner_user_id.is_some() {
            return Err(CloudError::InvalidArgument);
        }
        self.invite_with_owner(caller, invitation, now).await
    }
    /// Provision an invitation for one authenticated account using trusted administration.
    /// The host derives the owner; a browser must never supply this identity.
    pub async fn invite_personal(
        &self,
        caller: &CloudCaller,
        mut invitation: Enrollment,
        owner_user_id: &str,
        now: u64,
    ) -> CloudResult<()> {
        if owner_user_id.is_empty()
            || owner_user_id.len() > 256
            || owner_user_id.chars().any(char::is_control)
        {
            return Err(CloudError::InvalidArgument);
        }
        invitation.owner_user_id = Some(owner_user_id.into());
        self.invite_with_owner(caller, invitation, now).await
    }
    async fn invite_with_owner(
        &self,
        caller: &CloudCaller,
        invitation: Enrollment,
        now: u64,
    ) -> CloudResult<()> {
        caller.require(CloudPermission::Administer)?;
        bounded_fingerprint(&invitation.token_hash)?;
        validate_identifier(&invitation.node_id)?;
        if invitation.project_id != caller.project_id()
            || invitation.redeemed_certificate.is_some()
            || invitation.redeemed_csr.is_some()
            || invitation.certificate_pem.is_some()
            || invitation.expires_at <= now
            || invitation.expires_at > now.saturating_add(600)
            || invitation.label.is_empty()
            || invitation.label.len() > 128
            || invitation.label.chars().any(char::is_control)
            || invitation.roles.is_empty()
            || invitation.roles.len() > 32
        {
            return Err(CloudError::InvalidArgument);
        }
        for role in &invitation.roles {
            validate_identifier(role)?;
        }
        self.append(
            caller.subject(),
            format!("cloud.invitation:{}", invitation.token_hash),
            0,
            "cloud.invitation.created.v1",
            &invitation,
        )
        .await
    }

    /// Extend only an exact hosted personal invitation while preserving pending CSR custody.
    ///
    /// The trusted hosted manager supplies the retained token hash and fixed placement.
    /// Already redeemed invitations remain unchanged so exact lost-response recovery uses
    /// the original certificate; this is not an ordinary browser invitation endpoint.
    pub async fn renew_personal_enrollment(
        &self,
        caller: &CloudCaller,
        token_hash: &str,
        owner_user_id: &str,
        node_id: &str,
        expires_at: u64,
        now: u64,
    ) -> CloudResult<Enrollment> {
        caller.require(CloudPermission::Administer)?;
        bounded_fingerprint(token_hash)?;
        validate_identifier(node_id)?;
        if owner_user_id.is_empty()
            || owner_user_id.len() > 256
            || owner_user_id.chars().any(char::is_control)
            || expires_at <= now
            || expires_at > now.saturating_add(600)
        {
            return Err(CloudError::InvalidArgument);
        }
        let key = format!("cloud.invitation:{token_hash}");
        let (mut invitation, revision) = self.read::<Enrollment>(&key).await?;
        if invitation.token_hash != token_hash {
            return Err(CloudError::Storage);
        }
        if invitation.project_id != caller.project_id()
            || invitation.node_id != node_id
            || invitation.owner_user_id.as_deref() != Some(owner_user_id)
        {
            return Err(CloudError::NotFound);
        }
        if invitation.redeemed_certificate.is_some() || invitation.expires_at >= expires_at {
            return Ok(invitation);
        }
        invitation.expires_at = expires_at;
        self.append(
            caller.subject(),
            key,
            revision,
            "cloud.invitation.renewed.v1",
            &invitation,
        )
        .await?;
        Ok(invitation)
    }

    /// Atomically consume an invitation and enroll its fixed placement. A repeated
    /// redeem with the same certificate/instance reconciles a lost HTTP response.
    pub async fn redeem(
        &self,
        token_hash: &str,
        instance_id: &str,
        certificate: CertificateRedemption,
        now: u64,
    ) -> CloudResult<(CloudNode, String)> {
        bounded_fingerprint(token_hash)?;
        bounded_fingerprint(&certificate.fingerprint)?;
        bounded_fingerprint(&certificate.csr_sha256)?;
        if certificate.certificate_pem.len() > 16384 {
            return Err(CloudError::InvalidArgument);
        }
        validate_identifier(instance_id)?;
        let invitation_stream = format!("cloud.invitation:{token_hash}");
        let (mut invitation, revision) = self
            .read::<Enrollment>(&invitation_stream)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        if let Some(existing) = &invitation.redeemed_certificate {
            if invitation.redeemed_csr.as_deref() != Some(&certificate.csr_sha256) {
                return Err(CloudError::PermissionDenied);
            }
            return self
                .authenticate_node(
                    &invitation.project_id,
                    &invitation.node_id,
                    existing,
                    instance_id,
                )
                .await
                .and_then(|node| {
                    Ok((node, invitation.certificate_pem.ok_or(CloudError::Storage)?))
                });
        }
        if invitation.expires_at <= now {
            return Err(CloudError::PermissionDenied);
        }
        let node = CloudNode {
            owner_user_id: invitation.owner_user_id.clone(),
            node_id: invitation.node_id.clone(),
            project_id: invitation.project_id.clone(),
            instance_id: instance_id.into(),
            label: invitation.label.clone(),
            certificate_sha256: certificate.fingerprint.clone(),
            roles: invitation.roles.clone(),
            revoked: false,
            host_id: None,
            workspace_id: None,
            workspace_label: None,
            runtime_ready: false,
            policy: None,
            policy_observed_at: None,
            revision: 1,
        };
        invitation.redeemed_certificate = Some(certificate.fingerprint);
        invitation.redeemed_csr = Some(certificate.csr_sha256);
        invitation.certificate_pem = Some(certificate.certificate_pem.clone());
        self.commit(vec![
            self.event(
                "enrollment",
                invitation_stream,
                revision,
                "cloud.invitation.redeemed.v1",
                &invitation,
            )?,
            self.event(
                "enrollment",
                format!("cloud.node:{}:{}", node.project_id, node.node_id),
                0,
                "cloud.node.enrolled.v1",
                &node,
            )?,
        ])
        .await?;
        Ok((node, certificate.certificate_pem))
    }
}
