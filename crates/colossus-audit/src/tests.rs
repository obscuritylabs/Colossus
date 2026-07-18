use super::{
    AUDIT_EXPORT_ACTOR, AuditExportService, GatewayDirectoryAuditExporter,
    GatewayWormAuditExporter, evidence,
};
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, AuditEvidence, DecisionOutcome, EffectRequest, EventClassification,
    ExecutionContext, NewEvent, QuarantinedEffectResult,
};
use colossus_journal_redb::{Ed25519CheckpointSigner, RedbEventJournal, StaticKeyProvider};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit,
    SafetyKernel,
};
use colossus_ports::{AuditExporter, EventJournal, ExternalWorkQueue, ProjectionStore, StoreError};
use colossus_projection::JournalExternalWorkQueue;
use colossus_sandbox::FilesystemExecutor;
use colossus_testkit::{
    InMemoryEventJournal, InMemoryProjectionStore, assert_audit_exporter_conformance,
};
use serde_json::json;
use std::{
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

fn event(actor: Actor, stream: &str) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: stream.into(),
        expected_stream_version: 0,
        classification: EventClassification::Domain,
        event_type: "audit.fixture.v1".into(),
        actor,
        context: ExecutionContext {
            correlation_id: "audit-test".into(),
            ..ExecutionContext::default()
        },
        payload: json!({"secret_payload": "never export plaintext"}),
    }
}

fn queue(journal: Arc<dyn EventJournal>) -> Arc<dyn ExternalWorkQueue> {
    let store: Arc<dyn ProjectionStore> = Arc::new(InMemoryProjectionStore::default());
    Arc::new(JournalExternalWorkQueue::new(journal, store))
}

fn persistent_journal(path: &std::path::Path) -> Arc<RedbEventJournal> {
    Arc::new(
        RedbEventJournal::open(
            path,
            Arc::new(StaticKeyProvider::new("audit-crash-key", [71_u8; 32])),
            Arc::new(Ed25519CheckpointSigner::new(
                "audit-crash-signing",
                [72_u8; 32],
            )),
        )
        .expect("open persistent journal"),
    )
}

fn persistent_service(
    state: &std::path::Path,
    exports: &std::path::Path,
) -> (Arc<RedbEventJournal>, AuditExportService) {
    let journal = persistent_journal(state);
    let journal_port: Arc<dyn EventJournal> = journal.clone();
    let projection_store: Arc<dyn ProjectionStore> = journal.clone();
    let queue: Arc<dyn ExternalWorkQueue> = Arc::new(JournalExternalWorkQueue::new(
        Arc::clone(&journal_port),
        projection_store,
    ));
    let policy = BuiltInPolicy::offline_default()
        .with_action("audit.export.write", DecisionOutcome::Allow)
        .with_filesystem_root(exports.display().to_string(), "write");
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal_port),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.write".into()]),
        [73_u8; 32],
    ));
    let exporter: Arc<dyn AuditExporter> = Arc::new(
        GatewayDirectoryAuditExporter::new(exports, gateway, Arc::new(FilesystemExecutor::new()))
            .expect("directory exporter"),
    );
    (
        journal,
        AuditExportService::new(journal_port, queue, Some(exporter)),
    )
}

#[tokio::test]
async fn crash_after_export_child() {
    let (Ok(state), Ok(exports)) = (
        std::env::var("COLOSSUS_AUDIT_TEST_CRASH_STATE"),
        std::env::var("COLOSSUS_AUDIT_TEST_CRASH_EXPORTS"),
    ) else {
        return;
    };
    let (_, service) =
        persistent_service(std::path::Path::new(&state), std::path::Path::new(&exports));
    service
        .run_once(8)
        .await
        .expect("fault point must terminate before export returns");
    panic!("configured audit crash point did not terminate the child process");
}

