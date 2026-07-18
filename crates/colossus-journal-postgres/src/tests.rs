use super::{PostgresEventJournal, PostgresJournalConfig, PostgresTlsConfig};
use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
use colossus_journal_redb::{Ed25519CheckpointSigner, StaticKeyProvider};
use colossus_ports::{EventJournal, ProjectionStore, StoreError};
use colossus_projection::JournalExternalWorkQueue;
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::{
    assert_external_work_queue_conformance, assert_session_repository_conformance,
    assert_work_repository_conformance, assert_workflow_repository_conformance,
};
use colossus_testkit::{assert_journal_conformance, assert_projection_store_conformance};
use colossus_work::EventSourcedWorkRepository;
use colossus_workflow::EventSourcedWorkflowRepository;
use serde_json::json;
use std::{
    fs,
    process::Command,
    sync::{Arc, Barrier},
};
use uuid::Uuid;

fn event(stream: &str, version: u64, value: u64) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: stream.into(),
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: "test.recorded.v1".into(),
        actor: Actor {
            actor_type: ActorType::System,
            id: "postgres-test".into(),
        },
        context: ExecutionContext {
            correlation_id: "postgres-conformance".into(),
            ..ExecutionContext::default()
        },
        payload: json!({"value": value}),
    }
}

fn live_config() -> Option<PostgresJournalConfig> {
    std::env::var("COLOSSUS_TEST_POSTGRES_URL").ok()?;
    PostgresJournalConfig::new(
        "COLOSSUS_TEST_POSTGRES_URL",
        format!("colossus_test_{}", Uuid::now_v7().simple()),
        PostgresTlsConfig::Disabled,
    )
    .ok()
}

fn open(config: &PostgresJournalConfig) -> PostgresEventJournal {
    PostgresEventJournal::open(
        config.clone(),
        Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    )
    .expect("open PostgreSQL journal")
}

fn with_schema(config: &PostgresJournalConfig, suffix: &str) -> PostgresJournalConfig {
    PostgresJournalConfig {
        schema: format!("{}_{suffix}", config.schema),
        ..config.clone()
    }
}

#[test]
fn configuration_rejects_identifiers_and_does_not_echo_connection_values() {
    assert!(PostgresJournalConfig::new("bad-name", "valid", PostgresTlsConfig::Disabled).is_err());
    assert!(
        PostgresJournalConfig::new("DATABASE_URL", "bad-name", PostgresTlsConfig::Disabled)
            .is_err()
    );
    let config = PostgresJournalConfig::new(
        "COLOSSUS_INTENTIONALLY_MISSING_DATABASE_URL",
        "valid_schema",
        PostgresTlsConfig::Disabled,
    )
    .expect("valid reference-only config");
    let error = match PostgresEventJournal::open(
        config,
        Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    ) {
        Ok(_) => panic!("an unset connection variable must fail before opening"),
        Err(error) => error,
    };
    assert!(
        !error.to_string().contains("password"),
        "connection errors must not expose credential values"
    );
}

