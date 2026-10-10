use super::*;
use colossus_contracts::*;
use colossus_ports::{AgentInbox, ChildCommunication, EventJournal, RunControl, SessionRepository};
use colossus_session::EventSourcedSessionRepository;
use std::sync::Arc;

struct Fixture {
    service: CommunicationService,
    journal: Arc<dyn EventJournal>,
    sessions: Arc<dyn SessionRepository>,
    owner: Actor,
    context: ExecutionContext,
    parent: String,
}

fn fixture(journal: Arc<dyn EventJournal>) -> Fixture {
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "test-application".into(),
    };
    let context = ExecutionContext {
        correlation_id: "communication-test".into(),
        session_id: Some(uuid::Uuid::now_v7().to_string()),
        run_id: Some(uuid::Uuid::now_v7().to_string()),
        ..ExecutionContext::default()
    };
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(journal.clone()));
    sessions
        .create_session(context.session_id.as_deref().unwrap(), None, owner.clone())
        .unwrap();
    let service = CommunicationService::new(journal.clone(), sessions.clone());
    service
        .begin_run(&context, &owner, RunControl::default())
        .unwrap();
    let parent = service
        .list_participants(&owner, context.run_id.as_deref().unwrap())
        .unwrap()[0]
        .id
        .clone();
    Fixture {
        service,
        journal,
        sessions,
        owner,
        context,
        parent,
    }
}

impl Fixture {
    fn child(&self, name: &str) -> AgentParticipant {
        let child_session_id = uuid::Uuid::now_v7().to_string();
        self.sessions
            .create_session(&child_session_id, None, self.owner.clone())
            .unwrap();
        let job = SubagentJob {
            id: format!("agent-{name}"),
            session_id: self.context.session_id.clone().unwrap(),
            parent_run_id: self.context.run_id.clone().unwrap(),
            parent_call_id: name.into(),
            task: "bounded task".into(),
            role: "subagent_default".into(),
            allowed_tools: Some(Vec::new()),
            status: SubagentStatus::Queued,
            child_session_id,
            child_run_id: None,
            final_output: String::new(),
            error: String::new(),
            created_at: "2026-10-09T00:00:00Z".into(),
            updated_at: "2026-10-09T00:00:00Z".into(),
            started_at: None,
            completed_at: None,
        };
        self.journal
            .append_batch(self.service.register_child(&job).unwrap())
            .unwrap();
        self.service
            .list_participants(&self.owner, self.context.run_id.as_deref().unwrap())
            .unwrap()
            .into_iter()
            .find(|participant| participant.subagent_id.as_ref() == Some(&job.id))
            .unwrap()
    }

    fn send(&self, target: &str, key: &str) -> SendAgentMessage {
        SendAgentMessage {
            recipient_id: target.into(),
            text: "use the updated requirements".into(),
            idempotency_key: key.into(),
            reply_to: None,
        }
    }

    fn start(&self, child: &AgentParticipant) -> ExecutionContext {
        let context = ExecutionContext {
            correlation_id: self.context.correlation_id.clone(),
            session_id: Some(child.session_id.clone()),
            run_id: Some(uuid::Uuid::now_v7().to_string()),
            subagent_id: child.subagent_id.clone(),
            ..ExecutionContext::default()
        };
        self.service
            .begin_run(&context, &self.owner, RunControl::default())
            .unwrap();
        context
    }
}

