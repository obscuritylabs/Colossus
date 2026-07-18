use super::{
    ChromaExecutor, ChromaMemoryIndex, ChromaProfile, GatewayOpenAiEmbeddingProvider,
    LocalHashEmbeddingProvider, OpenAiEmbeddingExecutor, OpenAiEmbeddingProfile, ProjectionState,
    persist_position, read_position,
};
use colossus_contracts::DecisionOutcome;
use colossus_policy::{BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel};
use colossus_ports::{EmbeddingProvider, EventJournal, MemoryIndex};
use colossus_testkit::{InMemoryEventJournal, assert_memory_index_conformance};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};

#[tokio::test]
async fn local_embeddings_are_deterministic_normalized_and_distinct() {
    let provider = LocalHashEmbeddingProvider::new(128).expect("profile");
    let first = provider.embed("Rust audit journal").await.expect("embed");
    let same = provider.embed("Rust audit journal").await.expect("embed");
    let different = provider
        .embed("semantic memory search")
        .await
        .expect("embed");
    assert_eq!(first, same);
    assert_ne!(first, different);
    let norm = first.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.000_1);
}

#[test]
fn position_metadata_round_trips_and_rejects_unknown_fields() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("position.json");
    assert_eq!(
        read_position(&path).expect("missing position"),
        ProjectionState::default()
    );
    let state = ProjectionState {
        position: 42,
        outcome_unknown: true,
    };
    persist_position(&path, state).expect("persist");
    assert_eq!(read_position(&path).expect("position"), state);
    std::fs::write(
        &path,
        br#"{"schemaVersion":1,"position":42,"outcomeUnknown":true,"unexpected":true}"#,
    )
    .expect("write");
    assert!(read_position(&path).is_err());
}

#[tokio::test]
async fn chroma_conformance_runs_only_through_audited_effects() {
    let fixture = ChromaFixture::start().await;
    let (index, journal) = fixture.index(true).expect("index");
    index
        .upsert(
            "event-1",
            "memory-1",
            "Rust audit journal",
            &serde_json::json!({"scope": "global"}),
            None,
        )
        .await
        .expect("upsert");
    let candidates = index.search("audit journal", 4).await.expect("search");
    assert_eq!(candidates, vec![("memory-1".into(), 0.8)]);
    let status = index.status().await.expect("status");
    assert_eq!(status["kind"], "chroma");
    assert_eq!(status["documents"], 1);
    index.remove("event-2", "memory-1").await.expect("remove");
    index
        .rebuild(&[(
            "memory-2".into(),
            "durable workflow".into(),
            serde_json::json!({"scope": "global"}),
        )])
        .await
        .expect("rebuild");
    index.set_position(17).await.expect("position");
    assert_eq!(index.position().expect("position"), 17);

    let events = journal.read_global(1, 1_000).expect("events");
    let requested = events
        .iter()
        .filter(|event| event.event_type == "effect.requested.v1")
        .count();
    assert_eq!(requested, 6);
    let requests = fixture.requests.lock().expect("requests").clone();
    assert_eq!(requests.len(), 11);
    assert!(requests.iter().any(|request| request.starts_with(
        "POST /api/v2/tenants/default_tenant/databases/default_database/collections HTTP/1.1"
    )));
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/upsert HTTP/1.1"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/query HTTP/1.1"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/count HTTP/1.1"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/delete HTTP/1.1"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("DELETE "))
    );
    fixture.task.abort();
}

#[tokio::test]
async fn chroma_index_passes_shared_conformance() {
    let fixture = ChromaFixture::start().await;
    let (index, journal) = fixture.index(true).expect("index");
    assert_memory_index_conformance(&index).await;
    let events = journal.read_global(1, 1_000).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.requested.v1")
    );
    fixture.task.abort();
}

#[tokio::test]
async fn denied_chroma_effect_never_reaches_the_network() {
    let fixture = ChromaFixture::start().await;
    let (index, journal) = fixture.index(false).expect("index");
    assert!(
        index
            .upsert(
                "event-1",
                "memory-1",
                "denied content",
                &serde_json::json!({}),
                None,
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.accepted.load(Ordering::Acquire), 0);
    let events = journal.read_global(1, 100).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.denied.v1")
    );
    fixture.task.abort();
}

#[tokio::test]
async fn interrupted_chroma_mutation_is_audited_as_outcome_unknown() {
    let fixture = ChromaFixture::start().await;
    let (index, journal) = fixture.index(true).expect("index");
    fixture.task.abort();
    tokio::task::yield_now().await;
    assert!(
        index
            .upsert(
                "event-unknown",
                "memory-unknown",
                "external outcome cannot be proven",
                &serde_json::json!({}),
                None,
            )
            .await
            .is_err()
    );
    let events = journal.read_global(1, 100).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.outcome_unknown.v1")
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "effect.failed.v1")
    );
    let accepted = fixture.accepted.load(Ordering::Acquire);
    assert!(
        index
            .upsert(
                "event-retry",
                "memory-unknown",
                "automatic retry must remain blocked",
                &serde_json::json!({}),
                None,
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.accepted.load(Ordering::Acquire), accepted);
    assert_eq!(
        index.status().await.expect("status")["outcome_unknown"],
        true
    );
    let (reopened, _) = fixture.index(false).expect("reopen");
    assert_eq!(
        reopened.status().await.expect("reopened status")["outcome_unknown"],
        true
    );
}