#[tokio::test]
async fn crash_after_delivery_replays_idempotently_before_acknowledging_queue() {
    let directory = tempfile::tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let exports = directory.path().join("exports");
    std::fs::create_dir(&exports).expect("export directory");
    {
        let journal = persistent_journal(&state);
        journal
            .append(event(
                Actor {
                    actor_type: ActorType::User,
                    id: "operator".into(),
                },
                "fixture:crash-delivery",
            ))
            .expect("source event");
    }

    let child = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "tests::crash_after_export_child", "--nocapture"])
        .env("COLOSSUS_AUDIT_TEST_CRASH_STATE", &state)
        .env("COLOSSUS_AUDIT_TEST_CRASH_EXPORTS", &exports)
        .env("COLOSSUS_AUDIT_TEST_CRASH_POINT", "after_export_before_ack")
        .status()
        .expect("spawn audit crash child");
    assert!(!child.success(), "audit crash child unexpectedly succeeded");

    let delivered = std::fs::read_dir(&exports)
        .expect("export directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("exported evidence");
    assert_eq!(delivered.len(), 1);
    let (journal, service) = persistent_service(&state, &exports);
    assert_eq!(service.status().expect("status").position, 0);
    let before_replay = journal
        .read_global(1, 32)
        .expect("journal after delivery crash");
    assert!(before_replay.len() > 1);
    assert_ne!(before_replay[0].actor.id, AUDIT_EXPORT_ACTOR);
    assert!(before_replay[1..].iter().all(|event| {
        event.actor.actor_type == ActorType::System && event.actor.id == AUDIT_EXPORT_ACTOR
    }));

    let report = service.drain(8, 8).await.expect("replay and drain");
    assert_eq!(report.exported, 1);
    assert_eq!(
        report.skipped,
        u64::try_from(before_replay.len().saturating_sub(1))
            .expect("lifecycle count")
            .saturating_mul(2)
    );
    assert!(report.status.ready);
    assert_eq!(report.status.position, report.status.journal_head);
    assert_eq!(
        std::fs::read_dir(&exports)
            .expect("export directory")
            .count(),
        1
    );
    journal.verify().expect("post-replay journal verification");
}

#[derive(Default)]
struct RecordingExporter {
    evidence: Mutex<Vec<AuditEvidence>>,
}

#[derive(Default)]
struct RecordingEffectExecutor {
    requests: Mutex<Vec<EffectRequest>>,
}

#[async_trait]
impl EffectExecutor for RecordingEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.requests
            .lock()
            .map_err(|error| ExecutionError::Failed(error.to_string()))?
            .push(request.clone());
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: Vec::new(),
            effect_succeeded: true,
        })
    }
}

#[tokio::test]
async fn worm_export_is_create_only_redacted_and_credential_reference_only() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let stored = journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "operator".into(),
            },
            "fixture:worm",
        ))
        .expect("source event");
    let policy = BuiltInPolicy::offline_default()
        .with_action("audit.export.worm.write", DecisionOutcome::Allow)
        .with_network_destination("https://worm.example")
        .with_environment("WORM_TOKEN");
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.worm.write".into()]),
        [89_u8; 32],
    ));
    let executor = Arc::new(RecordingEffectExecutor::default());
    let exporter = GatewayWormAuditExporter::new(
        "https://worm.example/retained/",
        Some("env:WORM_TOKEN".into()),
        gateway,
        executor.clone(),
    )
    .expect("WORM exporter");
    let mut record = evidence(&stored);
    record.event_id = "../escape?query#fragment/slash".into();
    assert_audit_exporter_conformance(&exporter, &record).await;
    let requests = executor.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].action, "audit.export.worm.write");
    assert_eq!(requests[0].resource, requests[1].resource);
    assert!(requests[0].resource.ends_with(".json"));
    let target = url::Url::parse(&requests[0].resource).expect("object URL");
    assert_eq!(
        target.origin().ascii_serialization(),
        "https://worm.example"
    );
    assert!(target.path().starts_with("/retained/"));
    assert!(target.query().is_none());
    assert!(target.fragment().is_none());
    assert!(target.path().contains("%2F"));
    assert_eq!(requests[0].content["method"], "PUT");
    assert_eq!(requests[0].content["create_only"], true);
    assert_eq!(requests[0].credential_references.len(), 1);
    assert_eq!(
        requests[0].credential_references[0].reference,
        "env:WORM_TOKEN"
    );
    let encoded = requests[0].content["body_base64"]
        .as_str()
        .expect("encoded evidence");
    let body = BASE64.decode(encoded).expect("evidence body");
    let body = String::from_utf8(body).expect("UTF-8 evidence");
    assert!(!body.contains("never export plaintext"));
    assert!(!body.contains("ciphertext"));
    assert!(!body.contains("WORM_TOKEN"));
    assert!(!body.contains("token-value"));
}

