//! Runtime composition root. Interfaces call this layer and own no product logic.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, DecisionOutcome, EffectRequest, EventClassification, ExecutionContext,
    NewEvent, ProjectionStatus, QuarantinedEffectResult,
};
use colossus_journal_redb::{
    Ed25519CheckpointSigner, EnvironmentKeyProvider, PlatformKeyProvider, RedbEventJournal,
    RedbWriterLease, platform_secret,
};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit,
    GatewayError, OpaConfig, OpaPolicy, ReleasedEffectResult, SafetyKernel, effect_request,
    system_actor,
};
use colossus_ports::{
    EventJournal, KeyProvider, PolicyDecisionPoint, ProjectionStore, SessionRepository, StoreError,
    WorkRepository, WorkflowRepository,
};
use colossus_projection::{
    ProjectedSessionRepository, ProjectedWorkRepository, ProjectionRunReport, ProjectionWorker,
    default_handlers,
};
use colossus_workflow::{
    EventSourcedWorkflowRepository, ValidatedWorkflow, WorkflowEffect, WorkflowEffectRunner,
    WorkflowError, WorkflowService, validate_definition,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use uuid::Uuid;

/// Strict fresh Rust runtime configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Configuration schema version.
    pub schema_version: u16,
    /// Canonical journal and key settings.
    pub storage: StorageConfig,
    /// Policy decision point settings.
    pub policy: PolicyConfig,
    /// Workflow definition libraries.
    pub workflows: WorkflowLibraryConfig,
}

/// Canonical storage configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageConfig {
    /// Fresh redb state file.
    pub path: PathBuf,
    /// Mandatory key provider.
    pub keys: KeyConfig,
}

/// Mandatory encryption/signing key provider configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyConfig {
    /// OS Keychain, DPAPI, or Secret Service.
    Platform {
        /// Credential-store service namespace.
        service: String,
        /// Journal encryption key identifier.
        journal_key_id: String,
        /// Checkpoint signing key identifier.
        signing_key_id: String,
    },
    /// Explicit environment credentials for headless/airgapped deployments.
    Environment {
        /// Environment variable containing the journal key.
        journal_variable: String,
        /// Journal key identifier.
        journal_key_id: String,
        /// Environment variable containing the signing key.
        signing_variable: String,
        /// Separately persisted secure anchor path.
        anchor_path: PathBuf,
    },
}

/// Policy configuration. Unknown fields fail deserialization.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PolicyConfig {
    /// Offline deny-by-default policy.
    BuiltIn {
        /// Additional exact actions to allow.
        #[serde(default)]
        allow_actions: Vec<String>,
        /// Exact actions that require approval and re-evaluation.
        #[serde(default)]
        approval_actions: Vec<String>,
        /// Require post-effect content authorization.
        #[serde(default)]
        require_post_effect: bool,
    },
    /// OPA REST policy with strict disclosure/TLS requirements.
    Opa {
        /// OPA base URL.
        base_url: String,
        /// Fixed OPA data decision path.
        decision_path: String,
        /// Optional pinned PEM CA path; required remotely.
        ca_pem_path: Option<PathBuf>,
        /// Optional PEM mTLS identity path; required remotely.
        identity_pem_path: Option<PathBuf>,
        /// Explicit full logical content disclosure acknowledgement.
        full_content_disclosure_acknowledged: bool,
        /// Whether decision logs were disabled or masking verified.
        decision_log_masking_verified: bool,
        /// Transport timeout in milliseconds.
        timeout_ms: u64,
    },
}

/// Repository and user workflow libraries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowLibraryConfig {
    /// Repository workflow directory.
    pub repository: PathBuf,
    /// Platform user workflow directory.
    pub user: PathBuf,
}

impl RuntimeConfig {
    /// Strictly parse YAML with no unknown fields.
    pub fn from_yaml(yaml: &str) -> Result<Self, RuntimeError> {
        let config: Self = serde_saphyr::from_str(yaml)
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        if config.schema_version != 1 {
            return Err(RuntimeError::Config(
                "schemaVersion must be exactly 1".into(),
            ));
        }
        Ok(config)
    }

