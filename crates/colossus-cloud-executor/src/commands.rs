//! Execute the closed Cloud operation set through a caller-bound SDK.
use colossus_cloud_protocol::{CloudReply, Command};
use colossus_sdk::{AgentRunClient, CancelRunRequest, GetRunRequest};

/// Execute one operation. Admission, authority, effects and recovery stay in the SDK/runtime.
pub async fn execute(runs: &dyn AgentRunClient, command: Command) -> CloudReply {
    let result = match command {
        Command::InspectInboxes {
            root_run_id,
            participant_id,
            after_sequence,
        } => crate::inboxes::read(runs, root_run_id, participant_id, after_sequence).await,
        Command::History {
            source_run_id,
            page_token,
            page_size,
        } => crate::history::read(runs, source_run_id, page_token, page_size).await,
        Command::Create { request } => match runs.create_run(*request).await {
            Ok(created) => runs
                .get_run(GetRunRequest {
                    run_id: created.run.run_id,
                })
                .await
                .map(|run| CloudReply::Run { run: Box::new(run) }),
            Err(error) => Err(error),
        },
        Command::Cancel {
            run_id,
            idempotency_key,
        } => runs
            .cancel_run(CancelRunRequest {
                run_id,
                idempotency_key,
            })
            .await
            .map(|response| CloudReply::Cancelled { response }),
        Command::Respond { request } => runs
            .respond_interaction(*request)
            .await
            .map(|response| CloudReply::Responded { response }),
        Command::Watch { .. } => Err(colossus_sdk::ApiError::invalid(
            colossus_sdk::ApiErrorReason::InvalidArgument,
            "command",
            "watch operations require an event stream",
        )),
    };
    result.unwrap_or_else(|error| CloudReply::Failed { error })
}
