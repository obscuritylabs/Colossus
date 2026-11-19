use super::*;
use async_trait::async_trait;
use colossus_sdk::{
    AgentRunClient, ApiResult, Backend, BackendKind, CancelRunRequest, CancelRunResponse,
    CreateRunRequest, CreateRunResponse, GetRunRequest, GetRunResponse, RespondInteractionRequest,
    RespondInteractionResponse, RunUpdateStream, SdkResult, WatchRunRequest,
};
use std::{future::Future as _, task::Poll};

#[derive(Clone, Default)]
struct SelectionClient {
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl AgentRunClient for SelectionClient {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        panic!("selection must not create a run")
    }

    async fn get_run(&self, _: GetRunRequest) -> ApiResult<GetRunResponse> {
        panic!("selection must not read a run")
    }

    async fn list_runs(&self, _: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        panic!("selection must not list runs")
    }

    async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        panic!("selection must not watch a run")
    }

    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        panic!("selection must not cancel a run")
    }

    async fn respond_interaction(
        &self,
        _: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        panic!("selection must not answer an interaction")
    }
}

#[async_trait]
impl Backend for SelectionClient {
    fn kind(&self) -> BackendKind {
        BackendKind::Sidecar
    }

    fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        Arc::new(self.clone())
    }

    async fn close(&self) -> SdkResult<()> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

async fn register_selected_target(state: &AppState) -> SelectionClient {
    let backend = SelectionClient::default();
    state
        .replace_target(
            MANAGED_TARGET_ID,
            Colossus::from_backend(backend.clone()),
            TargetConsentContext::ManagedLocal,
        )
        .await;
    state.select_target(Some(MANAGED_TARGET_ID.into())).await;
    backend
}

#[tokio::test]
async fn queued_selection_writer_does_not_block_certificate_target_lookup() {
    let state = AppState::default();
    let _backend = register_selected_target(&state).await;
    let before = state
        .try_selected_target(MANAGED_TARGET_ID)
        .expect("registered current target");
    assert_eq!(before.epoch(), *state.subscribe_selection().borrow());
    assert_eq!(before.target.consent, TargetConsentContext::ManagedLocal);
    drop(before);

    // Initial native Open retains this guard while its certificate callback waits.
    let opening = state.browser_selection().await;
    let mut switching = std::pin::pin!(state.select_target(None));
    let queued =
        std::future::poll_fn(|context| Poll::Ready(switching.as_mut().poll(context).is_pending()))
            .await;
    assert!(queued, "the workspace writer must be queued behind Open");
    assert!(state.selected_target_id.try_read().is_err());
    assert!(state.try_selected_target(MANAGED_TARGET_ID).is_none());

    drop(opening);
    tokio::time::timeout(Duration::from_secs(1), switching)
        .await
        .expect("declining the challenge releases Open and the queued writer");
    assert!(state.selected_target_id().await.is_none());
    assert!(state.try_selected_target(MANAGED_TARGET_ID).is_none());
}

#[tokio::test]
async fn certificate_target_lookup_requires_an_available_registered_open_client() {
    let state = AppState::default();
    state.select_target(Some(MANAGED_TARGET_ID.into())).await;
    assert!(state.try_selected_target(MANAGED_TARGET_ID).is_none());
    let backend = register_selected_target(&state).await;
    assert!(state.try_selected_target("unselected-target").is_none());

    let registry_update = state.targets.write().await;
    assert!(state.try_selected_target(MANAGED_TARGET_ID).is_none());
    drop(registry_update);
    let lease = state
        .try_selected_target(MANAGED_TARGET_ID)
        .expect("registry update released");
    assert_eq!(lease.target.client.backend_kind(), BackendKind::Sidecar);
    assert_eq!(lease.epoch(), *state.subscribe_selection().borrow());
    drop(lease);

    backend.close().await.expect("fixture client close");
    assert!(state.try_selected_target(MANAGED_TARGET_ID).is_none());
}

#[tokio::test]
async fn history_admission_preserves_recent_active_binding_and_rejects_wrong_target() {
    let state = AppState::default();
    register_selected_target(&state).await;
    let lease = state.selected_target(MANAGED_TARGET_ID).await.unwrap();
    state.bind_runs(&lease, vec!["active".into()]).await;
    for page in 0..crate::run_bindings::MAX_BINDINGS / 3 + 2 {
        assert!(state.run_is_bound(&lease, "active").await);
        state
            .bind_runs(
                &lease,
                (0..3)
                    .map(|offset| format!("history-{}", page * 3 + offset))
                    .collect(),
            )
            .await;
    }
    assert_eq!(
        state.run_targets.read().await.len(),
        crate::run_bindings::MAX_BINDINGS
    );
    assert!(state.run_is_bound(&lease, "active").await);
    assert!(!state.run_is_bound(&lease, "history-0").await);
    state
        .run_targets
        .write()
        .await
        .insert("foreign".into(), "another-target".into());
    // The fixture panics on SDK calls, proving mismatch is rejected before any
    // attempt to rediscover or rebind it against the current target.
    let result = crate::commands::require_run_binding(&state, &lease, "foreign").await;
    assert!(result.is_err());
    assert_eq!(
        state.run_binding_target("foreign").await.as_deref(),
        Some("another-target")
    );
}
