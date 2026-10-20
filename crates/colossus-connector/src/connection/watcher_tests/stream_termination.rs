use super::*;
use async_trait::async_trait;
use colossus_sdk::{
    ApiResult, CancelRunResponse, CreateRunRequest, CreateRunResponse, GetRunResponse,
    ListRunsResponse, RespondInteractionRequest, RespondInteractionResponse, Run, RunMode,
    RunResult, RunStatus, RunTerminal, RunUpdate, RunUpdateKind, RunUpdateStream,
};

struct ClosedFeed {
    terminal: bool,
    updates: Vec<RunUpdate>,
}

fn result() -> RunResult {
    RunResult {
        output: "complete".into(),
        plan_id: None,
        plan_revision: None,
        plan_status: None,
        goal_id: None,
        profile: "test".into(),
        model_profile: "test".into(),
        provider_profile: "test".into(),
        model: "test".into(),
        elapsed_seconds: 1.0,
    }
}

#[async_trait]
impl AgentRunClient for ClosedFeed {
    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!()
    }
    async fn get_run(&self, _: GetRunRequest) -> ApiResult<GetRunResponse> {
        Ok(GetRunResponse {
            run: Run {
                plugin_skill_ids: vec![],
                run_id: "run".into(),
                session_id: "session".into(),
                title: "Test".into(),
                role: "primary".into(),
                mode: RunMode::Execute,
                status: if self.terminal {
                    RunStatus::Completed
                } else {
                    RunStatus::Queued
                },
                created_at: "2026-10-20T00:00:00Z".into(),
                updated_at: "2026-10-20T00:00:00Z".into(),
                started_at: None,
                finished_at: None,
                last_sequence: 2,
                pending_interaction_count: 0,
                etag: "etag".into(),
                archived: false,
                terminal: self.terminal.then(|| RunTerminal::Result(result())),
            },
            pending_interactions: vec![],
        })
    }
    async fn list_runs(&self, _: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!()
    }
    async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        Ok(Box::pin(futures::stream::iter(
            self.updates.clone().into_iter().map(Ok),
        )))
    }
    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!()
    }
    async fn respond_interaction(
        &self,
        _: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!()
    }
}

async fn run(terminal: bool, updates: Vec<RunUpdate>, after: u64) -> Result<(), Status> {
    let (sender, _receiver) = crate::outbound::channel();
    watch_task(
        Arc::new(ClosedFeed { terminal, updates }),
        sender,
        "task".into(),
        "run".into(),
        after,
        false,
    )
    .await
}

#[tokio::test]
async fn nonterminal_eof_requires_connection_reconciliation() {
    let error = run(false, vec![], 0).await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn terminal_snapshot_cannot_substitute_for_unreleased_updates() {
    assert_eq!(
        run(true, vec![], 0).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    let partial = RunUpdate {
        run_id: "run".into(),
        sequence: 1,
        created_at: "2026-10-20T00:00:00Z".into(),
        update: RunUpdateKind::State(RunStatus::Running),
    };
    assert_eq!(
        run(true, vec![partial], 0).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
}

#[tokio::test]
async fn terminal_eof_completes_only_after_released_cursor_catches_up() {
    run(true, vec![], 2).await.unwrap();
    let updates = vec![
        RunUpdate {
            run_id: "run".into(),
            sequence: 1,
            created_at: "2026-10-20T00:00:00Z".into(),
            update: RunUpdateKind::State(RunStatus::Running),
        },
        RunUpdate {
            run_id: "run".into(),
            sequence: 2,
            created_at: "2026-10-20T00:00:00Z".into(),
            update: RunUpdateKind::Result(result()),
        },
    ];
    run(true, updates, 0).await.unwrap();
}