    /// Read and strictly parse a YAML file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        Self::from_yaml(&fs::read_to_string(path).map_err(RuntimeError::Io)?)
    }

    /// Safe offline configuration template using the platform credential store.
    pub fn offline_template(state_path: impl Into<PathBuf>) -> Self {
        let instance_id = Uuid::now_v7();
        Self {
            schema_version: 1,
            storage: StorageConfig {
                path: state_path.into(),
                keys: KeyConfig::Platform {
                    service: "dev.colossus.runtime".into(),
                    journal_key_id: format!("journal-{instance_id}"),
                    signing_key_id: format!("checkpoint-{instance_id}"),
                },
            },
            policy: PolicyConfig::BuiltIn {
                allow_actions: Vec::new(),
                approval_actions: Vec::new(),
                require_post_effect: false,
            },
            workflows: WorkflowLibraryConfig {
                repository: PathBuf::from(".colossus/workflows"),
                user: PathBuf::from("workflows"),
            },
        }
    }

    /// Render fresh YAML without resolving or exposing secrets.
    pub fn to_yaml(&self) -> Result<String, RuntimeError> {
        serde_saphyr::to_string(self).map_err(|error| RuntimeError::Config(error.to_string()))
    }
}

/// Runtime construction or application failure.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Strict configuration failed.
    #[error("configuration error: {0}")]
    Config(String),
    /// Filesystem read/write failed before runtime composition.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Journal/key adapter failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Effect authorization or execution failed.
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    /// Workflow validation or execution failed.
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
}

fn explicit_secret(variable: &str) -> Result<[u8; 32], RuntimeError> {
    let encoded = std::env::var(variable)
        .map_err(|_| RuntimeError::Config(format!("environment variable {variable} is unset")))?;
    let decoded = hex::decode(&encoded)
        .or_else(|_| BASE64.decode(&encoded))
        .map_err(|_| {
            RuntimeError::Config(format!(
                "environment variable {variable} must be hex or base64"
            ))
        })?;
    decoded.try_into().map_err(|_| {
        RuntimeError::Config(format!(
            "environment variable {variable} must decode to exactly 32 bytes"
        ))
    })
}

fn read_optional(path: Option<&PathBuf>) -> Result<Option<Vec<u8>>, RuntimeError> {
    path.map(fs::read).transpose().map_err(Into::into)
}

/// Fully composed auditable runtime.
pub struct Runtime {
    writer_lease: RedbWriterLease,
    journal: Arc<dyn EventJournal>,
    recovery_reason: Option<String>,
    projections: Arc<ProjectionWorker>,
    sessions: Arc<dyn SessionRepository>,
    work: Arc<dyn WorkRepository>,
    policy: Arc<dyn PolicyDecisionPoint>,
    gateway: Arc<EffectGateway>,
    workflow_repository: Arc<dyn WorkflowRepository>,
    workflows: Arc<WorkflowService>,
}

