use super::*;
use colossus_contracts::{
    ProviderContinuation, ProviderContinuationPlan, ProviderContinuationView,
};
use colossus_ports::ProviderContinuationRepository;
use colossus_testkit::InMemoryEventJournal;

fn anthropic_candidate() -> ProviderContinuation {
    let mut state = candidate();
    state.view.replay = true;
    state
        .assistant
        .tool_calls
        .push(colossus_contracts::ModelToolCall {
            call_id: "call-1".into(),
            name: "test.lookup".into(),
            arguments: json!({}),
        });
    state.hidden_reasoning = vec![
        json!({"type":"anthropic_replay"}),
        json!({
            "message":state.assistant,
            "content":[{"type":"thinking","thinking":"private-thought","signature":"private-signature"},
                {"type":"tool_use","id":"call-1","name":"test_lookup","input":{}}]
        }),
    ];
    state.view.bytes = serde_json::to_vec(&state.hidden_reasoning).unwrap().len();
    state.view.reserved_tokens = (state.view.bytes as u64).div_ceil(3) + 64;
    state
}

#[test]
fn anthropic_replay_is_staged_privately_and_keyless_restart_loses_it() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedProviderContinuations::new(journal.clone(), false);
    repository.stage(anthropic_candidate()).unwrap();
    assert!(repository.load("session").unwrap().is_none());
    let mut state = repository.take_staged("candidate").unwrap().unwrap();
    state.settled_hash = "settled".into();
    repository
        .save(state, &ExecutionContext::default())
        .unwrap();
    assert!(repository.load("session").unwrap().unwrap().view.replay);
    assert!(
        journal
            .read_stream("provider-continuation:session")
            .unwrap()
            .is_empty()
    );
    assert!(
        EventSourcedProviderContinuations::new(journal, false)
            .load("session")
            .unwrap()
            .is_none()
    );
}

#[test]
fn anthropic_replay_cannot_be_misclassified_as_responses_compaction() {
    let repository =
        EventSourcedProviderContinuations::new(Arc::new(InMemoryEventJournal::default()), false);
    let mut state = anthropic_candidate();
    state.view.replay = false;
    assert!(repository.stage(state).is_err());
    let mut legacy = serde_json::to_value(candidate()).unwrap();
    legacy["view"].as_object_mut().unwrap().remove("replay");
    let state: ProviderContinuation = serde_json::from_value(legacy).unwrap();
    assert!(!state.view.replay);
    repository.stage(state).unwrap();
}

#[test]
fn anthropic_protected_replay_reopens_without_exposing_signed_blocks() {
    use colossus_journal_redb::{Ed25519CheckpointSigner, RedbEventJournal, StaticKeyProvider};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("anthropic.redb");
    let open = || -> Arc<dyn EventJournal> {
        Arc::new(
            RedbEventJournal::open(
                &path,
                Arc::new(StaticKeyProvider::new("test", [31; 32])),
                Arc::new(Ed25519CheckpointSigner::new("test", [32; 32])),
            )
            .unwrap(),
        )
    };
    {
        let journal = open();
        let repository = EventSourcedProviderContinuations::new(journal.clone(), true);
        let mut state = anthropic_candidate();
        state.settled_hash = "settled".into();
        repository
            .save(state, &ExecutionContext::default())
            .unwrap();
        let events = journal
            .read_stream("provider-continuation:session")
            .unwrap();
        let encoded = serde_json::to_string(&events).unwrap();
        assert!(!encoded.contains("private-signature"));
        assert!(!encoded.contains("private-thought"));
        assert!(
            !format!("{:?}", repository.load("session").unwrap()).contains("private-signature")
        );
    }
    let repository = EventSourcedProviderContinuations::new(open(), true);
    let state = repository.load("session").unwrap().unwrap();
    assert!(state.view.replay);
    assert_eq!(
        state.hidden_reasoning[1]["content"][0]["signature"],
        "private-signature"
    );
    repository
        .clear("session", &ExecutionContext::default())
        .unwrap();
    assert!(repository.load("session").unwrap().is_none());
}

