use super::*;
use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext};
use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
use std::sync::atomic::{AtomicBool, Ordering};

struct Fence(AtomicBool);
impl RuntimeExecutionFence for Fence {
    fn verify(&self) -> Result<(), StoreError> {
        if self.0.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(StoreError::WriterLeaseHeld)
        }
    }
}

#[test]
fn expired_owner_cannot_read_write_checkpoint_or_recover_through_runtime_storage() {
    let journal = Arc::new(
        RedbEventJournal::open_in_memory(
            Arc::new(PlaintextKeyProvider),
            Arc::new(DisabledCheckpointSigner),
        )
        .unwrap(),
    );
    let fence = Arc::new(Fence(AtomicBool::new(true)));
    let guarded = FencedJournal::new(journal.clone(), fence.clone());
    let event = || NewEvent {
        event_version: 1,
        stream_id: "effect:already-started".into(),
        expected_stream_version: 0,
        classification: EventClassification::Effect,
        event_type: "effect.started.v1".into(),
        actor: Actor {
            actor_type: ActorType::System,
            id: "host".into(),
        },
        context: ExecutionContext::default(),
        payload: serde_json::json!({}),
    };
    guarded.append(event()).unwrap();
    fence.0.store(false, Ordering::SeqCst);
    assert!(guarded.is_recovery_mode());
    assert!(matches!(guarded.head(), Err(StoreError::WriterLeaseHeld)));
    assert!(matches!(
        guarded.append(event()),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert!(matches!(
        guarded.checkpoint(),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert_eq!(
        journal.head().unwrap().0,
        1,
        "loss of ownership preserves the started effect without a fabricated terminal outcome"
    );
    let projections = FencedProjections::new(journal, fence);
    assert!(matches!(
        projections.reset("sessions"),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert!(matches!(
        projections.apply(ProjectionBatch {
            projection: "sessions".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: vec![]
        }),
        Err(StoreError::WriterLeaseHeld)
    ));
}
