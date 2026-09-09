//! Platform-independent projection and routing for an authenticated sidecar pair.
//!
//! Reads retain the primary application's scope. Only approval answers use the
//! separately bootstrapped broker; the server still authorizes every response.

use std::sync::Arc;

use futures::StreamExt as _;

use crate::{
    AgentRunClient, ApiResult, GetRunRequest, GetRunResponse, Interaction, InteractionAnswer,
    InteractionContent, InteractionKind, InteractionStatus, RespondInteractionRequest,
    RespondInteractionResponse, RunUpdateKind, RunUpdateStream, WatchRunRequest,
};

#[derive(Clone)]
pub(super) struct AgentRunTransports {
    pub(super) primary: Arc<dyn AgentRunClient>,
    pub(super) approval_broker: Option<Arc<dyn AgentRunClient>>,
}

impl AgentRunTransports {
    pub(super) async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        let mut response = self.primary.get_run(request).await?;
        if self.approval_broker.is_some() {
            response
                .pending_interactions
                .iter_mut()
                .for_each(expose_approval_broker_capability);
        }
        Ok(response)
    }

    pub(super) async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        let stream = self.primary.watch_run(request).await?;
        if self.approval_broker.is_none() {
            return Ok(stream);
        }
        Ok(Box::pin(stream.map(|item| {
            item.map(|mut update| {
                if let RunUpdateKind::Interaction(interaction) = &mut update.update {
                    expose_approval_broker_capability(interaction);
                }
                update
            })
        })))
    }

    pub(super) async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        let transport = if matches!(&request.response, InteractionAnswer::Approval { .. }) {
            self.approval_broker.as_ref().unwrap_or(&self.primary)
        } else {
            &self.primary
        };
        transport.respond_interaction(request).await
    }
}

fn expose_approval_broker_capability(interaction: &mut Interaction) {
    if interaction.kind == InteractionKind::Approval
        && interaction.status == InteractionStatus::Pending
        && !interaction.etag.is_empty()
        && matches!(&interaction.content, InteractionContent::Approval(_))
    {
        interaction.respondable_by_caller = true;
    }
}

#[cfg(test)]
mod tests;
