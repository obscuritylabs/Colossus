//! Pure catalog resolution. Scope correlation is not native admission authority.
use crate::{
    BrokerError, PkiSelection,
    registry_metadata::{RegistryMetadata, ReviewedScope},
};

pub(super) struct Plan {
    pub ca_indices: Vec<usize>,
    pub identity_index: Option<usize>,
}

pub(super) fn resolve(
    metadata: &RegistryMetadata,
    origin: &str,
    selection: &PkiSelection,
) -> Result<Plan, BrokerError> {
    let denied = BrokerError::InvalidBootstrap;
    selection.encode().map_err(|_| denied)?;
    // The independent native Core admission proves who supplied this selection.
    // The opaque scope digest is only a correlation for that admitted request;
    // it cannot prove workspace/application consent by itself.
    if !metadata.reviews.iter().any(|review| {
        review.origins.iter().any(|allowed| allowed == origin)
            && reviewed_selection(review, selection)
    }) {
        return Err(denied);
    }
    let ca_indices = selection
        .ca_ids
        .iter()
        .map(|id| {
            metadata
                .cas
                .iter()
                .position(|ca| {
                    ca.id == hex::encode(id) && ca.origins.iter().any(|allowed| allowed == origin)
                })
                .ok_or(denied)
        })
        .collect::<Result<Vec<_>, _>>()?;
    // A configured PKI lease gets exactly these independently registered roots.
    // An ordinary Bootstrap's roots cannot silently add trust or serve as an
    // issuer fallback for a selected native identity.
    if ca_indices.is_empty() {
        return Err(denied);
    }
    let identity_index = selection
        .client
        .as_ref()
        .map(|client| {
            metadata
                .identities
                .iter()
                .position(|identity| {
                    identity.id == hex::encode(client.identity_id)
                        && identity.leaf_sha256 == hex::encode(client.leaf_sha256)
                        && identity.origins.iter().any(|allowed| allowed == origin)
                        && identity.ca_ids.iter().all(|issuer| {
                            selection.ca_ids.iter().any(|id| hex::encode(id) == *issuer)
                        })
                })
                .ok_or(denied)
        })
        .transpose()?;
    Ok(Plan {
        ca_indices,
        identity_index,
    })
}

fn reviewed_selection(review: &ReviewedScope, selection: &PkiSelection) -> bool {
    review.ca_ids.len() == selection.ca_ids.len()
        && review.ca_ids.iter().all(|id| {
            selection
                .ca_ids
                .iter()
                .any(|selected| hex::encode(selected) == *id)
        })
        && review.client_id.as_ref()
            == selection
                .client
                .as_ref()
                .map(|client| hex::encode(client.identity_id))
                .as_ref()
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
