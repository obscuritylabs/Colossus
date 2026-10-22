//! Native-only certificate review uses independent admission before a viewer exists.
use super::{NativeBrowserClient, NativeClientIdentityReview};
use crate::{SdkError, SdkResult};
use colossus_browser_presentation::{
    ClientIdentityPolicy,
    native_admission::{NativeBrowserReply, NativeBrowserRequest},
};

impl NativeBrowserClient {
    /// Inspect pending handshakes under this attested native application's authority.
    /// `AutomaticIfUnique` explicitly authorizes one eligible identity after Runtime
    /// validates the current owner. An uncertain selection is never replayed.
    /// This can complete while `open_human` or an agent's initial browser open is waiting.
    pub async fn pending_client_identity(
        &self,
        policy: ClientIdentityPolicy,
    ) -> SdkResult<Option<NativeClientIdentityReview>> {
        match self
            .connect()
            .await?
            .request(&NativeBrowserRequest::PendingClientIdentity { policy })
            .await
            .map_err(|_| SdkError::SidecarFailed)?
        {
            NativeBrowserReply::ClientIdentity { review } => {
                if let Some(review) = &review {
                    review.validate().map_err(|_| SdkError::IdentityMismatch)?;
                }
                Ok(review)
            }
            NativeBrowserReply::Unavailable | NativeBrowserReply::Denied => Ok(None),
            _ => Err(SdkError::SidecarFailed),
        }
    }

    /// Submit a choice from the trusted native review surface for one retained handshake.
    /// This method never retries a mutation or reports that server authentication succeeded.
    pub async fn select_client_identity(
        &self,
        review: &NativeClientIdentityReview,
        fingerprint_sha256: Option<String>,
    ) -> SdkResult<()> {
        review.validate().map_err(|_| SdkError::IdentityMismatch)?;
        if fingerprint_sha256.as_ref().is_some_and(|fingerprint| {
            !review
                .challenge
                .candidates
                .iter()
                .any(|candidate| &candidate.fingerprint_sha256 == fingerprint)
        }) {
            return Err(SdkError::IdentityMismatch);
        }
        let request = NativeBrowserRequest::SelectClientIdentity {
            session: review.session.clone(),
            key: review.challenge.key,
            fingerprint_sha256,
        };
        request.validate().map_err(|_| SdkError::IdentityMismatch)?;
        match self
            .connect()
            .await?
            .request(&request)
            .await
            .map_err(|_| SdkError::SidecarFailed)?
        {
            NativeBrowserReply::ClientIdentitySelected => Ok(()),
            _ => Err(SdkError::SidecarFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_browser_presentation::native_admission::NativeBrowserEnrollment;
    use std::sync::{Arc, atomic::AtomicU32};
    use zeroize::Zeroizing;

    fn client() -> NativeBrowserClient {
        NativeBrowserClient {
            endpoint: String::new(),
            authentication: Arc::new(Zeroizing::new([7; 32])),
            enrollment: NativeBrowserEnrollment {
                generation: [1; 16],
                instance: [2; 16],
                application_id: "desktop".into(),
                workspace_version: 3,
                workspace_digest: [3; 32],
                parent_process_id: std::process::id(),
            },
            child: Arc::new(AtomicU32::new(0)),
        }
    }

    #[test]
    fn retained_review_snapshot_cannot_follow_a_restarted_or_reenrolled_sidecar() {
        use std::sync::atomic::Ordering;
        let client = client();
        assert!(client.incarnation().is_err());
        client.child.store(42, Ordering::Release);
        let snapshot = client.incarnation().unwrap();
        let mut retained = client.clone();
        assert!(retained.matches_incarnation(&snapshot));
        client.child.store(0, Ordering::Release);
        assert!(!retained.matches_incarnation(&snapshot));
        client.child.store(43, Ordering::Release);
        assert!(!retained.matches_incarnation(&snapshot));
        client.child.store(42, Ordering::Release);
        retained.enrollment.generation = [4; 16];
        assert!(!retained.matches_incarnation(&snapshot));
        retained.enrollment = client.enrollment.clone();
        retained.enrollment.parent_process_id = 0;
        assert!(!retained.matches_incarnation(&snapshot));
    }
}
