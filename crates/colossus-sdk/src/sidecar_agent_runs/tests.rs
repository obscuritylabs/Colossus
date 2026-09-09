use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::{
    ApiError, ApiErrorReason, ApprovalInteraction, CancelRunRequest, CancelRunResponse,
    CommandApprovalContext, CreateRunRequest, CreateRunResponse, IdempotencyKey, ListRunsRequest,
    ListRunsResponse, PromptAnswer, Run, RunMode, RunStatus, RunUpdate, UserPromptInteraction,
};

fn approval() -> Interaction {
    Interaction {
        interaction_id: "approval".into(),
        run_id: "run".into(),
        kind: InteractionKind::Approval,
        status: InteractionStatus::Pending,
        created_at: "2026-09-09T00:00:00Z".into(),
        expires_at: "2999-01-01T00:00:00Z".into(),
        respondable_by_caller: false,
        etag: "opaque-etag".into(),
        content: InteractionContent::Approval(ApprovalInteraction {
            command_context: Some(CommandApprovalContext {
                justification: "Check the build.".into(),
                executable: "prepared-executable".into(),
                arguments: vec!["one argument".into()],
                working_directory: "workspace".into(),
                redacted: false,
            }),
            reason: "An effect requires approval".into(),
            action: "process.execute".into(),
            resource: "configured executable".into(),
            risk: None,
            request_hash: "public-binding".into(),
        }),
    }
}

fn interaction_cases() -> Vec<Interaction> {
    let pending = approval();
    let mut cases = vec![pending.clone()];
    for status in [
        InteractionStatus::Answered,
        InteractionStatus::Expired,
        InteractionStatus::Cancelled,
    ] {
        cases.push(Interaction {
            status,
            ..pending.clone()
        });
    }
    cases.push(Interaction {
        etag: String::new(),
        ..pending.clone()
    });
    cases.push(Interaction {
        kind: InteractionKind::UserPrompt,
        ..pending.clone()
    });
    let prompt_content = InteractionContent::UserPrompt(UserPromptInteraction {
        question: "Which build?".into(),
        choices: vec![],
        allow_free_form: true,
    });
    // Neither a mismatched content/kind pair nor an ordinary prompt acquires scope.
    cases.push(Interaction {
        content: prompt_content.clone(),
        ..pending.clone()
    });
    cases.push(Interaction {
        kind: InteractionKind::UserPrompt,
        content: prompt_content,
        ..pending
    });
    cases
}

struct Client {
    label: &'static str,
    reads: Mutex<usize>,
    responses: Mutex<Vec<RespondInteractionRequest>>,
}

impl Client {
    fn new(label: &'static str) -> Arc<Self> {
        Arc::new(Self {
            label,
            reads: Mutex::new(0),
            responses: Mutex::new(vec![]),
        })
    }
}

#[async_trait]
impl AgentRunClient for Client {
    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("not exercised")
    }

    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        assert_eq!(request.run_id, "run");
        *self.reads.lock().unwrap() += 1;
        Ok(GetRunResponse {
            run: Run {
                plugin_skill_ids: vec![],
                run_id: request.run_id,
                session_id: "session".into(),
                title: "Approval".into(),
                role: "primary".into(),
                mode: RunMode::Execute,
                status: RunStatus::Waiting,
                created_at: "2026-09-09T00:00:00Z".into(),
                updated_at: "2026-09-09T00:00:00Z".into(),
                started_at: None,
                finished_at: None,
                last_sequence: 1,
                pending_interaction_count: 1,
                terminal: None,
                etag: "run-etag".into(),
                archived: false,
            },
            pending_interactions: interaction_cases(),
        })
    }

    async fn list_runs(&self, _: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!("not exercised")
    }

    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        assert_eq!(request.run_id, "run");
        assert_eq!(request.after_sequence, 7);
        *self.reads.lock().unwrap() += 1;
        let updates = interaction_cases()
            .into_iter()
            .enumerate()
            .map(|(index, interaction)| {
                Ok(RunUpdate {
                    run_id: "run".into(),
                    sequence: 8 + index as u64,
                    created_at: "2026-09-09T00:00:00Z".into(),
                    update: RunUpdateKind::Interaction(interaction),
                })
            });
        Ok(Box::pin(futures::stream::iter(updates.chain([Err(
            ApiError::permission_denied(ApiErrorReason::ScopeDenied, "watch denied"),
        )]))))
    }

    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("not exercised")
    }

    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        self.responses.lock().unwrap().push(request);
        Err(ApiError::permission_denied(
            ApiErrorReason::ScopeDenied,
            self.label,
        ))
    }
}