impl Runtime {
    /// Compose mandatory encryption, journal verification, policy, gateway, and workflows.
    pub fn open(config: &RuntimeConfig) -> Result<Self, RuntimeError> {
        if let Some(parent) = config.storage.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let writer_lease = RedbWriterLease::acquire(&config.storage.path)?;
        let (keys, signing_key_id, signing_key): (Arc<dyn KeyProvider>, String, [u8; 32]) =
            match &config.storage.keys {
                KeyConfig::Platform {
                    service,
                    journal_key_id,
                    signing_key_id,
                } => (
                    Arc::new(PlatformKeyProvider::new(service, journal_key_id)?),
                    signing_key_id.clone(),
                    platform_secret(service, &format!("signing-key:{signing_key_id}"))?,
                ),
                KeyConfig::Environment {
                    journal_variable,
                    journal_key_id,
                    signing_variable,
                    anchor_path,
                } => (
                    Arc::new(EnvironmentKeyProvider::new(
                        journal_variable,
                        journal_key_id,
                        anchor_path,
                    )),
                    "environment-checkpoint-v1".into(),
                    explicit_secret(signing_variable)?,
                ),
            };
        let signer = Arc::new(Ed25519CheckpointSigner::new(signing_key_id, signing_key));
        let redb = Arc::new(RedbEventJournal::open(&config.storage.path, keys, signer)?);
        let recovery_reason = redb.recovery_reason()?;
        let journal: Arc<dyn EventJournal> = redb.clone();
        let projection_store: Arc<dyn ProjectionStore> = redb;
        let projections = Arc::new(ProjectionWorker::new(
            Arc::clone(&journal),
            Arc::clone(&projection_store),
            default_handlers(),
        )?);
        let sessions: Arc<dyn SessionRepository> = Arc::new(ProjectedSessionRepository::new(
            Arc::clone(&projection_store),
        ));
        let work: Arc<dyn WorkRepository> =
            Arc::new(ProjectedWorkRepository::new(Arc::clone(&projection_store)));
        if !journal.is_recovery_mode() {
            recover_unknown_effects(journal.as_ref())?;
        }
        let policy: Arc<dyn PolicyDecisionPoint> = match &config.policy {
            PolicyConfig::BuiltIn {
                allow_actions,
                approval_actions,
                require_post_effect,
            } => {
                let mut policy =
                    BuiltInPolicy::offline_default().with_post_effect(*require_post_effect);
                policy = policy.with_action("filesystem.read", DecisionOutcome::Allow);
                for root in [&config.workflows.repository, &config.workflows.user] {
                    if let Ok(root) = absolute_path(root).and_then(fs::canonicalize) {
                        policy = policy.with_filesystem_read_root(root.display().to_string());
                    }
                }
                for action in allow_actions {
                    policy = policy.with_action(action, DecisionOutcome::Allow);
                }
                for action in approval_actions {
                    policy = policy.with_action(action, DecisionOutcome::RequireApproval);
                }
                Arc::new(policy)
            }
            PolicyConfig::Opa {
                base_url,
                decision_path,
                ca_pem_path,
                identity_pem_path,
                full_content_disclosure_acknowledged,
                decision_log_masking_verified,
                timeout_ms,
            } => Arc::new(
                OpaPolicy::new(OpaConfig {
                    base_url: base_url.clone(),
                    decision_path: decision_path.clone(),
                    ca_pem: read_optional(ca_pem_path.as_ref())?,
                    identity_pem: read_optional(identity_pem_path.as_ref())?,
                    full_content_disclosure_acknowledged: *full_content_disclosure_acknowledged,
                    decision_log_masking_verified: *decision_log_masking_verified,
                    timeout: Duration::from_millis(*timeout_ms),
                })
                .map_err(GatewayError::from)?,
            ),
        };
        let permit_key = match &config.storage.keys {
            KeyConfig::Platform {
                service,
                journal_key_id,
                ..
            } => platform_secret(service, &format!("permit-mac:{journal_key_id}"))?,
            KeyConfig::Environment {
                signing_variable, ..
            } => {
                let signing = explicit_secret(signing_variable)?;
                sha2_compat(&signing, b"colossus-permit-mac-v1")
            }
        };
        let gateway = Arc::new(EffectGateway::new(
            Arc::clone(&journal),
            Arc::clone(&policy),
            Arc::new(DenyApproval),
            SafetyKernel::new([
                "provider.echo".to_owned(),
                "workflow.execute".to_owned(),
                "filesystem.read".to_owned(),
            ]),
            permit_key,
        ));
        let workflow_repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let effects = Arc::new(GatewayWorkflowEffects {
            gateway: Arc::clone(&gateway),
        });
        let workflows = Arc::new(WorkflowService::new(
            Arc::clone(&journal),
            Arc::clone(&workflow_repository),
            effects,
        ));
        if !journal.is_recovery_mode() {
            workflows.recover_interrupted()?;
            projections.drain(256, 16_384)?;
        }
        Ok(Self {
            writer_lease,
            journal,
            recovery_reason,
            projections,
            sessions,
            work,
            policy,
            gateway,
            workflow_repository,
            workflows,
        })
    }

    /// Authoritative event journal for bounded audit views.
    pub fn journal(&self) -> Arc<dyn EventJournal> {
        Arc::clone(&self.journal)
    }

    /// Durable workflow application API.
    pub fn workflows(&self) -> Arc<WorkflowService> {
        Arc::clone(&self.workflows)
    }

