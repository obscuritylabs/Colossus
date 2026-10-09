use super::*;
use colossus_contracts::{
    ProviderContinuation, ProviderContinuationPlan, ProviderContinuationView,
};
use colossus_ports::ProviderContinuationRepository;
use colossus_testkit::InMemoryEventJournal;

fn candidate() -> ProviderContinuation {
    let items = vec![json!({"type":"compaction","encrypted_content":"opaque-private-sentinel"})];
    let bytes = serde_json::to_vec(&items).unwrap().len();
    ProviderContinuation {
        view: ProviderContinuationView {
            id: "candidate".into(),
            covered_count: 2,
            context_binding_hash: "0".repeat(64),
            bytes,
            reserved_tokens: (bytes as u64).div_ceil(3) + 64,
        },
        plan: ProviderContinuationPlan {
            session_id: "session".into(),
            binding: "binding".into(),
            context_binding_hash: "0".repeat(64),
            source_count: 1,
            source_hash: "prefix".into(),
            snapshot_epoch: 0,
            selected: None,
        },
        settled_hash: String::new(),
        assistant: ModelMessage {
            role: ModelMessageRole::Assistant,
            content: "done".into(),
            tool_call_id: None,
            tool_calls: vec![],
        },
        hidden_reasoning: items,
    }
}

#[test]
fn responses_staging_is_never_durable_and_keyless_save_stays_in_memory() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let state = EventSourcedProviderContinuations::new(Arc::clone(&journal), false);
    state.stage(candidate()).unwrap();
    assert!(state.load("session").unwrap().is_none());
    let reopened = EventSourcedProviderContinuations::new(Arc::clone(&journal), false);
    assert!(reopened.take_staged("candidate").unwrap().is_none());
    let mut candidate = state.take_staged("candidate").unwrap().unwrap();
    assert!(state.take_staged("candidate").unwrap().is_none());
    assert!(
        state
            .save(candidate.clone(), &ExecutionContext::default())
            .is_err()
    );
    candidate.settled_hash = "settled".into();
    state.save(candidate, &ExecutionContext::default()).unwrap();
    assert!(state.load("session").unwrap().is_some());
    assert!(
        journal
            .read_stream("provider-continuation:session")
            .unwrap()
            .is_empty()
    );
    state
        .clear("session", &ExecutionContext::default())
        .unwrap();
    assert!(state.load("session").unwrap().is_none());
}

#[test]
fn responses_state_bounds_include_assistant_and_provenance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedProviderContinuations::new(journal, false);
    for large_assistant in [true, false] {
        let mut state = candidate();
        state.settled_hash = "settled".into();
        if large_assistant {
            state.assistant.content = "x".repeat(512 * 1024).into();
        } else {
            state.plan.binding = "x".repeat(512 * 1024);
        }
        assert!(state.view.bytes < 1024);
        assert!(repository.stage(state.clone()).is_err());
        assert!(
            repository
                .save(state, &ExecutionContext::default())
                .is_err()
        );
        assert!(repository.load("session").unwrap().is_none());
    }
}

#[test]
fn responses_staging_reserves_space_for_the_settled_hash() {
    let repository =
        EventSourcedProviderContinuations::new(Arc::new(InMemoryEventJournal::default()), false);
    let mut state = candidate();
    state.assistant.content = "".into();
    let overhead = serde_json::to_vec(&state).unwrap().len();
    state.assistant.content = "x".repeat(512 * 1024 - overhead - 64).into();
    repository.stage(state.clone()).unwrap();
    let mut oversized = state.clone();
    oversized.assistant.content = format!("{}x", state.assistant.content.plain_text()).into();
    assert!(repository.stage(oversized).is_err());
    state.settled_hash = "0".repeat(64);
    repository
        .save(state, &ExecutionContext::default())
        .unwrap();
    assert!(repository.load("session").unwrap().is_some());
}

#[test]
fn responses_protected_repository_reopens_and_retirement_prevents_stale_reuse() {
    use colossus_journal_redb::{Ed25519CheckpointSigner, RedbEventJournal, StaticKeyProvider};
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("state.redb");
    let open = || -> Arc<dyn EventJournal> {
        Arc::new(
            RedbEventJournal::open(
                &path,
                Arc::new(StaticKeyProvider::new("test", [7; 32])),
                Arc::new(Ed25519CheckpointSigner::new("test", [8; 32])),
            )
            .unwrap(),
        )
    };
    {
        let journal = open();
        let repository = EventSourcedProviderContinuations::new(Arc::clone(&journal), true);
        let mut state = candidate();
        state.settled_hash = "settled".into();
        repository
            .save(state, &ExecutionContext::default())
            .unwrap();
        let event = journal
            .read_stream("provider-continuation:session")
            .unwrap()
            .pop()
            .unwrap();
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains("opaque-private-sentinel")
        );
        assert!(
            !format!("{:?}", repository.load("session").unwrap())
                .contains("opaque-private-sentinel")
        );
    }
    let repository = EventSourcedProviderContinuations::new(open(), true);
    assert_eq!(
        repository
            .load("session")
            .unwrap()
            .unwrap()
            .hidden_reasoning[0]["encrypted_content"],
        "opaque-private-sentinel"
    );
    repository
        .clear("session", &ExecutionContext::default())
        .unwrap();
    assert!(repository.load("session").unwrap().is_none());
    let mut invalid = candidate();
    invalid.view.bytes += 1;
    assert!(repository.stage(invalid).is_err());
}