#[test]
fn queued_child_delivery_is_atomic_and_origin_does_not_begin_a_human_turn() {
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let child = f.child("first");
    let message = f
        .service
        .send_from_run(&f.context, f.send(&child.id, "input-1"))
        .unwrap();
    assert_eq!(message.receipt, AgentMessageReceipt::Accepted);
    let context = f.start(&child);
    let batch = f.service.prepare(&context).unwrap().unwrap();
    f.service
        .include(&batch, &context, 1, &"a".repeat(64))
        .unwrap();
    assert!(f.service.prepare(&context).unwrap().is_none());
    assert!(matches!(
        f.service
            .get_message(&f.owner, &message.id)
            .unwrap()
            .receipt,
        AgentMessageReceipt::IncludedInTurn { turn: 1, .. }
    ));
    let messages = f.sessions.list_messages(&child.session_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert!(!messages[0].message.begins_user_turn());
    assert_eq!(
        messages[0]
            .message
            .agent_message_origin
            .as_ref()
            .unwrap()
            .message_id,
        message.id
    );
    assert!(
        f.service
            .include(&batch, &context, 1, &"a".repeat(64))
            .is_err()
    );
    assert_eq!(
        f.sessions.list_messages(&child.session_id).unwrap().len(),
        1
    );
}

#[test]
fn concurrent_acceptance_invalidates_prepared_input_without_partial_session_writes() {
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let child = f.child("first");
    f.service
        .send_from_run(&f.context, f.send(&child.id, "first"))
        .unwrap();
    let context = f.start(&child);
    let batch = f.service.prepare(&context).unwrap().unwrap();
    f.service
        .send_from_run(&f.context, f.send(&child.id, "second"))
        .unwrap();
    assert!(
        f.service
            .include(&batch, &context, 1, &"b".repeat(64))
            .is_err()
    );
    assert!(
        f.sessions
            .list_messages(&child.session_id)
            .unwrap()
            .is_empty()
    );
    let fresh = f.service.prepare(&context).unwrap().unwrap();
    assert_eq!(fresh.messages.len(), 2);
    f.service
        .include(&fresh, &context, 1, &"b".repeat(64))
        .unwrap();
    assert_eq!(
        f.sessions.list_messages(&child.session_id).unwrap().len(),
        2
    );
}

#[test]
fn duplicate_keys_reconcile_after_closure_and_changed_input_conflicts() {
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let child = f.child("first");
    let request = f.send(&child.id, "stable");
    let message = f
        .service
        .send_from_run(&f.context, request.clone())
        .unwrap();
    assert_eq!(
        f.service
            .send_from_run(&f.context, request.clone())
            .unwrap()
            .id,
        message.id
    );
    let context = f.start(&child);
    f.service
        .close_run(
            context.run_id.as_deref().unwrap(),
            AgentMessageFailure::Cancelled,
        )
        .unwrap();
    let retry = f
        .service
        .send_from_run(&f.context, request.clone())
        .unwrap();
    assert_eq!(retry.id, message.id);
    assert_eq!(
        retry.receipt,
        AgentMessageReceipt::NotDelivered {
            reason: AgentMessageFailure::Cancelled
        }
    );
    let mut changed = request;
    changed.text = "different".into();
    assert!(matches!(
        f.service.send_from_run(&f.context, changed),
        Err(CommunicationError::IdempotencyConflict)
    ));
    assert!(matches!(
        f.service
            .send_from_run(&f.context, f.send(&child.id, "new")),
        Err(CommunicationError::Closed)
    ));
}

#[test]
fn owner_and_sibling_isolation_hold_even_with_valid_identifiers() {
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let one = f.child("one");
    let two = f.child("two");
    let context = f.start(&one);
    let visible = f.service.list_for_run(&context).unwrap();
    assert!(
        visible
            .iter()
            .all(|participant| participant.id == one.id || participant.id == f.parent)
    );
    assert!(matches!(
        f.service
            .send_from_run(&context, f.send(&two.id, "sibling")),
        Err(CommunicationError::Forbidden)
    ));
    f.service
        .send_from_run(&context, f.send(&f.parent, "to-parent"))
        .unwrap();
    let stranger = Actor {
        actor_type: ActorType::Application,
        id: "another-application".into(),
    };
    assert!(matches!(
        f.service
            .list_participants(&stranger, f.context.run_id.as_deref().unwrap()),
        Err(CommunicationError::Forbidden)
    ));
    assert!(matches!(
        f.service.list_messages(&stranger, &one.id, 0, 16),
        Err(CommunicationError::Forbidden)
    ));
    assert!(matches!(
        f.service
            .send_from_application(&stranger, f.send(&one.id, "unauthorized")),
        Err(CommunicationError::Forbidden)
    ));
}

#[test]
fn completion_defers_for_pending_input_and_explicit_termination_settles_receipts() {
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let message = f
        .service
        .send_from_application(&f.owner, f.send(&f.parent, "late"))
        .unwrap();
    assert!(!f.service.try_complete(&f.context).unwrap());
    f.service
        .close_run(
            f.context.run_id.as_deref().unwrap(),
            AgentMessageFailure::BudgetExhausted,
        )
        .unwrap();
    assert!(f.service.try_complete(&f.context).unwrap());
    assert_eq!(
        f.service
            .get_message(&f.owner, &message.id)
            .unwrap()
            .receipt,
        AgentMessageReceipt::NotDelivered {
            reason: AgentMessageFailure::BudgetExhausted
        }
    );
    assert_eq!(
        f.service
            .list_messages(&f.owner, &f.parent, 0, 16)
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn queue_limits_are_explicit_and_each_boundary_is_bounded() {
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let child = f.child("first");
    for n in 0..64 {
        f.service
            .send_from_run(&f.context, f.send(&child.id, &format!("message-{n}")))
            .unwrap();
    }
    assert!(matches!(
        f.service
            .send_from_run(&f.context, f.send(&child.id, "overflow")),
        Err(CommunicationError::Full)
    ));
    let context = f.start(&child);
    assert_eq!(
        f.service.prepare(&context).unwrap().unwrap().messages.len(),
        8
    );
    let page = f
        .service
        .list_messages(&f.owner, &child.id, 0, 1000)
        .unwrap();
    assert_eq!(page.messages.len(), 16);
    assert!(page.has_more);
}

#[test]
fn redb_reopen_recovers_started_attempt_and_keeps_canonical_receipts() {
    use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("messages.redb");
    let journal: Arc<dyn EventJournal> = Arc::new(
        RedbEventJournal::open(
            &path,
            Arc::new(PlaintextKeyProvider),
            Arc::new(DisabledCheckpointSigner),
        )
        .unwrap(),
    );
    let f = fixture(journal);
    let message = f
        .service
        .send_from_application(&f.owner, f.send(&f.parent, "before-crash"))
        .unwrap();
    let owner = f.owner.clone();
    let parent = f.parent.clone();
    drop(f);
    let reopened: Arc<dyn EventJournal> = Arc::new(
        RedbEventJournal::open(
            &path,
            Arc::new(PlaintextKeyProvider),
            Arc::new(DisabledCheckpointSigner),
        )
        .unwrap(),
    );
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(reopened.clone()));
    let service = CommunicationService::new(reopened, sessions);
    assert_eq!(service.recover().unwrap(), 1);
    assert_eq!(
        service.get_message(&owner, &message.id).unwrap().receipt,
        AgentMessageReceipt::NotDelivered {
            reason: AgentMessageFailure::Interrupted
        }
    );
    assert_eq!(
        service
            .list_messages(&owner, &parent, 0, 16)
            .unwrap()
            .messages[0]
            .id,
        message.id
    );
}

#[test]
fn work_cancellation_signals_the_running_child_and_requeue_allocates_a_fresh_address() {
    use colossus_work::{CreateSubagentRequest, EventSourcedWorkRepository, WorkService};
    let f = fixture(Arc::new(colossus_testkit::InMemoryEventJournal::default()));
    let communication = Arc::new(f.service);
    let repository = Arc::new(EventSourcedWorkRepository::new(f.journal.clone()));
    let work =
        WorkService::new(repository, f.sessions.clone()).with_communication(communication.clone());
    let job = work
        .create_subagent(
            CreateSubagentRequest {
                session_id: f.context.session_id.clone().unwrap(),
                parent_run_id: f.context.run_id.clone().unwrap(),
                parent_call_id: "delegate-call".into(),
                task: "inspect files".into(),
                role: "primary".into(),
                allowed_tools: Some(Vec::new()),
                instruction_snapshot_id: None,
            },
            f.owner.clone(),
        )
        .unwrap();
    let before = communication
        .list_participants(&f.owner, f.context.run_id.as_deref().unwrap())
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(before.generation, 1);
    work.start_subagent(&job.id, f.owner.clone()).unwrap();
    let child_context = ExecutionContext {
        correlation_id: "child".into(),
        run_id: Some("child-first-run".into()),
        session_id: Some(job.child_session_id.clone()),
        subagent_id: Some(job.id.clone()),
        ..Default::default()
    };
    let control = RunControl::default();
    communication
        .begin_run(&child_context, &f.owner, control.clone())
        .unwrap();
    let send = SendAgentMessage {
        recipient_id: before.id.clone(),
        text: "queued update".into(),
        idempotency_key: "pending-before-stop".into(),
        reply_to: None,
    };
    let message = communication
        .send_from_application(&f.owner, send.clone())
        .unwrap();
    work.stop_subagent(
        &job.id,
        SubagentStatus::Cancelled,
        "cancel requested",
        f.owner.clone(),
    )
    .unwrap();
    assert!(control.is_cancelled());
    assert_eq!(
        communication
            .get_message(&f.owner, &message.id)
            .unwrap()
            .receipt,
        AgentMessageReceipt::NotDelivered {
            reason: AgentMessageFailure::Cancelled
        }
    );
    // Requeue remains supported after the original parent has completed.
    communication
        .close_run(
            f.context.run_id.as_deref().unwrap(),
            AgentMessageFailure::Completed,
        )
        .unwrap();
    work.requeue_subagent(&job.id, f.owner.clone()).unwrap();
    let after = communication
        .list_participants(&f.owner, f.context.run_id.as_deref().unwrap())
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(after.generation, 2);
    assert_ne!(before.id, after.id);
    assert!(after.open);
    assert!(matches!(
        communication.send_from_application(
            &f.owner,
            SendAgentMessage {
                idempotency_key: "new-to-old-address".into(),
                ..send
            }
        ),
        Err(CommunicationError::Closed)
    ));
    assert_eq!(
        communication
            .list_messages(&f.owner, &after.id, 0, 16)
            .unwrap()
            .messages
            .len(),
        0
    );
}

#[test]
fn terminal_before_start_closes_a_queued_root_and_retains_verified_initial_peer_origin() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(journal.clone()));
    let service = CommunicationService::new(journal.clone(), sessions);
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "peer-owner".into(),
    };
    let context = ExecutionContext {
        run_id: Some("queued-root".into()),
        session_id: Some("peer-session".into()),
        ..Default::default()
    };
    journal
        .append_batch(
            service
                .stage_root_with_peer_input(&context, &owner, Some("peer-message-1"))
                .unwrap(),
        )
        .unwrap();
    let participant = service
        .list_participants(&owner, "queued-root")
        .unwrap()
        .pop()
        .unwrap();
    let origin = service.initial_origin(&context).unwrap().unwrap();
    assert_eq!(origin.recipient_id, participant.id);
    assert_eq!(
        origin.sender,
        AgentMessageSender::Application {
            application_id: owner.id.clone()
        }
    );
    let message = service
        .send_from_application(
            &owner,
            SendAgentMessage {
                recipient_id: participant.id.clone(),
                text: "follow-up before execution".into(),
                idempotency_key: "before-start".into(),
                reply_to: None,
            },
        )
        .unwrap();
    service
        .close_run("queued-root", AgentMessageFailure::Cancelled)
        .unwrap();
    assert_eq!(
        service.get_message(&owner, &message.id).unwrap().receipt,
        AgentMessageReceipt::NotDelivered {
            reason: AgentMessageFailure::Cancelled
        }
    );
    assert!(
        service
            .begin_run(&context, &owner, RunControl::default())
            .is_err()
    );
}
