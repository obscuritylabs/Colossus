use super::*;
use async_trait::async_trait;
use colossus_sdk::{
    ApiError, ApiErrorReason, ApiResult, CancelRunResponse, CreateRunRequest, CreateRunResponse,
    GetRunResponse, ListRunsResponse, RespondInteractionRequest, RespondInteractionResponse,
    RunUpdateStream,
};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

const HEALTHY: u8 = 0;
const UNAVAILABLE: u8 = 1;
const STALLED: u8 = 2;
const CLOSE_DURING_READ: u8 = 3;
const CLOSED: u8 = 4;

struct LocalClient {
    state: AtomicU8,
    reads: AtomicUsize,
}

impl LocalClient {
    fn new(state: u8) -> Self {
        Self {
            state: AtomicU8::new(state),
            reads: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl AgentRunClient for LocalClient {
    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        unreachable!("readiness must never execute work")
    }
    async fn get_run(&self, _: GetRunRequest) -> ApiResult<GetRunResponse> {
        unreachable!("readiness must not require an existing run")
    }
    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        assert_eq!(
            request.page.expect("bounded readiness page").page_size,
            1,
            "readiness must bound its local read"
        );
        self.reads.fetch_add(1, Ordering::AcqRel);
        match self.state.load(Ordering::Acquire) {
            UNAVAILABLE => {
                return Err(ApiError::permission_denied(
                    ApiErrorReason::ScopeDenied,
                    "private adapter detail must not reach cloud status",
                ));
            }
            STALLED => return std::future::pending().await,
            CLOSE_DURING_READ => self.state.store(CLOSED, Ordering::Release),
            CLOSED => unreachable!("closed clients must not be probed"),
            _ => {}
        }
        Ok(ListRunsResponse {
            runs: Vec::new(),
            page: None,
        })
    }
    async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        unreachable!("readiness must not open an event feed")
    }
    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        unreachable!("readiness must never cancel work")
    }
    async fn respond_interaction(
        &self,
        _: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        unreachable!("readiness must never answer an interaction")
    }
    fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire) == CLOSED
    }
}

#[tokio::test]
async fn daemon_readiness_detects_outage_and_recovery_without_client_closure() {
    let client = LocalClient::new(HEALTHY);
    check_local_readiness(&client).await.unwrap();
    client.state.store(UNAVAILABLE, Ordering::Release);
    assert!(!client.is_closed());
    let error = check_local_readiness(&client).await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unavailable);
    assert_eq!(error.message(), "local runtime unavailable");
    client.state.store(HEALTHY, Ordering::Release);
    check_local_readiness(&client).await.unwrap();
    assert_eq!(client.reads.load(Ordering::Acquire), 3);
}

#[tokio::test]
async fn daemon_readiness_bounds_a_stalled_local_transport() {
    let client = LocalClient::new(STALLED);
    let error = tokio::time::timeout(Duration::from_secs(4), check_local_readiness(&client))
        .await
        .expect("stalled local transport must return to bounded reconnect")
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::DeadlineExceeded);
    assert_eq!(error.message(), "local readiness timeout");
}

#[tokio::test]
async fn daemon_readiness_rejects_shutdown_before_and_during_a_probe() {
    let closed = LocalClient::new(CLOSED);
    assert!(check_local_readiness(&closed).await.is_err());
    assert_eq!(closed.reads.load(Ordering::Acquire), 0);
    let closing = LocalClient::new(CLOSE_DURING_READ);
    assert!(check_local_readiness(&closing).await.is_err());
    assert!(closing.is_closed());
}
