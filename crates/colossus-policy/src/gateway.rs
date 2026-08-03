use super::*;

/// Single policy-enforcement point for all external or sensitive effects.
pub struct EffectGateway {
    journal: Arc<dyn EventJournal>,
    policy: Arc<dyn PolicyDecisionPoint>,
    approvals: Arc<dyn ApprovalProvider>,
    risk_evaluator: RwLock<Option<Weak<dyn RiskEvaluator>>>,
    kernel: SafetyKernel,
    permit_key: [u8; 32],
}

struct StreamBridge<'a> {
    gateway: &'a EffectGateway,
    executor: &'a dyn StreamingEffectExecutor,
    observer: tokio::sync::Mutex<&'a mut dyn ReleasedEffectObserver>,
}

struct GatewayStreamSink<'a> {
    gateway: &'a EffectGateway,
    request: &'a EffectRequest,
    obligations: PolicyObligations,
    observer: &'a mut dyn ReleasedEffectObserver,
    sequence: u64,
    total_bytes: usize,
    last: Option<QuarantinedEffectResult>,
    failure: Option<StreamSinkFailure>,
}

enum StreamSinkFailure {
    Failed(String),
    Unknown(String),
    Denied(String),
}

fn risk_auto_ineligibility(request: &EffectRequest) -> Option<&'static str> {
    if !matches!(
        request.actor.actor_type,
        ActorType::Model | ActorType::Subagent
    ) {
        return Some("Risk-auto review is limited to model and child-agent effects.");
    }
    if request.context.workflow_id.is_some() || request.context.workflow_hash.is_some() {
        return Some("Risk-auto review is disabled for effects with workflow lineage.");
    }
    match request.action.as_str() {
        "shell.run" | "web.search" => None,
        "network.http"
            if request
                .content
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|method| method.eq_ignore_ascii_case("GET"))
                && request.content.get("body_base64").is_none() =>
        {
            None
        }
        "network.http" => {
            Some("Risk-auto review requires network.http to be a bodyless GET request.")
        }
        "mcp.call" if supported_mcp_review_metadata(request) => None,
        "mcp.call" => Some(
            "Risk-auto review requires a configured top-level MCP call with supported, request-bound discovery metadata.",
        ),
        _ => Some("This effect action is not eligible for risk-auto review."),
    }
}

fn supported_mcp_review_metadata(request: &EffectRequest) -> bool {
    let Some(content) = request.content.as_object() else {
        return false;
    };
    let Some(operation) = content.get("operation").and_then(Value::as_object) else {
        return false;
    };
    let supported_transport = match content.get("transport").and_then(Value::as_str) {
        Some("stdio") => content.get("url").is_none_or(Value::is_null),
        Some("streamable_http") => content
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| url == request.resource),
        _ => false,
    };
    let supported_identity = operation.get("kind").and_then(Value::as_str) == Some("call_tool")
        && operation
            .get("server")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value.len() <= 256)
        && operation
            .get("tool")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value.len() <= 256);
    let supported_description = operation.get("description").is_none_or(|value| {
        value.is_null() || value.as_str().is_some_and(|text| text.len() <= 32 * 1024)
    });
    let supported_annotations = operation
        .get("annotations")
        .is_none_or(supported_mcp_annotations);
    let Some(input_schema) = operation
        .get("input_schema")
        .filter(|value| value.is_object())
    else {
        return false;
    };
    let schema_hash_matches = canonical_bytes(input_schema).is_ok_and(|bytes| {
        let expected = sha256_hex(&bytes);
        bytes.len() <= 256 * 1024
            && operation.get("schema_sha256").and_then(Value::as_str) == Some(expected.as_str())
    });
    supported_transport
        && supported_identity
        && supported_description
        && supported_annotations
        && operation.get("arguments").is_some_and(Value::is_object)
        && schema_hash_matches
}

fn supported_mcp_annotations(value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(annotations) = value.as_object() else {
        return false;
    };
    annotations.iter().all(|(key, value)| match key.as_str() {
        "title" => value.is_null() || value.as_str().is_some_and(|text| text.len() <= 8 * 1024),
        "readOnlyHint" | "destructiveHint" | "idempotentHint" | "openWorldHint" => {
            value.is_null() || value.is_boolean()
        }
        _ => false,
    })
}

