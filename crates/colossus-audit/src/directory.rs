use super::*;

/// Filesystem exporter whose writes always cross the effect gateway.
pub struct GatewayDirectoryAuditExporter {
    root: std::path::PathBuf,
    gateway: Arc<EffectGateway>,
    executor: Arc<dyn EffectExecutor>,
}

impl GatewayDirectoryAuditExporter {
    /// Bind an existing canonical directory to a permit-requiring filesystem executor.
    pub fn new(
        root: impl AsRef<Path>,
        gateway: Arc<EffectGateway>,
        executor: Arc<dyn EffectExecutor>,
    ) -> Result<Self, StoreError> {
        let root = std::fs::canonicalize(root).map_err(adapter)?;
        if !root.is_dir() {
            return Err(StoreError::Adapter(
                "audit export root must be an existing directory".into(),
            ));
        }
        Ok(Self {
            root,
            gateway,
            executor,
        })
    }

    fn target(&self, evidence: &AuditEvidence) -> std::path::PathBuf {
        self.root.join(format!(
            "{:020}-{}.json",
            evidence.global_sequence, evidence.event_id
        ))
    }
}

#[async_trait]
impl AuditExporter for GatewayDirectoryAuditExporter {
    fn kind(&self) -> &'static str {
        "directory-json"
    }

    async fn export(&self, evidence: &AuditEvidence) -> Result<(), StoreError> {
        let mut encoded = serde_json::to_string(evidence).map_err(adapter)?;
        encoded.push('\n');
        if encoded.len() > MAX_EVIDENCE_BYTES {
            return Err(StoreError::Adapter(
                "redacted audit evidence exceeds 256 KiB".into(),
            ));
        }
        let target = self.target(evidence);
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::System,
                id: AUDIT_EXPORT_ACTOR.into(),
            },
            "audit.export.write",
            target.to_string_lossy(),
            json!({
                "operation": "write",
                "mode": "overwrite",
                "text": encoded,
                "display_path": target.file_name().map(|name| name.to_string_lossy()),
            }),
        );
        request.capabilities = vec!["audit.export.write".into()];
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