    /// Exact workflow definition repository for list/show surfaces.
    pub fn workflow_repository(&self) -> Arc<dyn WorkflowRepository> {
        Arc::clone(&self.workflow_repository)
    }

    /// Current session snapshots served by the disposable session projection.
    pub fn session_repository(&self) -> Arc<dyn SessionRepository> {
        Arc::clone(&self.sessions)
    }

    /// Current task, decision, plan, and goal snapshots.
    pub fn work_repository(&self) -> Arc<dyn WorkRepository> {
        Arc::clone(&self.work)
    }

    /// Projection position, lag, and readiness for every built-in reducer.
    pub fn projection_status(&self) -> Result<Vec<ProjectionStatus>, RuntimeError> {
        self.projections.status().map_err(Into::into)
    }

    /// Catch all built-in projections up to the current journal head.
    pub fn drain_projections(&self) -> Result<ProjectionRunReport, RuntimeError> {
        self.projections.drain(256, 16_384).map_err(Into::into)
    }

    /// Delete and replay one projection, or every projection when omitted.
    pub fn rebuild_projection(
        &self,
        name: Option<&str>,
    ) -> Result<ProjectionRunReport, RuntimeError> {
        name.map_or_else(
            || self.projections.rebuild_all(),
            |projection| self.projections.rebuild(projection),
        )
        .map_err(Into::into)
    }

    /// Bounded local storage health report without decrypted event payloads.
    pub fn state_doctor(&self) -> Result<Value, RuntimeError> {
        let (journal_head, record_hash) = self.journal.head()?;
        Ok(json!({
            "recovery_mode": self.journal.is_recovery_mode(),
            "recovery_reason": self.recovery_reason,
            "journal_head": journal_head,
            "record_hash": record_hash,
            "writer_lease": {
                "held": true,
                "path": self.writer_lease.path(),
            },
            "projection_store": {
                "adapter": "redb",
                "positions": self.projection_status()?,
            },
            "repository_adapters": {
                "sessions": "redb-projection:sessions-v1",
                "work": "redb-projection:work-v1",
                "memory": "event-journal+redb-projection:memory-v1",
                "workflows": "event-journal+redb-projection:workflows-v1",
            }
        }))
    }

    /// Policy readiness and decision-log safety status.
    pub async fn policy_doctor(&self) -> Result<Value, RuntimeError> {
        self.policy
            .doctor()
            .await
            .map_err(GatewayError::from)
            .map_err(Into::into)
    }

    /// Credential-free, network-free smoke provider routed through policy and journal.
    pub async fn echo(&self, message: &str) -> Result<ReleasedEffectResult, RuntimeError> {
        let request = effect_request(
            system_actor("offline-echo"),
            "provider.echo",
            "provider:echo",
            json!({"message": message}),
        );
        self.gateway
            .execute(request, &EchoExecutor)
            .await
            .map_err(Into::into)
    }

    /// Read bounded UTF-8 text through the universal filesystem effect boundary.
    pub async fn read_text_file(&self, path: impl AsRef<Path>) -> Result<String, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.read",
            path.display().to_string(),
            json!({"path": path.display().to_string(), "encoding": "utf-8"}),
        );
        request.capabilities = vec!["filesystem.read".into()];
        let result = self.gateway.execute(request, &FileReadExecutor).await?;
        String::from_utf8(result.bytes)
            .map_err(|error| RuntimeError::Config(format!("file is not valid UTF-8: {error}")))
    }

    /// Read and validate a workflow path through policy and post-effect release.
    pub async fn validate_workflow_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ValidatedWorkflow, RuntimeError> {
        let yaml = self.read_text_file(path).await?;
        validate_definition(&yaml).map_err(Into::into)
    }

    /// Read, validate, and register a workflow path without bypassing the gateway.
    pub async fn register_workflow_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ValidatedWorkflow, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let yaml = self.read_text_file(&path).await?;
        self.workflows
            .register_definition(&yaml, &format!("repo:{}", path.display()))
            .map_err(Into::into)
    }

    /// Sign the current chain head for clean shutdown.
    pub fn checkpoint(&self) -> Result<(), RuntimeError> {
        if self.journal.is_recovery_mode() {
            return Ok(());
        }
        self.drain_projections()?;
        self.journal.checkpoint()?;
        Ok(())
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir().map(|directory| directory.join(path))
    }
}