#[tokio::test]
async fn get_and_watch_project_only_pending_approvals_with_a_bootstrapped_broker() {
    for with_broker in [false, true] {
        let primary = Client::new("primary");
        let broker = Client::new("broker");
        let transports = AgentRunTransports {
            primary: primary.clone(),
            approval_broker: with_broker.then(|| broker.clone() as Arc<dyn AgentRunClient>),
        };
        let mut expected = interaction_cases();
        expected[0].respondable_by_caller = with_broker;
        let details = transports
            .get_run(GetRunRequest {
                run_id: "run".into(),
            })
            .await
            .unwrap();
        // Context, public binding, etag, scope on prompts, and closed states stay exact.
        assert_eq!(details.pending_interactions, expected);
        assert_eq!(details.run.etag, "run-etag");
        let mut stream = transports
            .watch_run(WatchRunRequest {
                run_id: "run".into(),
                after_sequence: 7,
            })
            .await
            .unwrap();
        for (index, interaction) in expected.into_iter().enumerate() {
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                RunUpdate {
                    run_id: "run".into(),
                    sequence: 8 + index as u64,
                    created_at: "2026-09-09T00:00:00Z".into(),
                    update: RunUpdateKind::Interaction(interaction),
                }
            );
        }
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.reason, ApiErrorReason::ScopeDenied);
        assert_eq!(error.message, "watch denied");
        assert!(stream.next().await.is_none());
        assert_eq!(*primary.reads.lock().unwrap(), 2);
        assert_eq!(
            *broker.reads.lock().unwrap(),
            0,
            "reads never use approval authority"
        );
        assert!(primary.responses.lock().unwrap().is_empty());
        assert!(broker.responses.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn only_approval_answers_use_broker_and_errors_do_not_retry_or_fallback() {
    for with_broker in [false, true] {
        for answer in [
            InteractionAnswer::Approval {
                approved: true,
                request_hash: "public-binding".into(),
            },
            InteractionAnswer::Approval {
                approved: false,
                request_hash: "public-binding".into(),
            },
            InteractionAnswer::Prompt(PromptAnswer::FreeForm("build-debug".into())),
        ] {
            let primary = Client::new("primary");
            let broker = Client::new("broker");
            let transports = AgentRunTransports {
                primary: primary.clone(),
                approval_broker: with_broker.then(|| broker.clone() as Arc<dyn AgentRunClient>),
            };
            let uses_broker = with_broker && matches!(answer, InteractionAnswer::Approval { .. });
            let request = RespondInteractionRequest {
                run_id: "run".into(),
                interaction_id: "interaction".into(),
                etag: "opaque-etag".into(),
                idempotency_key: IdempotencyKey::new("response-key").unwrap(),
                response: answer,
            };
            let error = transports
                .respond_interaction(request.clone())
                .await
                .unwrap_err();
            let (selected, unused) = if uses_broker {
                (&broker, &primary)
            } else {
                (&primary, &broker)
            };
            assert_eq!(error.reason, ApiErrorReason::ScopeDenied);
            assert_eq!(error.message, selected.label);
            assert_eq!(*selected.responses.lock().unwrap(), vec![request]);
            assert!(unused.responses.lock().unwrap().is_empty());
        }
    }
}
