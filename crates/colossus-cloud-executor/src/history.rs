//! Canonical released conversation reads with a bounded JSON transport budget.
use colossus_cloud_protocol::{CloudReply, MAX_DISCOVERY_PAGE_SIZE, encode};
use colossus_sdk::{
    AgentRunClient, ApiError, ApiErrorReason, ApiResult, ListSessionActivityRequest,
    ListSessionActivityResponse, PageRequest, SessionActivityKind, SessionActivityLane,
};
use std::future::Future;

pub(crate) async fn read(
    runs: &dyn AgentRunClient,
    source_run_id: String,
    page_token: Option<String>,
    page_size: u32,
) -> ApiResult<CloudReply> {
    bounded(source_run_id, page_token, page_size, |request| {
        runs.list_session_activity(request)
    })
    .await
}

async fn bounded<F, Fut>(
    source_run_id: String,
    page_token: Option<String>,
    mut page_size: u32,
    fetch: F,
) -> ApiResult<CloudReply>
where
    F: Fn(ListSessionActivityRequest) -> Fut,
    Fut: Future<Output = ApiResult<ListSessionActivityResponse>>,
{
    if page_size == 0
        || page_size > MAX_DISCOVERY_PAGE_SIZE
        || page_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > 512)
    {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "page",
            "invalid history page",
        ));
    }
    loop {
        let response = fetch(ListSessionActivityRequest {
            source_run_id: source_run_id.clone(),
            query: String::new(),
            lanes: vec![SessionActivityLane::Agent],
            kinds: vec![SessionActivityKind::User, SessionActivityKind::Assistant],
            statuses: vec![],
            page: Some(PageRequest {
                page_size,
                page_token: page_token.clone().unwrap_or_default(),
            }),
        })
        .await?;
        let reply = CloudReply::History { response };
        if encode(&reply).is_ok() {
            return Ok(reply);
        }
        if page_size == 1 {
            return Err(ApiError::bounded_resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "released history exceeds the cloud frame bound",
            ));
        }
        // Activity cursors bind the caller/session/filters, deliberately excluding
        // page size. Retrying this read rechecks sharing and never skips a record.
        page_size = (page_size / 2).max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_sdk::{PageResponse, SessionActivity, SessionActivityContent};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    #[tokio::test]
    async fn json_escaping_shrinks_read_page_without_changing_source_cursor() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let calls = requests.clone();
        let reply = bounded(
            "source-run".into(),
            Some("opaque-cursor".into()),
            32,
            move |request| {
                let calls = calls.clone();
                async move {
                    let page = request.page.expect("bounded page");
                    calls
                        .lock()
                        .expect("requests")
                        .push((page.page_size, page.page_token.clone()));
                    assert_eq!(request.lanes, vec![SessionActivityLane::Agent]);
                    let activity = SessionActivity {
                        activity_id: "message-1".into(),
                        run_id: Some(request.source_run_id),
                        turn: None,
                        lane: SessionActivityLane::Agent,
                        kind: SessionActivityKind::User,
                        title: "User".into(),
                        summary: String::new(),
                        actor: "local".into(),
                        status: None,
                        started_at: "2026-10-05T00:00:00Z".into(),
                        completed_at: None,
                        duration_ms: None,
                        input: Some(SessionActivityContent {
                            format: "text".into(),
                            value: "\"\n\\".repeat(21845),
                        }),
                        result: None,
                        attributes: BTreeMap::new(),
                        source_event_types: vec!["session.message.appended.v1".into()],
                        first_sequence: 1,
                        last_sequence: 1,
                    };
                    Ok(ListSessionActivityResponse {
                        activities: vec![activity; page.page_size as usize],
                        page: Some(PageResponse {
                            next_page_token: "next-cursor".into(),
                        }),
                        head_sequence: 1,
                        projected_through_sequence: 1,
                        caught_up: true,
                    })
                }
            },
        )
        .await
        .expect("smaller released page fits");
        let pages = requests.lock().expect("requests");
        assert!(pages.len() > 1);
        assert_eq!(pages[0].0, 32);
        assert!(pages.iter().all(|(_, token)| token == "opaque-cursor"));
        assert!(encode(&reply).is_ok());
        match reply {
            CloudReply::History { response } => assert_eq!(
                response.page.expect("next page").next_page_token,
                "next-cursor"
            ),
            _ => panic!("history reply"),
        }
    }
}