#[test]
fn tls_policy_defaults_to_pinned_webpki_and_rejects_invalid_custom_bundles() {
    let config: PostgresJournalConfig = serde_json::from_value(json!({
        "connectionVariable": "DATABASE_URL",
        "schema": "colossus"
    }))
    .expect("default PostgreSQL TLS config");
    assert_eq!(config.tls, PostgresTlsConfig::WebpkiRoots);
    PostgresEventJournal::build_tls_connector(&config.tls)
        .expect("pinned WebPKI roots build a rustls connector");
    assert!(PostgresEventJournal::build_tls_connector(&PostgresTlsConfig::Disabled).is_err());

    let path = std::env::temp_dir().join(format!(
        "colossus-invalid-postgres-ca-{}.pem",
        Uuid::now_v7()
    ));
    fs::write(&path, b"not a PEM certificate").expect("write invalid CA bundle");
    let error = match PostgresEventJournal::build_tls_connector(&PostgresTlsConfig::CustomCa {
        ca_pem_path: path.clone(),
    }) {
        Ok(_) => panic!("an invalid private CA bundle must fail closed"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "storage adapter failure: PostgreSQL CA bundle contains no certificates"
    );
    fs::remove_file(path).expect("remove invalid CA bundle");
}

#[test]
fn live_crash_append_child() {
    let (Ok(schema), Ok(point)) = (
        std::env::var("COLOSSUS_POSTGRES_TEST_CRASH_SCHEMA"),
        std::env::var("COLOSSUS_POSTGRES_TEST_CRASH_POINT"),
    ) else {
        return;
    };
    let config = PostgresJournalConfig::new(
        "COLOSSUS_TEST_POSTGRES_URL",
        schema,
        PostgresTlsConfig::Disabled,
    )
    .expect("crash config");
    open(&config)
        .append(event("crash-stream", 0, 1))
        .expect("configured fault must terminate the process");
    panic!("PostgreSQL crash point {point} did not terminate the child");
}

#[test]
fn live_kill_recovery_preserves_transaction_boundary_and_chain() {
    let Some(config) = live_config() else {
        return;
    };
    for (suffix, point, expected_events) in [
        ("before", "before_commit", 0_u64),
        ("after", "after_commit", 1_u64),
    ] {
        let crash_config = with_schema(&config, suffix);
        let child = Command::new(std::env::current_exe().expect("current test executable"))
            .args(["--exact", "tests::live_crash_append_child", "--nocapture"])
            .env("COLOSSUS_POSTGRES_TEST_CRASH_SCHEMA", &crash_config.schema)
            .env("COLOSSUS_POSTGRES_TEST_CRASH_POINT", point)
            .status()
            .expect("spawn PostgreSQL crash child");
        assert!(
            !child.success(),
            "PostgreSQL crash child unexpectedly succeeded"
        );
        let reopened = open(&crash_config);
        let report = reopened.verify().expect("verify after crash");
        assert_eq!(report.event_count, expected_events);
        assert_eq!(report.last_sequence, expected_events);
    }
}

#[test]
fn live_shared_journal_and_projection_conformance() {
    let Some(config) = live_config() else {
        return;
    };
    let journal = open(&config);
    assert_journal_conformance(
        &journal,
        event("conformance", 0, 1),
        event("conformance", 0, 2),
    );
    let projection_config = PostgresJournalConfig {
        schema: format!("{}_projection", config.schema),
        ..config
    };
    assert_projection_store_conformance(&open(&projection_config));
}

#[test]
fn live_concurrent_writers_preserve_one_global_chain_and_stream_conflicts() {
    let Some(config) = live_config() else {
        return;
    };
    let first = Arc::new(open(&config));
    let second = Arc::new(open(&config));
    let barrier = Arc::new(Barrier::new(2));
    let handles = [first, second]
        .into_iter()
        .enumerate()
        .map(|(index, journal)| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                journal.append(event("shared-stream", 0, index as u64))
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("writer thread"))
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::Conflict { .. })))
            .count(),
        1
    );
    let reopened = open(&config);
    assert_eq!(
        reopened
            .verify()
            .expect("verify concurrent chain")
            .event_count,
        1
    );
    reopened
        .append(event("other-stream", 0, 3))
        .expect("second global writer");
    assert_eq!(
        reopened.verify().expect("verify two streams").event_count,
        2
    );
}

#[test]
fn live_outage_is_sanitized_and_a_later_operation_reconnects() {
    let (Ok(_available_url), Ok(_unavailable_url)) = (
        std::env::var("COLOSSUS_TEST_POSTGRES_URL"),
        std::env::var("COLOSSUS_TEST_POSTGRES_OUTAGE_URL"),
    ) else {
        return;
    };
    let config = PostgresJournalConfig::new(
        "COLOSSUS_TEST_POSTGRES_URL",
        format!("colossus_test_{}", Uuid::now_v7().simple()),
        PostgresTlsConfig::Disabled,
    )
    .expect("outage config");
    let journal = open(&config);
    journal
        .append(event("before-outage", 0, 1))
        .expect("initial append");

    let unavailable = PostgresJournalConfig {
        connection_variable: "COLOSSUS_TEST_POSTGRES_OUTAGE_URL".into(),
        ..config.clone()
    };
    let outage = match PostgresEventJournal::open(
        unavailable,
        Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    ) {
        Ok(_) => panic!("unavailable database must fail"),
        Err(error) => error,
    };
    assert!(!outage.to_string().contains("credential-must-not-appear"));

    journal
        .append(event("after-outage", 0, 3))
        .expect("adapter reconnects after outage");
    assert_eq!(journal.verify().expect("verify recovery").event_count, 2);
}

#[test]
fn live_shared_repository_and_external_queue_conformance() {
    let Some(config) = live_config() else {
        return;
    };

    let sessions: Arc<dyn EventJournal> = Arc::new(open(&with_schema(&config, "sessions")));
    assert_session_repository_conformance(|| {
        Box::new(EventSourcedSessionRepository::new(Arc::clone(&sessions)))
    });

    let work: Arc<dyn EventJournal> = Arc::new(open(&with_schema(&config, "work")));
    assert_work_repository_conformance(|| {
        Box::new(EventSourcedWorkRepository::new(Arc::clone(&work)))
    });

    let workflows: Arc<dyn EventJournal> = Arc::new(open(&with_schema(&config, "workflows")));
    assert_workflow_repository_conformance(|| {
        Box::new(EventSourcedWorkflowRepository::new(Arc::clone(&workflows)))
    });

    let queue_journal = Arc::new(open(&with_schema(&config, "queue")));
    let journal: Arc<dyn EventJournal> = queue_journal.clone();
    let projection: Arc<dyn ProjectionStore> = queue_journal;
    let queue = JournalExternalWorkQueue::new(Arc::clone(&journal), projection);
    assert_external_work_queue_conformance(
        journal.as_ref(),
        &queue,
        event("queue-one", 0, 1),
        event("queue-two", 0, 2),
    );
}