fn recover_unknown_effects(journal: &dyn EventJournal) -> Result<u64, StoreError> {
    let mut last_by_stream = std::collections::BTreeMap::new();
    for event in journal.read_global(1, usize::MAX)? {
        if event.stream_id.starts_with("effect:") {
            last_by_stream.insert(event.stream_id.clone(), event);
        }
    }
    let mut recovered = 0_u64;
    for event in last_by_stream.into_values() {
        if event.event_type != "effect.started.v1" {
            continue;
        }
        journal.append(NewEvent {
            event_version: 1,
            stream_id: event.stream_id,
            expected_stream_version: event.stream_version,
            classification: EventClassification::Effect,
            event_type: "effect.outcome_unknown.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "startup-recovery".into(),
            },
            context: event.context,
            payload: json!({
                "reason": "process stopped after effect.started without a terminal event",
                "recovered_from_event_id": event.event_id,
                "automatic_retry": false,
            }),
        })?;
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

fn sha2_compat(secret: &[u8; 32], label: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    // The journal signing secret is already random. This local KDF only domain-separates
    // the permit MAC key without persisting another environment credential.
    Sha256::new()
        .chain_update(label)
        .chain_update(secret)
        .finalize()
        .into()
}

struct EchoExecutor;

#[async_trait]
impl EffectExecutor for EchoExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let message = request
            .content
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| ExecutionError::Failed("echo message is missing".into()))?;
        Ok(QuarantinedEffectResult {
            media_type: "text/plain; charset=utf-8".into(),
            bytes: message.as_bytes().to_vec(),
            effect_succeeded: true,
        })
    }
}

struct FileReadExecutor;

#[async_trait]
impl EffectExecutor for FileReadExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let path = fs::canonicalize(&request.resource)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let allowed = permit.obligations().filesystem.iter().any(|grant| {
            matches!(grant.mode.as_str(), "read" | "write")
                && fs::canonicalize(&grant.root).is_ok_and(|root| path.starts_with(root))
        });
        if !allowed {
            return Err(ExecutionError::Failed(
                "canonical path is outside permit roots".into(),
            ));
        }
        let metadata =
            fs::metadata(&path).map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if !metadata.is_file() {
            return Err(ExecutionError::Failed(
                "filesystem.read requires a regular file".into(),
            ));
        }
        if metadata.len() > permit.obligations().max_output_bytes {
            return Err(ExecutionError::Failed(
                "file exceeds permit output bound".into(),
            ));
        }
        let bytes = fs::read(path).map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "text/plain; charset=utf-8".into(),
            bytes,
            effect_succeeded: true,
        })
    }
}

struct UnavailableExecutor;

#[async_trait]
impl EffectExecutor for UnavailableExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        Err(ExecutionError::Failed(format!(
            "no adapter registered for {}",
            request.action
        )))
    }
}

struct GatewayWorkflowEffects {
    gateway: Arc<EffectGateway>,
}

