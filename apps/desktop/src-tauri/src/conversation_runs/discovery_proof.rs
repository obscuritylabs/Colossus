//! Discovery receipts locate an owned page; only authenticated replay proves ownership.
use super::{
    ConversationRuns, ConversationSource, MAX_PAGE_SIZE, SourceBinding, invalid, reader_for,
    source_changed,
};
use crate::run_list::RunList;
use colossus_sdk::{AgentRunClient, ApiError, ListRunsRequest, Run};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const MAX_PROOF_BYTES: usize = 4096;

#[derive(Clone, Default)]
pub(crate) struct ConversationProofs {
    pub(crate) discovery: Option<String>,
    pub(crate) continuation: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscoveryPage {
    pub(super) scope: String,
    pub(super) connector: bool,
    pub(super) request: ListRunsRequest,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscoveryProof {
    pub(super) page: DiscoveryPage,
    pub(super) run_id: String,
    pub(super) session_id: String,
}

impl DiscoveryProof {
    pub(super) fn encode(page: &DiscoveryPage, run: &Run) -> Result<String, ApiError> {
        let value = serde_json::to_string(&Self {
            page: page.clone(),
            run_id: run.run_id.clone(),
            session_id: run.session_id.clone(),
        })
        .map_err(|_| invalid())?;
        if value.len() > MAX_PROOF_BYTES {
            return Err(invalid());
        }
        Ok(value)
    }

    pub(super) fn decode(value: &str, scope: &str) -> Result<Self, ApiError> {
        if value.len() > MAX_PROOF_BYTES {
            return Err(invalid());
        }
        let proof: Self = serde_json::from_str(value).map_err(|_| invalid())?;
        let Some(page) = &proof.page.request.page else {
            return Err(invalid());
        };
        if proof.page.scope != scope
            || proof.run_id.is_empty()
            || proof.run_id.len() > 128
            || proof.session_id.is_empty()
            || proof.session_id.len() > 128
            || !(1..=MAX_PAGE_SIZE).contains(&page.page_size)
            || page.page_token.len() > 512
            || page.page_token.chars().any(char::is_control)
            || proof.page.request.statuses.len() > 9
            || proof
                .page
                .request
                .session_id
                .as_ref()
                .is_some_and(|session| session != &proof.session_id)
        {
            return Err(invalid());
        }
        Ok(proof)
    }

    pub(super) fn source(&self) -> ConversationSource {
        if self.page.connector {
            ConversationSource::Cloud
        } else {
            ConversationSource::Primary
        }
    }
}

impl ConversationRuns {
    pub(super) async fn replay_proof(
        &self,
        primary: Arc<dyn AgentRunClient>,
        cloud: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        proof: &DiscoveryProof,
    ) -> Result<SourceBinding, ApiError> {
        let reader = reader_for(proof.source(), primary, cloud)?;
        let page = list
            .list_client(reader.as_ref(), proof.page.request.clone())
            .await?;
        if page.runs.len() > MAX_PAGE_SIZE as usize {
            return Err(source_changed());
        }
        let matched = page
            .runs
            .iter()
            .find(|run| run.run_id == proof.run_id && run.session_id == proof.session_id)
            .ok_or_else(invalid)?;
        // Caller-supplied source flags, cursors and filters are merely locators.
        // This exact record came back from the selected authenticated owner's index.
        self.remember_source(std::slice::from_ref(matched), proof.source())?;
        Ok(SourceBinding {
            source: proof.source(),
            session_id: matched.session_id.clone(),
        })
    }
}
