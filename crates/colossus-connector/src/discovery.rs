//! Bounded caller-visible discovery; no worker filesystem or primary credential access.
use colossus_cloud_protocol::{MAX_DISCOVERY_PAGE_SIZE, encode, v1alpha1 as wire};
use colossus_sdk::{AgentRunClient, ListRunsRequest, PageRequest};
use std::time::Duration;
use tonic::Status;

pub(crate) async fn discover(
    runs: &dyn AgentRunClient,
    request: wire::DiscoverRuns,
) -> Result<wire::ReleasedDiscoveryPage, Status> {
    if request.sync_id.is_empty()
        || request.sync_id.len() > 128
        || !request
            .sync_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
        || request.page_token.len() > 512
        || request.page_size == 0
        || request.page_size > MAX_DISCOVERY_PAGE_SIZE
    {
        return Err(Status::invalid_argument("invalid discovery request"));
    }
    let query = ListRunsRequest {
        session_id: None,
        statuses: Vec::new(),
        page: Some(PageRequest {
            page_size: request.page_size,
            page_token: request.page_token.clone(),
        }),
        include_archived: request.include_archived,
    };
    // Discovery shares the public listing admission pool with readiness and local
    // callers. Respect its small token bucket instead of dropping the connection
    // while paginating historical sessions.
    let page = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match runs.list_visible_runs(query.clone()).await {
                Ok(page) => return Ok(page),
                Err(error) if error.code == colossus_sdk::ApiErrorCode::ResourceExhausted => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(_) => return Err(Status::unavailable("local discovery unavailable")),
            }
        }
    })
    .await
    .map_err(|_| Status::deadline_exceeded("local discovery timeout"))??;
    // Discovery carries identities and lifecycle. Durable Watch owns released text,
    // so terminal output cannot multiply the inventory page's memory budget.
    let mut released = page.runs;
    for value in &mut released {
        if let Some(colossus_sdk::RunTerminal::Result(result)) = &mut value.run.terminal {
            result.output.clear();
        }
    }
    Ok(wire::ReleasedDiscoveryPage {
        sync_id: request.sync_id,
        page_token: request.page_token,
        runs_json: encode(&released)
            .map_err(|_| Status::resource_exhausted("discovery page too large"))?,
        next_page_token: page
            .page
            .map(|page| page.next_page_token)
            .unwrap_or_default(),
    })
}