#[async_trait]
impl WorkflowEffectRunner for GatewayWorkflowEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError> {
        let action = if effect.action == "echo" {
            "provider.echo".to_owned()
        } else {
            effect.action.clone()
        };
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::Workflow,
                id: effect.run_id.clone(),
            },
            action,
            format!("workflow-step:{}", effect.step_id),
            effect.content,
        );
        request.capabilities = vec!["workflow.execute".into()];
        request.idempotency_id = effect.idempotency;
        request.context = ExecutionContext {
            correlation_id: effect.run_id.clone(),
            run_id: Some(effect.run_id.clone()),
            workflow_id: Some(effect.run_id),
            workflow_hash: Some(effect.workflow_hash),
            step_id: Some(effect.step_id),
            attempt: Some(effect.attempt),
            ..ExecutionContext::default()
        };
        let executor: &dyn EffectExecutor = if request.action == "provider.echo" {
            &EchoExecutor
        } else {
            &UnavailableExecutor
        };
        match self.gateway.execute(request, executor).await {
            Ok(result) => Ok(json!({
                "media_type": result.media_type,
                "text": String::from_utf8_lossy(&result.bytes),
            })),
            Err(GatewayError::OutcomeUnknown(message)) => {
                Err(WorkflowError::OutcomeUnknown(message))
            }
            Err(error) => Err(WorkflowError::Effect(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FileReadExecutor, RuntimeConfig, recover_unknown_effects};
    use colossus_contracts::{
        Actor, ActorType, DecisionOutcome, EventClassification, ExecutionContext, NewEvent,
    };
    use colossus_ports::EventJournal;
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::{fs, sync::Arc};
    use tempfile::tempdir;

    #[test]
    fn strict_config_rejects_unknown_fields() {
        let yaml = r#"
schemaVersion: 1
storage:
  path: state.redb
  keys:
    kind: platform
    service: test
    journal_key_id: journal
    signing_key_id: signing
policy:
  kind: built_in
  allow_actions: []
  approval_actions: []
  require_post_effect: false
workflows:
  repository: .colossus/workflows
  user: workflows
surprise: true
"#;
        assert!(RuntimeConfig::from_yaml(yaml).is_err());
    }

    #[test]
    fn startup_marks_started_effects_unknown_without_retrying() {
        let journal = InMemoryEventJournal::default();
        journal
            .append(NewEvent {
                event_version: 1,
                stream_id: "effect:request-1".into(),
                expected_stream_version: 0,
                classification: EventClassification::Effect,
                event_type: "effect.started.v1".into(),
                actor: Actor {
                    actor_type: ActorType::System,
                    id: "test".into(),
                },
                context: ExecutionContext {
                    correlation_id: "correlation".into(),
                    ..ExecutionContext::default()
                },
                payload: json!({}),
            })
            .expect("started event");
        assert_eq!(recover_unknown_effects(&journal).expect("recover"), 1);
        assert_eq!(recover_unknown_effects(&journal).expect("idempotent"), 0);
        let events = journal
            .read_stream("effect:request-1")
            .expect("effect stream");
        assert_eq!(
            events.last().expect("terminal event").event_type,
            "effect.outcome_unknown.v1"
        );
    }

    #[tokio::test]
    async fn filesystem_adapter_cannot_escape_permitted_root_and_uses_post_release() {
        let allowed = tempdir().expect("allowed root");
        let denied = tempdir().expect("denied root");
        let allowed_file = allowed.path().join("workflow.yaml");
        let denied_file = denied.path().join("secret.txt");
        fs::write(&allowed_file, "safe").expect("allowed file");
        fs::write(&denied_file, "secret").expect("denied file");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = colossus_policy::BuiltInPolicy::offline_default()
            .with_action("filesystem.read", DecisionOutcome::Allow)
            .with_filesystem_read_root(allowed.path().display().to_string());
        let gateway = colossus_policy::EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(colossus_policy::DenyApproval),
            colossus_policy::SafetyKernel::new(["filesystem.read".into()]),
            [4_u8; 32],
        );

        let mut allowed_request = colossus_policy::effect_request(
            colossus_policy::system_actor("test"),
            "filesystem.read",
            allowed_file.display().to_string(),
            json!({"path": allowed_file}),
        );
        allowed_request.capabilities = vec!["filesystem.read".into()];
        let released = gateway
            .execute(allowed_request, &FileReadExecutor)
            .await
            .expect("allowed read");
        assert_eq!(released.bytes, b"safe");
        assert!(
            journal
                .read_global(1, 20)
                .expect("events")
                .iter()
                .any(|event| event.event_type == "effect.release_requested.v1")
        );

        let mut denied_request = colossus_policy::effect_request(
            colossus_policy::system_actor("test"),
            "filesystem.read",
            denied_file.display().to_string(),
            json!({"path": denied_file}),
        );
        denied_request.capabilities = vec!["filesystem.read".into()];
        let error = gateway
            .execute(denied_request, &FileReadExecutor)
            .await
            .expect_err("path escape denied");
        assert!(matches!(error, colossus_policy::GatewayError::Safety(_)));
    }
}