#[tokio::test]
async fn explicit_rebuild_clears_durable_unknown_outcome_marker() {
    let fixture = ChromaFixture::start().await;
    let path = fixture.directory.path().join("position.json");
    persist_position(
        &path,
        ProjectionState {
            position: 19,
            outcome_unknown: true,
        },
    )
    .expect("unknown marker");
    let (index, _) = fixture.index(true).expect("index");
    assert_eq!(
        index.status().await.expect("blocked status")["ready"],
        false
    );
    assert_eq!(fixture.accepted.load(Ordering::Acquire), 0);
    index.rebuild(&[]).await.expect("explicit rebuild");
    assert!(!read_position(&path).expect("position").outcome_unknown);
    assert_eq!(index.status().await.expect("ready status")["ready"], true);
    fixture.task.abort();
}

#[tokio::test]
async fn openai_compatible_embeddings_are_permit_bound_and_strictly_normalized() {
    let fixture = ChromaFixture::start().await;
    let profile = OpenAiEmbeddingProfile::new(
        "fixture",
        "embed-test",
        format!("{}/v1", fixture.origin),
        None,
        5_000,
        Some(3),
    )
    .expect("profile");
    let executor = Arc::new(OpenAiEmbeddingExecutor::new(profile.clone()));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("embedding.openai.create", DecisionOutcome::Allow)
                .with_network_destination(&fixture.origin),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["embedding.openai.create".into()]),
        [41_u8; 32],
    ));
    let provider = GatewayOpenAiEmbeddingProvider::new(gateway, executor, profile);
    assert_eq!(
        provider
            .embed("bounded semantic input")
            .await
            .expect("embed"),
        vec![0.1, 0.2, 0.3]
    );
    let events = journal.read_global(1, 100).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.requested.v1")
    );
    assert!(
        fixture
            .requests
            .lock()
            .expect("requests")
            .iter()
            .any(|request| request.contains("/v1/embeddings HTTP/1.1"))
    );
    fixture.task.abort();
}

struct ChromaFixture {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    accepted: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
    directory: tempfile::TempDir,
}

impl ChromaFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let requests_for_task = Arc::clone(&requests);
        let accepted_for_task = Arc::clone(&accepted);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                accepted_for_task.fetch_add(1, Ordering::AcqRel);
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4_096];
                let header_end = loop {
                    let Ok(read) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end.saturating_add(content_length) {
                    let Ok(read) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let request_line = headers.lines().next().unwrap_or_default().to_owned();
                requests_for_task
                    .lock()
                    .expect("requests")
                    .push(request_line.clone());
                let body = fixture_response(&request_line);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                if stream.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        Self {
            origin: format!("http://{address}"),
            requests,
            accepted,
            task,
            directory: tempfile::tempdir().expect("directory"),
        }
    }

    fn index(
        &self,
        allow: bool,
    ) -> Result<(ChromaMemoryIndex, Arc<dyn EventJournal>), colossus_ports::StoreError> {
        let profile = ChromaProfile::new(
            &self.origin,
            "default_tenant",
            "default_database",
            "colossus-memory",
            None,
            5_000,
        )?;
        let actions = [
            "memory.index.chroma.upsert",
            "memory.index.chroma.remove",
            "memory.index.chroma.search",
            "memory.index.chroma.status",
            "memory.index.chroma.reset",
        ];
        let mut policy = BuiltInPolicy::offline_default().with_network_destination(&self.origin);
        if allow {
            for action in actions {
                policy = policy.with_action(action, DecisionOutcome::Allow);
            }
        }
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = Arc::new(EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(actions.into_iter().map(str::to_owned)),
            [37_u8; 32],
        ));
        let executor = Arc::new(ChromaExecutor::new(profile.clone()));
        let embedding: Arc<dyn EmbeddingProvider> = Arc::new(LocalHashEmbeddingProvider::new(128)?);
        let index = ChromaMemoryIndex::open(
            gateway,
            executor,
            embedding,
            profile,
            self.directory.path().join("position.json"),
        )?;
        Ok((index, journal))
    }
}

fn fixture_response(request_line: &str) -> &'static str {
    if request_line.contains("/v1/embeddings HTTP/1.1") {
        r#"{"data":[{"embedding":[0.1,0.2,0.3],"index":0,"object":"embedding"}],"object":"list","model":"embed-test","usage":{"prompt_tokens":3,"total_tokens":3}}"#
    } else if request_line.contains("/query HTTP/1.1") {
        r#"{"ids":[["memory-1"]],"distances":[[0.25]]}"#
    } else if request_line.contains("/count HTTP/1.1") {
        "1"
    } else if request_line.ends_with("/collections HTTP/1.1")
        || request_line.contains("/collections/colossus-memory HTTP/1.1")
    {
        r#"{"id":"collection-id","name":"colossus-memory","tenant":"default_tenant","database":"default_database","dimension":128,"configuration_json":{"hnsw":{"space":"l2"}}}"#
    } else if request_line.contains("/delete HTTP/1.1") {
        r#"{"deleted":1}"#
    } else {
        "{}"
    }
}
