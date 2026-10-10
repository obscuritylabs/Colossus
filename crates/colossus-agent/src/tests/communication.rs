use super::*;
use colossus_communication::CommunicationService;
use colossus_ports::AgentInbox;
use sha2::{Digest, Sha256};

struct AcceptDuringTurn {
    provider: ScriptedProvider,
    inbox: Arc<CommunicationService>,
    owner: Actor,
    sent: Mutex<Option<colossus_contracts::AgentMessage>>,
}
#[async_trait]
impl ModelProvider for AcceptDuringTurn {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        self.provider.route(role)
    }
    async fn turn(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        if self.sent.lock().unwrap().is_none() {
            let recipient = self
                .inbox
                .list_participants(&self.owner, context.run_id.as_deref().unwrap())
                .unwrap()[0]
                .id
                .clone();
            let message = self
                .inbox
                .send_from_application(
                    &self.owner,
                    colossus_contracts::SendAgentMessage {
                        recipient_id: recipient,
                        text: "Updated peer instructions".into(),
                        idempotency_key: "during-provider".into(),
                        reply_to: None,
                    },
                )
                .unwrap();
            *self.sent.lock().unwrap() = Some(message);
        }
        self.provider.turn(role, request, context).await
    }
}

async fn run_late_input(budget: u16) {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(journal.clone()));
    let inbox = Arc::new(CommunicationService::new(journal.clone(), sessions.clone()));
    let allocation_journal = journal.clone();
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "test-peer".into(),
    };
    let provider = Arc::new(AcceptDuringTurn {
        provider: ScriptedProvider::new(vec![
            turn(vec![ProviderEvent::FinalOutput {
                text: "initial result".into(),
            }]),
            turn(vec![ProviderEvent::FinalOutput {
                text: "updated result".into(),
            }]),
        ]),
        inbox: inbox.clone(),
        owner: owner.clone(),
        sent: Mutex::default(),
    });
    let service = AgentService::new(
        journal,
        provider.clone(),
        Arc::new(StaticToolRegistry::builtins(&[]).unwrap()),
        Arc::new(EchoTools),
        sessions.clone(),
    )
    .with_inbox(inbox.clone());
    let context = ExecutionContext {
        run_id: Some("peer-run".into()),
        session_id: Some("peer-session".into()),
        ..Default::default()
    };
    sessions
        .create_session("peer-session", None, owner.clone())
        .unwrap();
    allocation_journal
        .append_batch(
            inbox
                .stage_root_with_peer_input(&context, &owner, Some("original-peer-input"))
                .unwrap(),
        )
        .unwrap();
    let mut observer = RecordingRunObserver::default();
    let outcome = service
        .run_public_with_mode_and_skills_stream_controlled(
            "primary",
            "base instructions",
            "Initial peer task",
            budget,
            "peer-run",
            "peer-session",
            false,
            &[],
            &[],
            AgentRunMode::Execute,
            None,
            None,
            owner.clone(),
            &mut observer,
            &RunControl::default(),
        )
        .await
        .unwrap();
    let AgentRunOutcome::Completed { result } = outcome else {
        panic!("unexpected cancellation")
    };
    let requests = provider.provider.requests.lock().unwrap();
    assert_eq!(requests.len(), budget as usize);
    assert!(requests[0].messages.iter().any(|message| {
        message
            .agent_message_origin
            .as_ref()
            .is_some_and(|origin| origin.message_id == "original-peer-input")
            && !message.begins_user_turn()
    }));
    let sent = provider.sent.lock().unwrap().clone().unwrap();
    let receipt = inbox.get_message(&owner, &sent.id).unwrap().receipt;
    if budget == 2 {
        assert_eq!(result.output, "updated result");
        assert!(requests[1].messages.iter().any(|message| {
            message
                .agent_message_origin
                .as_ref()
                .is_some_and(|origin| origin.message_id == sent.id)
        }));
        let expected_hash = hex::encode(Sha256::digest(serde_json::to_vec(&requests[1]).unwrap()));
        assert_eq!(
            receipt,
            colossus_contracts::AgentMessageReceipt::IncludedInTurn {
                run_id: "peer-run".into(),
                turn: 2,
                request_hash: expected_hash
            }
        );
        assert_eq!(
            sessions
                .list_messages("peer-session")
                .unwrap()
                .iter()
                .filter(|record| record
                    .message
                    .agent_message_origin
                    .as_ref()
                    .is_some_and(|origin| origin.message_id == sent.id))
                .count(),
            1
        );
    } else {
        assert_eq!(
            receipt,
            colossus_contracts::AgentMessageReceipt::NotDelivered {
                reason: colossus_contracts::AgentMessageFailure::BudgetExhausted
            }
        );
    }
    assert!(!inbox.list_participants(&owner, "peer-run").unwrap()[0].open);
    assert!(inbox.prepare(&context).unwrap().is_none());
}

#[tokio::test]
async fn accepted_input_at_completion_uses_the_next_turn_and_exact_prepared_request_hash() {
    run_late_input(2).await;
}
#[tokio::test]
async fn peer_input_does_not_extend_an_exhausted_turn_budget() {
    run_late_input(1).await;
}
