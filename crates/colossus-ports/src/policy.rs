use super::*;

/// Policy-decision failures always fail closed at the gateway.
#[derive(Debug, Error)]
pub enum PolicyError {
    /// Input exceeded the configured disclosure limit.
    #[error("policy input exceeds {limit} bytes")]
    InputTooLarge {
        /// Configured byte limit.
        limit: usize,
    },
    /// Transport, readiness, or timeout failure.
    #[error("policy unavailable: {0}")]
    Unavailable(String),
    /// Response failed the strict contract.
    #[error("invalid policy response: {0}")]
    InvalidDecision(String),
}

/// Model-assisted risk evaluation failures require an explicit approval fallback.
#[derive(Debug, Error)]
pub enum RiskEvaluationError {
    /// The configured evaluator or its policy-bound provider was unavailable.
    #[error("risk evaluator unavailable: {0}")]
    Unavailable(String),
    /// The evaluator returned output outside the strict response contract.
    #[error("invalid risk assessment: {0}")]
    InvalidAssessment(String),
}

/// Advisory risk evaluator invoked only after deterministic policy requires approval.
#[async_trait]
pub trait RiskEvaluator: Send + Sync {
    /// Assess one prepared request using tools-disabled, policy-bound model access.
    async fn evaluate(
        &self,
        request: &EffectRequest,
        decision: &PolicyDecision,
    ) -> Result<colossus_contracts::RiskAssessment, RiskEvaluationError>;
}

/// Built-in or OPA policy decision point.
#[async_trait]
pub trait PolicyDecisionPoint: Send + Sync {
    /// Evaluate a fully redacted logical request.
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError>;

    /// Report current readiness and bounded revision metadata.
    async fn doctor(&self) -> Result<Value, PolicyError>;
}

/// Interactive or application-supplied approval handler.
#[async_trait]
pub trait ApprovalProvider: Send + Sync {
    /// Whether eligible approval-required effects should receive risk review.
    fn risk_auto_enabled(&self) -> bool {
        false
    }

    /// Best-effort release of a durable automatic low-risk approval notice.
    async fn automatic_approval_granted(&self, _notice: AutomaticApprovalNotice) {}

    /// Best-effort warning that risk-auto fell back to explicit approval.
    async fn risk_review_fallback(&self, _notice: RiskReviewFallbackNotice) {}

    /// Request a proof bound to the canonical request hash and initial decision.
    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError>;
}
