//! Read-only tenant transcript hydration using the ordinary fenced Hosted owner.
use super::{management::Management, *};
use colossus_sdk::{ReadSessionTranscriptRequest, TranscriptPage};

impl HostedExecution {
    pub(crate) async fn read_transcript(
        &self,
        state: &Arc<State>,
        caller: &CloudCaller,
        request: ReadSessionTranscriptRequest,
    ) -> std::result::Result<TranscriptPage, CloudError> {
        request
            .validate()
            .map_err(|_| CloudError::InvalidArgument)?;
        caller.require(CloudPermission::Read)?;
        // Hydration is a host operation, as for managed-file reads. This loader can
        // open the caller's existing runtime but cannot admit or execute a run.
        let loader = CloudCaller::new(
            caller.subject().into(),
            caller.project_id().into(),
            [CloudPermission::Read, CloudPermission::Execute]
                .into_iter()
                .collect(),
        )?;
        self.ensure(state, &loader).await?;
        let _lifecycle = self.lifecycle.lock().await;
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        let live = self.live.lock().await.get(&identity).cloned();
        if let Some(live) = live {
            if *live.shutdown.borrow() {
                return Err(CloudError::Conflict);
            }
            if live.project_id != caller.project_id() || live.user_id != caller.subject() {
                return Err(CloudError::PermissionDenied);
            }
            if let (Some(shared), Some(lease)) = (&self.shared, &live.lease) {
                shared.sql.verify_lease(lease)?;
            }
            let page = Self::handle_transcript(&live, request).await?;
            if let (Some(shared), Some(lease)) = (&self.shared, &live.lease) {
                shared.sql.verify_lease(lease)?;
            }
            return Ok(page);
        }
        let lease = self
            .shared
            .as_ref()
            .ok_or(CloudError::Storage)?
            .sql
            .read_lease(&identity)?;
        if lease.project_id != caller.project_id() || lease.user_id != caller.subject() {
            return Err(CloudError::PermissionDenied);
        }
        let value = self
            .management_request(&lease, Management::Transcript { request })
            .await?;
        serde_json::from_value(value).map_err(|_| CloudError::Storage)
    }

    pub(super) async fn handle_transcript(
        live: &LiveRuntime,
        request: ReadSessionTranscriptRequest,
    ) -> std::result::Result<TranscriptPage, CloudError> {
        request
            .validate()
            .map_err(|_| CloudError::InvalidArgument)?;
        let _operation = live.pin();
        live.runtime
            .runs()
            .read_session_transcript(request)
            .await
            .map_err(|error| match error.code {
                colossus_sdk::ApiErrorCode::NotFound => CloudError::NotFound,
                colossus_sdk::ApiErrorCode::PermissionDenied => CloudError::PermissionDenied,
                colossus_sdk::ApiErrorCode::InvalidArgument => CloudError::InvalidArgument,
                colossus_sdk::ApiErrorCode::Conflict
                | colossus_sdk::ApiErrorCode::FailedPrecondition => CloudError::Conflict,
                colossus_sdk::ApiErrorCode::ResourceExhausted => CloudError::ResourceExhausted,
                _ => CloudError::Storage,
            })
    }
}