impl StreamSinkFailure {
    fn execution_error(&self) -> ExecutionError {
        match self {
            Self::Failed(message) => ExecutionError::Failed(message.clone()),
            Self::Unknown(message) => ExecutionError::OutcomeUnknown(message.clone()),
            Self::Denied(message) => ExecutionError::ReleaseDenied(message.clone()),
        }
    }
}

#[async_trait]
impl QuarantinedEffectObserver for GatewayStreamSink<'_> {
    async fn observe(&mut self, result: QuarantinedEffectResult) -> Result<(), ExecutionError> {
        if let Some(failure) = &self.failure {
            return Err(failure.execution_error());
        }
        if !result.effect_succeeded {
            let failure =
                StreamSinkFailure::Failed("streaming adapter reported chunk failure".into());
            let error = failure.execution_error();
            self.failure = Some(failure);
            return Err(error);
        }
        let limit = match usize::try_from(self.obligations.max_output_bytes) {
            Ok(limit) => limit,
            Err(error) => {
                let failure = StreamSinkFailure::Failed(error.to_string());
                let error = failure.execution_error();
                self.failure = Some(failure);
                return Err(error);
            }
        };
        self.total_bytes = self.total_bytes.saturating_add(result.bytes.len());
        if self.total_bytes > limit {
            let failure = StreamSinkFailure::Unknown(
                "streamed provider output exceeds the cumulative permitted bound".into(),
            );
            let error = failure.execution_error();
            self.failure = Some(failure);
            return Err(error);
        }
        self.sequence = self.sequence.saturating_add(1);
        let released = match self
            .gateway
            .release_stream_chunk(self.request, &self.obligations, self.sequence, &result)
            .await
        {
            Ok(released) => released,
            Err(GatewayError::Denied(message)) => {
                let failure = StreamSinkFailure::Denied(message);
                let error = failure.execution_error();
                self.failure = Some(failure);
                return Err(error);
            }
            Err(error) => {
                let failure = StreamSinkFailure::Unknown(format!(
                    "stream release failed after execution began: {error}"
                ));
                let error = failure.execution_error();
                self.failure = Some(failure);
                return Err(error);
            }
        };
        if let Err(error) = self.observer.observe(released).await {
            let failure = match error {
                ExecutionError::ReleaseDenied(message) => StreamSinkFailure::Denied(message),
                ExecutionError::Failed(message)
                | ExecutionError::OutcomeUnknown(message)
                | ExecutionError::Recoverable { message, .. }
                | ExecutionError::HttpStatus { message, .. } => StreamSinkFailure::Unknown(
                    format!("released stream observation failed: {message}"),
                ),
            };
            let error = failure.execution_error();
            self.failure = Some(failure);
            return Err(error);
        }
        self.last = Some(result);
        Ok(())
    }
}

#[async_trait]
impl EffectExecutor for StreamBridge<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let obligations = permit.obligations().clone();
        let mut observer = self.observer.lock().await;
        let mut sink = GatewayStreamSink {
            gateway: self.gateway,
            request,
            obligations,
            observer: &mut **observer,
            sequence: 0,
            total_bytes: 0,
            last: None,
            failure: None,
        };
        let terminal = self
            .executor
            .execute_stream(request, permit, &mut sink)
            .await?;
        if let Some(failure) = &sink.failure {
            return Err(failure.execution_error());
        }
        if sink.sequence == 0 || sink.last.as_ref() != Some(&terminal) {
            return Err(ExecutionError::Failed(
                "streaming adapter terminal result did not match its last released chunk".into(),
            ));
        }
        Ok(terminal)
    }
}

