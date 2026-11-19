//! Full released transcript paging under the closed Cloud JSON frame budget.
use colossus_cloud_protocol::{ResourceReply, encode};
use colossus_sdk::{
    AgentRunClient, ApiError, ApiErrorReason, ApiResult, ReadSessionTranscriptRequest,
};

pub(crate) async fn read(
    runs: &dyn AgentRunClient,
    source_run_id: String,
    page_token: Option<String>,
    page_size: u32,
) -> ApiResult<serde_json::Value> {
    let mut page_size = if page_size == 0 { 32 } else { page_size };
    if page_size > 32
        || source_run_id.is_empty()
        || source_run_id.len() > 128
        || source_run_id.chars().any(char::is_control)
        || page_token.as_ref().is_some_and(|token| {
            token.is_empty() || token.len() > 4096 || token.chars().any(char::is_control)
        })
    {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "page",
            "invalid transcript page",
        ));
    }
    loop {
        let page = runs
            .read_session_transcript(ReadSessionTranscriptRequest {
                source_run_id: source_run_id.clone(),
                page_size,
                page_token: page_token.clone(),
            })
            .await?;
        let value = serde_json::to_value(page).map_err(|_| {
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "transcript projection unavailable",
            )
        })?;
        if encode(&ResourceReply::Result {
            value: value.clone(),
        })
        .is_ok()
        {
            return Ok(value);
        }
        if page_size == 1 {
            return Err(ApiError::bounded_resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "one transcript record exceeds the cloud transport bound",
            ));
        }
        // Retry the same read cursor; no record is omitted or shortened to fit JSON.
        page_size = (page_size / 2).max(1);
    }
}
