use super::*;

/// Approval provider that always denies; safe for non-interactive runs.
pub struct DenyApproval;

#[async_trait]
impl ApprovalProvider for DenyApproval {
    async fn request_approval(
        &self,
        _request: &EffectRequest,
        _request_hash: &str,
        _decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        Ok(None)
    }
}

/// Approval provider used by trusted application APIs after explicit operator action.
pub struct AllowApproval {
    /// Stable approving operator identifier.
    pub approved_by: String,
}

#[async_trait]
impl ApprovalProvider for AllowApproval {
    async fn request_approval(
        &self,
        _request: &EffectRequest,
        request_hash: &str,
        _decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        Ok(Some(approval_proof(
            request_hash,
            self.approved_by.clone(),
        )?))
    }
}
