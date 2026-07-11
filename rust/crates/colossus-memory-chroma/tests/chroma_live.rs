//! Opt-in compatibility acceptance against a real Chroma v2 endpoint.

use colossus_contracts::DecisionOutcome;
use colossus_memory_chroma::{
    ChromaExecutor, ChromaMemoryIndex, ChromaProfile, LocalHashEmbeddingProvider,
};
use colossus_policy::{BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel};
use colossus_ports::{EmbeddingProvider, EventJournal, MemoryIndex};
use colossus_testkit::InMemoryEventJournal;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

#[tokio::test]
#[ignore = "requires COLOSSUS_CHROMA_URL pointing to a live Chroma v2 endpoint"]
async fn live_chroma_v2_supports_create_upsert_query_count_delete_and_reset() {
    let base_url = std::env::var("COLOSSUS_CHROMA_URL")
        .expect("COLOSSUS_CHROMA_URL must name a live Chroma v2 endpoint");
    let tenant =
        std::env::var("COLOSSUS_CHROMA_TENANT").unwrap_or_else(|_| "default_tenant".into());
    let database =
        std::env::var("COLOSSUS_CHROMA_DATABASE").unwrap_or_else(|_| "default_database".into());
    let credential_reference = std::env::var("COLOSSUS_CHROMA_CREDENTIAL_ENV")
        .ok()
        .map(|variable| format!("env:{variable}"));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let collection = format!("colossus-live-{}-{nonce}", std::process::id());
    let profile = ChromaProfile::new(
        &base_url,
        tenant,
        database,
        collection,
        credential_reference,
        15_000,
    )
    .expect("live profile");
    let origin = Url::parse(&base_url)
        .expect("Chroma URL")
        .origin()
        .ascii_serialization();
    let actions = [
        "memory.index.chroma.upsert",
        "memory.index.chroma.remove",
        "memory.index.chroma.search",
        "memory.index.chroma.status",
        "memory.index.chroma.reset",
    ];
    let mut policy = BuiltInPolicy::offline_default().with_network_destination(origin);
    for action in actions {
        policy = policy.with_action(action, DecisionOutcome::Allow);
    }
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = Arc::new(EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(actions.into_iter().map(str::to_owned)),
        [73_u8; 32],
    ));
    let executor = Arc::new(ChromaExecutor::new(profile.clone()));
    let embedding: Arc<dyn EmbeddingProvider> =
        Arc::new(LocalHashEmbeddingProvider::new(128).expect("embedding profile"));
    let directory = tempfile::tempdir().expect("position directory");
    let index = ChromaMemoryIndex::open(
        gateway,
        executor,
        embedding,
        profile,
        directory.path().join("position.json"),
    )
    .expect("live index");

    index
        .upsert(
            "live-event-1",
            "live-memory-1",
            "Colossus live Chroma compatibility",
            &serde_json::json!({"scope": "global", "suite": "live-v2"}),
            None,
        )
        .await
        .expect("live upsert");
    let candidates = index
        .search("live Chroma compatibility", 4)
        .await
        .expect("live query");
    assert!(
        candidates
            .iter()
            .any(|(id, score)| id == "live-memory-1" && score.is_finite())
    );
    let status = index.status().await.expect("live count");
    assert_eq!(status["ready"], true);
    assert!(status["documents"].as_u64().is_some_and(|count| count >= 1));
    index
        .remove("live-event-2", "live-memory-1")
        .await
        .expect("live delete");
    index.rebuild(&[]).await.expect("live reset cleanup");

    let events = journal.read_global(1, 1_000).expect("audit events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.completed.v1")
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "effect.denied.v1")
    );
}