fn candidate() -> ProviderContinuation {
    let items = vec![json!({"type":"compaction","encrypted_content":"opaque-private-sentinel"})];
    let bytes = serde_json::to_vec(&items).unwrap().len();
    ProviderContinuation {
        view: ProviderContinuationView {
            replay: false,
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
            agent_message_origin: None,
            role: ModelMessageRole::Assistant,
            content: "done".into(),
            tool_call_id: None,
            tool_calls: vec![],
        },
        hidden_reasoning: items,
        replay_request: None,
        route_binding: String::new(),
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

#[test]
fn released_replay_admission_and_settlement_share_the_session_transactions() {
    use colossus_contracts::{ModelToolCall, PendingSessionToolTurn, SessionMessageAppend};
    use colossus_ports::SessionRepository;
    use colossus_session::EventSourcedSessionRepository;

    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = EventSourcedSessionRepository::new(journal.clone());
    let actor = Actor {
        actor_type: ActorType::System,
        id: "test".into(),
    };
    sessions
        .create_session("session", None, actor.clone())
        .unwrap();
    let repository = EventSourcedProviderContinuations::new(journal.clone(), true);
    let mut state = anthropic_candidate();
    let context = ExecutionContext {
        session_id: Some("session".into()),
        run_id: Some("run".into()),
        ..Default::default()
    };
    let pending = PendingSessionToolTurn {
        run_id: "run".into(),
        turn: 1,
        call_ids: vec!["call-1".into()],
    };
    repository.stage(state.clone()).unwrap();
    let admission = repository
        .admission_events("session", "candidate", &context)
        .unwrap();
    assert!(
        EventSourcedProviderContinuations::new(journal.clone(), true)
            .candidate("session", "candidate")
            .unwrap()
            .is_none()
    );
    let mut conflicting_admission = admission.clone();
    conflicting_admission[0].expected_stream_version += 1;
    assert!(
        sessions
            .begin_tool_turn_with_events(
                "session",
                pending.clone(),
                actor.clone(),
                conflicting_admission
            )
            .is_err()
    );
    assert!(
        sessions.pending_tool_turn("session").unwrap().is_none(),
        "failed private admission must not leave a marker"
    );
    sessions
        .begin_tool_turn_with_events("session", pending.clone(), actor.clone(), admission)
        .unwrap();
    let reopened = EventSourcedProviderContinuations::new(journal.clone(), true);
    assert!(
        reopened
            .candidate("session", "candidate")
            .unwrap()
            .is_some()
    );
    assert!(
        reopened.load("session").unwrap().is_none(),
        "admitted state is not yet reusable"
    );
    state.settled_hash = "0".repeat(64);
    let settlement = reopened.settlement_events(&state, &context).unwrap();
    let assistant = ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::Assistant,
        content: "".into(),
        tool_call_id: None,
        tool_calls: vec![ModelToolCall {
            call_id: "call-1".into(),
            name: "test.lookup".into(),
            arguments: json!({}),
        }],
    };
    let result = ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::Tool,
        content: "done".into(),
        tool_call_id: Some("call-1".into()),
        tool_calls: vec![],
    };
    let appends = vec![
        SessionMessageAppend {
            message: assistant,
            actor: actor.clone(),
        },
        SessionMessageAppend {
            message: result,
            actor: actor.clone(),
        },
    ];
    let mut conflicting_settlement = settlement.clone();
    conflicting_settlement[0].expected_stream_version += 1;
    assert!(
        sessions
            .complete_tool_turn_with_events(
                "session",
                &pending,
                appends.clone(),
                actor.clone(),
                conflicting_settlement
            )
            .is_err()
    );
    assert!(
        sessions.list_messages("session").unwrap().is_empty(),
        "failed private settlement must not expose an incomplete transcript"
    );
    assert_eq!(
        sessions.pending_tool_turn("session").unwrap(),
        Some(pending.clone())
    );
    sessions
        .complete_tool_turn_with_events("session", &pending, appends, actor, settlement)
        .unwrap();
    assert!(sessions.pending_tool_turn("session").unwrap().is_none());
    assert_eq!(sessions.list_messages("session").unwrap().len(), 2);
    assert!(reopened.load("session").unwrap().is_some());
    assert!(
        reopened
            .candidate("session", "candidate")
            .unwrap()
            .is_none()
    );
}

#[test]
fn keyless_admission_never_writes_private_state_and_legacy_retirement_is_explicit() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let keyless = EventSourcedProviderContinuations::new(journal.clone(), false);
    let mut state = anthropic_candidate();
    keyless.stage(state.clone()).unwrap();
    assert!(
        keyless
            .admission_events("session", "candidate", &ExecutionContext::default())
            .unwrap()
            .is_empty()
    );
    state.settled_hash = "0".repeat(64);
    assert!(
        keyless
            .settlement_events(&state, &ExecutionContext::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(journal.head().unwrap().0, 0);
    let protected = EventSourcedProviderContinuations::new(journal, true);
    protected.save(state, &ExecutionContext::default()).unwrap();
    protected
        .clear("session", &ExecutionContext::default())
        .unwrap();
    assert!(protected.load("session").unwrap().is_none());
    assert!(protected.retired_replay("session").unwrap().is_some());
}

#[test]
fn replay_request_has_its_own_existing_request_budget() {
    let repository =
        EventSourcedProviderContinuations::new(Arc::new(InMemoryEventJournal::default()), false);
    let mut state = anthropic_candidate();
    state.replay_request = Some(ModelRequest {
        instructions: "x".repeat(700 * 1024),
        messages: Vec::new(),
        tools: Vec::new(),
        max_output_tokens: None,
    });
    repository.stage(state.clone()).unwrap();
    state.replay_request.as_mut().unwrap().instructions = "x".repeat(896 * 1024);
    assert!(repository.stage(state).is_err());
}