#[tokio::test]
#[ignore = "requires COLOSSUS_TEST_WORM_ENDPOINT and COLOSSUS_TEST_WORM_TOKEN"]
async fn live_https_worm_endpoint_accepts_idempotent_create_only_delivery() {
    let (Ok(endpoint), Ok(_token)) = (
        std::env::var("COLOSSUS_TEST_WORM_ENDPOINT"),
        std::env::var("COLOSSUS_TEST_WORM_TOKEN"),
    ) else {
        return;
    };
    let origin = url::Url::parse(&endpoint)
        .expect("WORM endpoint URL")
        .origin()
        .ascii_serialization();
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let stored = journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "live-worm-acceptance".into(),
            },
            &format!("fixture:worm-live:{}", uuid::Uuid::now_v7()),
        ))
        .expect("source event");
    let policy = BuiltInPolicy::offline_default()
        .with_action("audit.export.worm.write", DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_environment("COLOSSUS_TEST_WORM_TOKEN");
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.worm.write".into()]),
        [87_u8; 32],
    ));
    let exporter = GatewayWormAuditExporter::new(
        &endpoint,
        Some("env:COLOSSUS_TEST_WORM_TOKEN".into()),
        gateway,
        Arc::new(colossus_sandbox::HttpExecutor::new()),
    )
    .expect("live WORM exporter");
    assert_audit_exporter_conformance(&exporter, &evidence(&stored)).await;
}

#[async_trait]
impl AuditExporter for RecordingExporter {
    fn kind(&self) -> &'static str {
        "recording"
    }

    async fn export(&self, evidence: &AuditEvidence) -> Result<(), StoreError> {
        self.evidence
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?
            .push(evidence.clone());
        Ok(())
    }
}

#[tokio::test]
async fn queue_exports_redacted_evidence_and_skips_its_own_lifecycle() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "operator".into(),
            },
            "fixture:one",
        ))
        .expect("source event");
    journal
        .append(event(
            Actor {
                actor_type: ActorType::System,
                id: AUDIT_EXPORT_ACTOR.into(),
            },
            "fixture:export-lifecycle",
        ))
        .expect("export lifecycle");
    let exporter = Arc::new(RecordingExporter::default());
    let exporter_port: Arc<dyn AuditExporter> = exporter.clone();
    let service = AuditExportService::new(
        Arc::clone(&journal),
        queue(Arc::clone(&journal)),
        Some(exporter_port),
    );
    let report = service.drain(8, 8).await.expect("drain");
    assert_eq!(report.examined, 2);
    assert_eq!(report.exported, 1);
    assert_eq!(report.skipped, 1);
    assert!(report.status.ready);
    let records = exporter.evidence.lock().expect("records");
    assert_eq!(records.len(), 1);
    let encoded = serde_json::to_string(&records[0]).expect("evidence JSON");
    assert!(!encoded.contains("ciphertext"));
    assert!(!encoded.contains("nonce"));
    assert!(!encoded.contains("never export plaintext"));
}

struct UnknownExporter {
    calls: AtomicU64,
}

#[async_trait]
impl AuditExporter for UnknownExporter {
    fn kind(&self) -> &'static str {
        "unknown"
    }

    async fn export(&self, _evidence: &AuditEvidence) -> Result<(), StoreError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(StoreError::OutcomeUnknown(
            "fixture export outcome is unknown".into(),
        ))
    }
}

#[tokio::test]
async fn unknown_export_outcome_is_durable_and_never_retried_implicitly() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "operator".into(),
            },
            "fixture:unknown",
        ))
        .expect("source event");
    let exporter = Arc::new(UnknownExporter {
        calls: AtomicU64::new(0),
    });
    let exporter_port: Arc<dyn AuditExporter> = exporter.clone();
    let service =
        AuditExportService::new(Arc::clone(&journal), queue(journal), Some(exporter_port));
    assert!(matches!(
        service.run_once(8).await,
        Err(StoreError::OutcomeUnknown(_))
    ));
    assert_eq!(exporter.calls.load(Ordering::Acquire), 1);
    assert!(
        !service
            .status()
            .expect("status")
            .retry
            .expect("retry")
            .retryable
    );
    assert!(matches!(
        service.run_once(8).await,
        Err(StoreError::OutcomeUnknown(_))
    ));
    assert_eq!(exporter.calls.load(Ordering::Acquire), 1);
}