impl EffectGateway {
    /// Compose trusted journal, policy, approval, and permit services.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        policy: Arc<dyn PolicyDecisionPoint>,
        approvals: Arc<dyn ApprovalProvider>,
        kernel: SafetyKernel,
        permit_key: [u8; 32],
    ) -> Self {
        Self {
            journal,
            policy,
            approvals,
            risk_evaluator: RwLock::new(None),
            kernel,
            permit_key,
        }
    }

    /// Bind the policy-gated model evaluator after provider composition is complete.
    pub fn bind_risk_evaluator(
        &self,
        evaluator: Weak<dyn RiskEvaluator>,
    ) -> Result<(), GatewayError> {
        *self
            .risk_evaluator
            .write()
            .map_err(|_| GatewayError::Contract("risk evaluator lock is poisoned".into()))? =
            Some(evaluator);
        Ok(())
    }

    async fn review_risk(
        &self,
        request: &mut EffectRequest,
        decision: &PolicyDecision,
    ) -> Result<bool, GatewayError> {
        if !self.approvals.risk_auto_enabled() {
            return Ok(false);
        }
        if let Some(reason) = risk_auto_ineligibility(request) {
            request.risk.status = RiskStatus::Unavailable;
            request.risk.level = None;
            request.risk.reason = Some(reason.into());
            self.event(
                request,
                "risk.review.ineligible.v1",
                EventClassification::Policy,
                json!({
                    "decision_id": decision.decision_id,
                    "reason": reason,
                }),
            )?;
            return Ok(false);
        }
        self.event(
            request,
            "risk.review.requested.v1",
            EventClassification::Policy,
            json!({
                "decision_id": decision.decision_id,
                "policy_revision": decision.policy_revision,
            }),
        )?;
        let evaluator = self
            .risk_evaluator
            .read()
            .map_err(|_| GatewayError::Contract("risk evaluator lock is poisoned".into()))?
            .as_ref()
            .and_then(Weak::upgrade);
        let Some(evaluator) = evaluator else {
            let reason =
                "The configured risk evaluator was unavailable, so manual approval is required.";
            request.risk.status = RiskStatus::Unavailable;
            request.risk.level = None;
            request.risk.reason = Some(reason.into());
            self.event(
                request,
                "risk.review.unavailable.v1",
                EventClassification::Policy,
                json!({"failure": RiskReviewFailure::EvaluatorUnavailable}),
            )?;
            self.approvals
                .risk_review_fallback(RiskReviewFallbackNotice {
                    action: request.action.clone(),
                    resource: request.resource.clone(),
                    failure: RiskReviewFailure::EvaluatorUnavailable,
                    reason: reason.into(),
                })
                .await;
            return Ok(false);
        };
        match evaluator.evaluate(request, decision).await {
            Ok(assessment) => {
                let reason = assessment.reason.trim();
                if reason.is_empty() || reason.chars().count() > 1_000 {
                    let reason = "The risk evaluator response failed strict validation, so manual approval is required.";
                    request.risk.status = RiskStatus::Unavailable;
                    request.risk.level = None;
                    request.risk.reason = Some(reason.into());
                    self.event(
                        request,
                        "risk.review.unavailable.v1",
                        EventClassification::Policy,
                        json!({"failure": RiskReviewFailure::InvalidAssessment}),
                    )?;
                    self.approvals
                        .risk_review_fallback(RiskReviewFallbackNotice {
                            action: request.action.clone(),
                            resource: request.resource.clone(),
                            failure: RiskReviewFailure::InvalidAssessment,
                            reason: reason.into(),
                        })
                        .await;
                    return Ok(false);
                }
                request.risk.status = RiskStatus::Available;
                request.risk.level = Some(
                    match assessment.risk_level {
                        RiskLevel::Low => "low",
                        RiskLevel::Medium => "medium",
                        RiskLevel::High => "high",
                    }
                    .into(),
                );
                request.risk.reason = Some(reason.into());
                self.event(
                    request,
                    "risk.review.completed.v1",
                    EventClassification::Policy,
                    json!({
                        "decision_id": decision.decision_id,
                        "risk_level": assessment.risk_level,
                        "recommended_decision": assessment.recommended_decision,
                        "reason": reason,
                    }),
                )?;
                Ok(assessment.risk_level == RiskLevel::Low
                    && assessment.recommended_decision == RiskRecommendation::Allow)
            }
            Err(error) => {
                let (failure, reason) = match error {
                    RiskEvaluationError::Unavailable(_) => (
                        RiskReviewFailure::EvaluatorUnavailable,
                        "The configured risk evaluator was unavailable, so manual approval is required.",
                    ),
                    RiskEvaluationError::InvalidAssessment(_) => (
                        RiskReviewFailure::InvalidAssessment,
                        "The risk evaluator response failed strict validation, so manual approval is required.",
                    ),
                };
                request.risk.status = RiskStatus::Unavailable;
                request.risk.level = None;
                request.risk.reason = Some(reason.into());
                self.event(
                    request,
                    "risk.review.unavailable.v1",
                    EventClassification::Policy,
                    json!({"failure": failure}),
                )?;
                self.approvals
                    .risk_review_fallback(RiskReviewFallbackNotice {
                        action: request.action.clone(),
                        resource: request.resource.clone(),
                        failure,
                        reason: reason.into(),
                    })
                    .await;
                Ok(false)
            }
        }
    }

    fn event(
        &self,
        request: &EffectRequest,
        event_type: &str,
        classification: EventClassification,
        payload: Value,
    ) -> Result<(), GatewayError> {
        let stream_id = format!("effect:{}", request.request_id);
        let version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| GatewayError::Contract(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: version,
            classification,
            event_type: event_type.into(),
            actor: request.actor.clone(),
            context: request.context.clone(),
            payload,
        })?;
        Ok(())
    }

    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, GatewayError> {
        let decision = match self.policy.decide(request).await {
            Ok(decision) => decision,
            Err(error) => {
                self.event(
                    request,
                    "policy.error.v1",
                    EventClassification::Policy,
                    json!({"error_kind": "unavailable_or_invalid", "message": error.to_string()}),
                )?;
                self.event(
                    request,
                    "effect.denied.v1",
                    EventClassification::Effect,
                    json!({"reason": "policy failure; fail closed"}),
                )?;
                return Err(error.into());
            }
        };
        if let Err(error) = self.kernel.validate_decision(request, &decision) {
            self.event(
                request,
                "policy.error.v1",
                EventClassification::Policy,
                json!({"error_kind": "invalid_decision", "message": error.to_string()}),
            )?;
            self.event(
                request,
                "effect.denied.v1",
                EventClassification::Effect,
                json!({"reason": "invalid policy decision; fail closed"}),
            )?;
            return Err(error);
        }
        self.event(
            request,
            "policy.decided.v1",
            EventClassification::Policy,
            json!({
                "decision_id": decision.decision_id,
                "policy_revision": decision.policy_revision,
                "outcome": decision.outcome,
                "reason": decision.reason,
                "audit_labels": decision.obligations.audit_labels,
            }),
        )?;
        Ok(decision)
    }

    pub(super) fn mint_permit(
        &self,
        request: &EffectRequest,
        request_hash: String,
        decision: &PolicyDecision,
    ) -> Result<ExecutionPermit, GatewayError> {
        let obligations_hash = sha256_hex(&canonical_bytes(&decision.obligations)?);
        let nonce = Uuid::now_v7().to_string();
        let expires_at_unix_ms = now_unix_ms() + PERMIT_LIFETIME_MS;
        let claims = PermitClaims {
            request_hash: &request_hash,
            decision_id: &decision.decision_id,
            obligations_hash: &obligations_hash,
            actor_id: &request.actor.id,
            nonce: &nonce,
            expires_at_unix_ms,
        };
        let mut mac = HmacSha256::new_from_slice(&self.permit_key)
            .map_err(|error| GatewayError::Contract(error.to_string()))?;
        mac.update(&canonical_bytes(&claims)?);
        Ok(ExecutionPermit {
            request_hash,
            decision_id: decision.decision_id.clone(),
            obligations_hash,
            actor_id: request.actor.id.clone(),
            nonce,
            expires_at_unix_ms,
            authentication_tag: mac.finalize().into_bytes().to_vec(),
            obligations: decision.obligations.clone(),
            consumed: AtomicBool::new(false),
        })
    }

    pub(super) fn authenticate_and_consume(
        &self,
        permit: &ExecutionPermit,
        request: &EffectRequest,
        decision: &PolicyDecision,
    ) -> Result<(), GatewayError> {
        let request_hash = sha256_hex(&canonical_bytes(request)?);
        let obligations_hash = sha256_hex(&canonical_bytes(&decision.obligations)?);
        if permit.request_hash != request_hash
            || permit.decision_id != decision.decision_id
            || permit.obligations_hash != obligations_hash
            || permit.actor_id != request.actor.id
            || permit.expires_at_unix_ms < now_unix_ms()
        {
            return Err(GatewayError::Safety(
                "permit does not match request, decision, actor, obligations, or expiry".into(),
            ));
        }
        let claims = PermitClaims {
            request_hash: &permit.request_hash,
            decision_id: &permit.decision_id,
            obligations_hash: &permit.obligations_hash,
            actor_id: &permit.actor_id,
            nonce: &permit.nonce,
            expires_at_unix_ms: permit.expires_at_unix_ms,
        };
        let mut mac = HmacSha256::new_from_slice(&self.permit_key)
            .map_err(|error| GatewayError::Contract(error.to_string()))?;
        mac.update(&canonical_bytes(&claims)?);
        mac.verify_slice(&permit.authentication_tag)
            .map_err(|_| GatewayError::Safety("permit authentication failed".into()))?;
        permit
            .consumed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| GatewayError::Safety("permit has already been consumed".into()))?;
        Ok(())
    }

    /// Authorize, execute into quarantine, optionally authorize output, and release.
    pub async fn execute(
        &self,
        request: EffectRequest,
        executor: &dyn EffectExecutor,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        self.execute_internal(request, executor, false).await
    }

    /// Authorize one streaming effect and release only gateway-approved normalized chunks.
    pub async fn execute_stream(
        &self,
        request: EffectRequest,
        executor: &dyn StreamingEffectExecutor,
        observer: &mut dyn ReleasedEffectObserver,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        let bridge = StreamBridge {
            gateway: self,
            executor,
            observer: tokio::sync::Mutex::new(observer),
        };
        self.execute_internal(request, &bridge, true).await
    }

    async fn execute_internal(
        &self,
        request: EffectRequest,
        executor: &dyn EffectExecutor,
        chunks_already_released: bool,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        if self.journal.is_recovery_mode() {
            return Err(GatewayError::Journal(StoreError::RecoveryMode));
        }
        if request.schema_version != 1
            || request.request_id.is_empty()
            || request.phase != EffectPhase::PreEffect
        {
            return Err(GatewayError::Safety(
                "unsupported schema version, empty request id, or caller-supplied post-effect phase"
                    .into(),
            ));
        }
        self.event(
            &request,
            "effect.requested.v1",
            EventClassification::Effect,
            disclosure_summary(&request),
        )?;
        let mut request = match self.kernel.prepare(&request) {
            Ok(request) => request,
            Err(error) => {
                self.event(
                    &request,
                    "effect.denied.v1",
                    EventClassification::Effect,
                    json!({"reason": error.to_string(), "source": "safety_kernel"}),
                )?;
                return Err(error);
            }
        };
        let mut decision = self.decide(&request).await?;
        if decision.outcome == DecisionOutcome::RequireApproval {
            let risk_auto_approved = self.review_risk(&mut request, &decision).await?;
            let request_hash = sha256_hex(&canonical_bytes(&request)?);
            let approval = if risk_auto_approved {
                Ok(Some(approval_proof(
                    &request_hash,
                    "risk-evaluator:auto-low-risk",
                )?))
            } else {
                self.approvals
                    .request_approval(&request, &request_hash, &decision)
                    .await
            };
            let proof = match approval {
                Ok(Some(proof)) => proof,
                Ok(None) => {
                    self.event(
                        &request,
                        "approval.denied.v1",
                        EventClassification::Approval,
                        json!({"decision_id": decision.decision_id, "reason": "operator declined"}),
                    )?;
                    self.event(
                        &request,
                        "effect.denied.v1",
                        EventClassification::Effect,
                        json!({"decision_id": decision.decision_id, "reason": "operator declined"}),
                    )?;
                    return Err(GatewayError::Approval("operator declined".into()));
                }
                Err(error) => {
                    self.event(
                        &request,
                        "approval.error.v1",
                        EventClassification::Approval,
                        json!({"decision_id": decision.decision_id, "message": error.to_string()}),
                    )?;
                    self.event(
                        &request,
                        "effect.denied.v1",
                        EventClassification::Effect,
                        json!({"decision_id": decision.decision_id, "reason": "approval provider failed"}),
                    )?;
                    return Err(GatewayError::Policy(error));
                }
            };
            if proof.request_hash != request_hash {
                return Err(GatewayError::Approval(
                    "approval proof is bound to a different request".into(),
                ));
            }
            self.event(
                &request,
                "approval.granted.v1",
                EventClassification::Approval,
                json!({
                    "approval_id": proof.approval_id,
                    "approved_by": proof.approved_by,
                    "request_hash": proof.request_hash,
                }),
            )?;
            if risk_auto_approved {
                self.approvals
                    .automatic_approval_granted(AutomaticApprovalNotice {
                        action: request.action.clone(),
                        resource: request.resource.clone(),
                        risk_level: RiskLevel::Low,
                        reason: request.risk.reason.clone().unwrap_or_else(|| {
                            "automatic low-risk review approved the effect".into()
                        }),
                    })
                    .await;
            }
            request.approval = Some(proof);
            decision = self.decide(&request).await?;
        }
        if decision.outcome != DecisionOutcome::Allow {
            self.event(
                &request,
                "effect.denied.v1",
                EventClassification::Effect,
                json!({"decision_id": decision.decision_id, "reason": decision.reason}),
            )?;
            return Err(GatewayError::Denied(decision.reason));
        }
        let request_hash = sha256_hex(&canonical_bytes(&request)?);
        let permit = self.mint_permit(&request, request_hash, &decision)?;
        self.authenticate_and_consume(&permit, &request, &decision)?;
        self.event(
            &request,
            "effect.started.v1",
            EventClassification::Effect,
            json!({
                "decision_id": decision.decision_id,
                "permit_nonce": permit.nonce,
                "permit_expires_at_unix_ms": permit.expires_at_unix_ms,
            }),
        )?;

        let result = match tokio::time::timeout(
            Duration::from_millis(decision.obligations.timeout_ms),
            executor.execute(&request, permit),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(ExecutionError::Failed(message))) => {
                self.event(
                    &request,
                    "effect.failed.v1",
                    EventClassification::Effect,
                    json!({"message": message}),
                )?;
                return Err(GatewayError::Execution(message));
            }
            Ok(Err(ExecutionError::Recoverable {
                code,
                message,
                http_status,
                retry_after_ms,
            })) => {
                self.event(
                    &request,
                    "effect.failed.v1",
                    EventClassification::Effect,
                    json!({
                        "code": code,
                        "message": message,
                        "recoverable": true,
                        "http_status": http_status,
                        "retry_after_ms": retry_after_ms,
                    }),
                )?;
                return Err(GatewayError::RecoverableExecution {
                    code,
                    message,
                    http_status,
                    retry_after_ms,
                });
            }
            Ok(Err(ExecutionError::HttpStatus { status, message })) => {
                self.event(
                    &request,
                    "effect.failed.v1",
                    EventClassification::Effect,
                    json!({
                        "message": message,
                        "recoverable": false,
                        "http_status": status,
                    }),
                )?;
                return Err(GatewayError::HttpStatus { status, message });
            }
            Ok(Err(ExecutionError::OutcomeUnknown(message))) => {
                self.event(
                    &request,
                    "effect.outcome_unknown.v1",
                    EventClassification::Effect,
                    json!({"message": message}),
                )?;
                return Err(GatewayError::OutcomeUnknown(message));
            }
            Ok(Err(ExecutionError::ReleaseDenied(message))) => {
                return Err(GatewayError::Denied(message));
            }
            Err(_) => {
                let message = "adapter timed out after execution began".to_owned();
                self.event(
                    &request,
                    "effect.outcome_unknown.v1",
                    EventClassification::Effect,
                    json!({"message": message}),
                )?;
                return Err(GatewayError::OutcomeUnknown(message));
            }
        };
        if result.bytes.len()
            > usize::try_from(decision.obligations.max_output_bytes).unwrap_or(usize::MAX)
        {
            self.event(
                &request,
                "effect.failed.v1",
                EventClassification::Effect,
                json!({"message": "quarantined output exceeded policy limit"}),
            )?;
            return Err(GatewayError::Execution(
                "quarantined output exceeded policy limit".into(),
            ));
        }
        if !result.effect_succeeded {
            self.event(
                &request,
                "effect.failed.v1",
                EventClassification::Effect,
                json!({"message": "adapter reported effect failure"}),
            )?;
            return Err(GatewayError::Execution(
                "adapter reported effect failure".into(),
            ));
        }

        if decision.obligations.require_post_effect && !chunks_already_released {
            let mut post_request = request.clone();
            post_request.request_id = format!("{}:post", request.request_id);
            post_request.phase = EffectPhase::PostEffect;
            post_request.approval = None;
            post_request.content = json!({
                "media_type": result.media_type,
                "size": result.bytes.len(),
                "content_base64": BASE64.encode(&result.bytes),
            });
            let post_request = self.kernel.prepare(&post_request)?;
            self.event(
                &post_request,
                "effect.release_requested.v1",
                EventClassification::Effect,
                disclosure_summary(&post_request),
            )?;
            let post_decision = self.decide(&post_request).await?;
            if post_decision.outcome != DecisionOutcome::Allow {
                self.event(
                    &post_request,
                    "effect.release_denied.v1",
                    EventClassification::Effect,
                    json!({
                        "decision_id": post_decision.decision_id,
                        "reason": post_decision.reason,
                        "content_hash": sha256_hex(&result.bytes),
                        "size": result.bytes.len(),
                    }),
                )?;
                return Err(GatewayError::Denied(format!(
                    "post-effect release denied: {}",
                    post_decision.reason
                )));
            }
        }

        self.event(
            &request,
            "effect.completed.v1",
            EventClassification::Effect,
            json!({
                "decision_id": decision.decision_id,
                "content_hash": sha256_hex(&result.bytes),
                "size": result.bytes.len(),
            }),
        )?;
        Ok(ReleasedEffectResult {
            media_type: result.media_type,
            bytes: result.bytes,
        })
    }

    async fn release_stream_chunk(
        &self,
        request: &EffectRequest,
        obligations: &PolicyObligations,
        sequence: u64,
        result: &QuarantinedEffectResult,
    ) -> Result<ReleasedEffectResult, GatewayError> {
        if obligations.require_post_effect {
            let mut post_request = request.clone();
            post_request.request_id = format!("{}:post:chunk:{sequence}", request.request_id);
            post_request.phase = EffectPhase::PostEffect;
            post_request.approval = None;
            post_request.content = json!({
                "media_type": result.media_type,
                "size": result.bytes.len(),
                "sequence": sequence,
                "content_base64": BASE64.encode(&result.bytes),
            });
            let post_request = self.kernel.prepare(&post_request)?;
            self.event(
                &post_request,
                "effect.release_requested.v1",
                EventClassification::Effect,
                disclosure_summary(&post_request),
            )?;
            let post_decision = self.decide(&post_request).await?;
            if post_decision.outcome != DecisionOutcome::Allow {
                self.event(
                    &post_request,
                    "effect.release_denied.v1",
                    EventClassification::Effect,
                    json!({
                        "decision_id": post_decision.decision_id,
                        "reason": post_decision.reason,
                        "content_hash": sha256_hex(&result.bytes),
                        "size": result.bytes.len(),
                        "sequence": sequence,
                    }),
                )?;
                return Err(GatewayError::Denied(format!(
                    "stream chunk post-effect release denied: {}",
                    post_decision.reason
                )));
            }
        }
        self.event(
            request,
            "effect.chunk_released.v1",
            EventClassification::Effect,
            json!({
                "content_hash": sha256_hex(&result.bytes),
                "size": result.bytes.len(),
                "sequence": sequence,
                "media_type": result.media_type,
            }),
        )?;
        Ok(ReleasedEffectResult {
            media_type: result.media_type.clone(),
            bytes: result.bytes.clone(),
        })
    }
}
