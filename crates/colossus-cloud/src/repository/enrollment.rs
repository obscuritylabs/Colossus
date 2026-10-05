use super::*;
use crate::{
    CertificateRedemption, Enrollment, validate_identifier, validation::bounded_fingerprint,
};

impl CloudRepository {
    /// Allocate an expiring invitation beneath explicit administration authority.
    pub fn invite(
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
    }

    /// Atomically consume an invitation and enroll its fixed placement. A repeated
    /// redeem with the same certificate/instance reconciles a lost HTTP response.
    pub fn redeem(
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
                .and_then(|node| {
                    self.journal.checkpoint()?;
                    Ok((node, invitation.certificate_pem.ok_or(CloudError::Storage)?))
                });
        }
        if invitation.expires_at <= now {
            return Err(CloudError::PermissionDenied);
        }
        let node = CloudNode {
            node_id: invitation.node_id.clone(),
            project_id: invitation.project_id.clone(),
            instance_id: instance_id.into(),
            label: invitation.label.clone(),
            certificate_sha256: certificate.fingerprint.clone(),
            roles: invitation.roles.clone(),
            revoked: false,
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
        ])?;
        Ok((node, certificate.certificate_pem))
    }
}
