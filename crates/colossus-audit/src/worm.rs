use super::*;

/// HTTPS create-only exporter for a remote retention-locked or WORM object endpoint.
pub struct GatewayWormAuditExporter {
    endpoint: Url,
    credential_reference: Option<String>,
    gateway: Arc<EffectGateway>,
    executor: Arc<dyn EffectExecutor>,
}

impl GatewayWormAuditExporter {
    /// Bind a trailing-slash HTTPS collection endpoint to the permit-bearing HTTP executor.
    pub fn new(
        endpoint: &str,
        credential_reference: Option<String>,
        gateway: Arc<EffectGateway>,
        executor: Arc<dyn EffectExecutor>,
    ) -> Result<Self, StoreError> {
        let endpoint = Url::parse(endpoint)
            .map_err(|_| StoreError::Adapter("WORM audit endpoint is invalid".into()))?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.path().ends_with('/')
            || endpoint.cannot_be_a_base()
        {
            return Err(StoreError::Adapter(
                "WORM audit endpoint must be a credential-free trailing-slash HTTPS URL".into(),
            ));
        }
        if credential_reference
            .as_deref()
            .is_some_and(|reference| !valid_environment_reference(reference))
        {
            return Err(StoreError::Adapter(
                "WORM audit credential must be an env:VARIABLE reference".into(),
            ));
        }
        Ok(Self {
            endpoint,
            credential_reference,
            gateway,
            executor,
        })
    }

    fn target(&self, evidence: &AuditEvidence, content_hash: &str) -> Result<Url, StoreError> {
        let mut target = self.endpoint.clone();
        target
            .path_segments_mut()
            .map_err(|_| StoreError::Adapter("WORM audit object URL is invalid".into()))?
            .push(&format!(
                "{:020}-{}-{content_hash}.json",
                evidence.global_sequence, evidence.event_id
            ));
        Ok(target)
    }
}

#[async_trait]
impl AuditExporter for GatewayWormAuditExporter {
    fn kind(&self) -> &'static str {
        "https-create-only-worm-json"
    }

    async fn export(&self, evidence: &AuditEvidence) -> Result<(), StoreError> {
        let mut encoded = serde_json::to_vec(evidence).map_err(adapter)?;
        encoded.push(b'\n');
        if encoded.len() > MAX_EVIDENCE_BYTES {
            return Err(StoreError::Adapter(
                "redacted audit evidence exceeds 256 KiB".into(),
            ));
        }
        let content_hash = hex::encode(Sha256::digest(&encoded));
        let target = self.target(evidence, &content_hash)?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::System,
                id: AUDIT_EXPORT_ACTOR.into(),
            },
            "audit.export.worm.write",
            target.as_str(),
            json!({
                "method": "PUT",
                "create_only": true,
                "body_base64": BASE64.encode(encoded),
                "content_sha256": content_hash,
            }),
        );
        request.capabilities = vec!["audit.export.worm.write".into()];
        request.credential_references = self
            .credential_reference
            .iter()
            .map(|reference| CredentialReference {
                reference: reference.clone(),
                value_hash: None,
            })
            .collect();
        request.context = ExecutionContext {
            correlation_id: format!("audit-export:{}", evidence.event_id),
            causation_id: Some(evidence.event_id.clone()),
            ..ExecutionContext::default()
        };
        self.gateway
            .execute(request, self.executor.as_ref())
            .await
            .map(|_| ())
            .map_err(|error| match error {
                GatewayError::OutcomeUnknown(message) => StoreError::OutcomeUnknown(message),
                error => StoreError::Adapter(error.to_string()),
            })
    }
}

fn valid_environment_reference(reference: &str) -> bool {
    reference.strip_prefix("env:").is_some_and(|name| {
        let mut bytes = name.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    })
}
