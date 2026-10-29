use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct PredictionRuns {
    calls: Arc<AtomicUsize>,
    text: &'static str,
}

#[async_trait]
impl AgentRunClient for PredictionRuns {
    async fn predict_next_prompt(
        &self,
        request: crate::PredictNextPromptRequest,
    ) -> ApiResult<crate::NextPromptPrediction> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(crate::NextPromptPrediction {
            source_run_id: request.source_run_id,
            text: Some(self.text.into()),
            usage: None,
        })
    }
    async fn create_run(&self, _request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!()
    }
    async fn get_run(&self, _request: GetRunRequest) -> ApiResult<GetRunResponse> {
        unreachable!()
    }
    async fn list_runs(&self, _request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!()
    }
    async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        unreachable!()
    }
    async fn cancel_run(&self, _request: CancelRunRequest) -> ApiResult<crate::CancelRunResponse> {
        unreachable!()
    }
    async fn respond_interaction(
        &self,
        _request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!()
    }
}

#[tokio::test]
async fn windows_prediction_routes_to_primary_application_not_approval_broker() {
    let primary = Arc::new(AtomicUsize::new(0));
    let broker = Arc::new(AtomicUsize::new(0));
    let runs = WindowsAgentRuns {
        transports: AgentRunTransports {
            primary: Arc::new(PredictionRuns {
                calls: primary.clone(),
                text: "Review this source",
            }),
            approval_broker: Some(Arc::new(PredictionRuns {
                calls: broker.clone(),
                text: "wrong owner",
            })),
        },
        closed: watch::channel(false).0,
    };
    let prediction = runs
        .predict_next_prompt(crate::PredictNextPromptRequest {
            source_run_id: "completed-source".into(),
        })
        .await
        .unwrap();
    assert_eq!(prediction.source_run_id, "completed-source");
    assert_eq!(prediction.text.as_deref(), Some("Review this source"));
    assert_eq!(primary.load(Ordering::SeqCst), 1);
    assert_eq!(broker.load(Ordering::SeqCst), 0);
}
