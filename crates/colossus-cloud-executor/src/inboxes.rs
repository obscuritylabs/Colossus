use colossus_cloud_protocol::CloudReply;
use colossus_sdk::{
    AgentRunClient, ApiError, ApiErrorCode, ApiErrorReason, ApiResult, ListAgentMessagesRequest,
    ListAgentParticipantsRequest,
};

pub(crate) async fn read(
    runs: &dyn AgentRunClient,
    root_run_id: String,
    participant_id: Option<String>,
    after_sequence: u64,
) -> ApiResult<CloudReply> {
    let participants = admitted_read(|| {
        runs.list_agent_participants(ListAgentParticipantsRequest {
            root_run_id: root_run_id.clone(),
        })
    })
    .await?;
    let page = if let Some(id) = participant_id {
        if !participants.iter().any(|participant| participant.id == id) {
            return Err(ApiError::not_found(
                ApiErrorReason::RunNotFound,
                "agent inbox was not found",
            ));
        }
        Some(
            admitted_read(|| {
                runs.list_agent_messages(ListAgentMessagesRequest {
                    participant_id: id.clone(),
                    after_sequence,
                    limit: 16,
                })
            })
            .await?,
        )
    } else {
        None
    };
    Ok(CloudReply::Inboxes { participants, page })
}

async fn admitted_read<T: Send, F, Fut>(read: F) -> ApiResult<T>
where
    F: Fn() -> Fut + Send,
    Fut: std::future::Future<Output = ApiResult<T>> + Send,
{
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match read().await {
                Err(error) if error.code == ApiErrorCode::ResourceExhausted => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                result => return result,
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        Err(ApiError::resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "Inbox read admission timed out",
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn read_throttling_retries_but_authority_denial_does_not() {
        let attempts = AtomicUsize::new(0);
        let result = admitted_read(|| async {
            if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                Err(ApiError::resource_exhausted(
                    ApiErrorReason::CapacityExceeded,
                    "read throttled",
                ))
            } else {
                Ok(())
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        attempts.store(0, Ordering::Relaxed);
        let result: ApiResult<()> = admitted_read(|| async {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(ApiError::permission_denied(
                ApiErrorReason::ScopeDenied,
                "read denied",
            ))
        })
        .await;
        assert_eq!(result.unwrap_err().code, ApiErrorCode::PermissionDenied);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }
}