struct TransientExporter {
    calls: AtomicU64,
}

#[async_trait]
impl AuditExporter for TransientExporter {
    fn kind(&self) -> &'static str {
        "transient"
    }

    async fn export(&self, _evidence: &AuditEvidence) -> Result<(), StoreError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(StoreError::Adapter(
            "fixture exporter is temporarily unavailable".into(),
        ))
    }
}

#[tokio::test]
async fn transient_failure_defers_immediate_retry_without_duplicate_delivery() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "operator".into(),
            },
            "fixture:transient",
        ))
        .expect("source event");
    let exporter = Arc::new(TransientExporter {
        calls: AtomicU64::new(0),
    });
    let exporter_port: Arc<dyn AuditExporter> = exporter.clone();
    let service =
        AuditExportService::new(Arc::clone(&journal), queue(journal), Some(exporter_port));
    assert!(matches!(
        service.run_once(8).await,
        Err(StoreError::Adapter(_))
    ));
    assert_eq!(exporter.calls.load(Ordering::Acquire), 1);
    let retry = service.status().expect("status").retry.expect("retry");
    assert!(retry.retryable);
    assert_eq!(retry.attempts, 1);
    assert!(retry.next_retry_at.is_some());
    assert!(matches!(
        service.run_once(8).await,
        Err(StoreError::Adapter(message)) if message.contains("retry is deferred")
    ));
    assert_eq!(exporter.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn directory_export_is_permit_bound_idempotent_and_ciphertext_free() {
    let directory = tempfile::tempdir().expect("directory");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let stored = journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "operator".into(),
            },
            "fixture:directory",
        ))
        .expect("source event");
    let policy = BuiltInPolicy::offline_default()
        .with_action("audit.export.write", DecisionOutcome::Allow)
        .with_filesystem_root(directory.path().display().to_string(), "write");
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.write".into()]),
        [91_u8; 32],
    ));
    let exporter = GatewayDirectoryAuditExporter::new(
        directory.path(),
        gateway,
        Arc::new(FilesystemExecutor::new()),
    )
    .expect("exporter");
    let record = evidence(&stored);
    assert_audit_exporter_conformance(&exporter, &record).await;
    let target = directory.path().join(format!(
        "{:020}-{}.json",
        stored.global_sequence, stored.event_id
    ));
    let output = std::fs::read_to_string(target).expect("evidence file");
    let output_value: serde_json::Value = serde_json::from_str(&output).expect("evidence JSON");
    assert!(output_value.get("ciphertext").is_none());
    assert!(output_value.get("nonce").is_none());
    assert!(!output.contains("never export plaintext"));
    assert!(output.contains(&stored.payload.plaintext_hash));
}

#[tokio::test]
async fn policy_denial_prevents_directory_write() {
    let directory = tempfile::tempdir().expect("directory");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let stored = journal
        .append(event(
            Actor {
                actor_type: ActorType::User,
                id: "operator".into(),
            },
            "fixture:denied",
        ))
        .expect("source event");
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.write".into()]),
        [92_u8; 32],
    ));
    let exporter = GatewayDirectoryAuditExporter::new(
        directory.path(),
        gateway,
        Arc::new(FilesystemExecutor::new()),
    )
    .expect("exporter");
    let error = exporter
        .export(&evidence(&stored))
        .await
        .expect_err("policy must deny export");
    assert!(matches!(error, StoreError::Adapter(_)));
    let entries = std::fs::read_dir(directory.path())
        .expect("read export directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("directory entries");
    assert!(entries.is_empty());
    assert!(
        journal
            .read_global(1, 16)
            .expect("journal events")
            .iter()
            .any(|event| event.event_type == "effect.denied.v1")
    );
}
