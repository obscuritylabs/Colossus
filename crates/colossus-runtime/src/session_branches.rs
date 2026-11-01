//! Private provenance for the canonical prefix imported by a branch.

use super::*;

const PREPARED_EVENT: &str = "session.branch.prepared.v1";
const MAX_PREFIX_MESSAGES: usize = 512;
const MAX_PREFIX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedBranch {
    session_id: String,
    run_id: String,
    source_session_id: String,
    source_message_count: u64,
    context_mode: RunBranchContextMode,
    copied_message_count: u64,
    prefix_sha256: String,
}

#[cfg(test)]
#[path = "session_branch_tests.rs"]
mod tests;

fn stream(session_id: &str) -> String {
    format!("runtime.session.branch:{session_id}")
}

fn invalid() -> StoreError {
    StoreError::Verification("the canonical branch input boundary could not be verified".into())
}

fn prefix_digest(messages: &[&ModelMessage]) -> Result<String, StoreError> {
    if messages.len() > MAX_PREFIX_MESSAGES {
        return Err(invalid());
    }
    let encoded = serde_json::to_vec(messages).map_err(|_| invalid())?;
    if encoded.len() > MAX_PREFIX_BYTES {
        return Err(invalid());
    }
    Ok(hex::encode(Sha256::digest(encoded)))
}

impl Runtime {
    fn prepared_branch(&self, session_id: &str) -> Result<Option<PreparedBranch>, StoreError> {
        let events = self.journal.read_stream_from(&stream(session_id), 0, 2)?;
        let Some(event) = events.first() else {
            return Ok(None);
        };
        if events.len() != 1
            || event.event_version != 1
            || event.event_type != PREPARED_EVENT
            || event.classification != EventClassification::Domain
            || event.context.session_id.as_deref() != Some(session_id)
        {
            return Err(invalid());
        }
        let receipt: PreparedBranch =
            serde_json::from_value(self.journal.decrypt_payload(event)?).map_err(|_| invalid())?;
        if receipt.session_id != session_id
            || receipt.run_id.is_empty()
            || receipt.source_session_id.is_empty()
            || receipt.source_message_count > MAX_PREFIX_MESSAGES as u64
            || receipt.copied_message_count > receipt.source_message_count
            || receipt.prefix_sha256.len() != 64
            || event.context.run_id.as_deref() != Some(receipt.run_id.as_str())
        {
            return Err(invalid());
        }
        Ok(Some(receipt))
    }

    /// Verify the private imported-prefix receipt without consulting its source.
    /// This is a runtime coordinator seam, not an application authority grant.
    pub fn session_branch_input_boundary(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<Option<u64>, RuntimeError> {
        let Some(receipt) = self.prepared_branch(session_id)? else {
            return Ok(None);
        };
        if receipt.run_id != run_id {
            return Err(invalid().into());
        }
        let mut messages = Vec::new();
        let mut before = Some(receipt.copied_message_count + 1);
        while messages.len() < receipt.copied_message_count as usize {
            let page = self.session_messages_page(session_id, before, 64)?;
            if page.messages.is_empty() || page.before_sequence == before {
                return Err(invalid().into());
            }
            before = page.before_sequence;
            messages.extend(page.messages);
            if messages.len() > MAX_PREFIX_MESSAGES {
                return Err(invalid().into());
            }
        }
        messages.sort_by_key(|message| message.sequence);
        if messages.len() != receipt.copied_message_count as usize
            || messages.iter().enumerate().any(|(index, message)| {
                message.sequence != index as u64 + 1
                    || message.session_id != session_id
                    || message.run_id != run_id
            })
            || prefix_digest(
                &messages
                    .iter()
                    .map(|message| &message.message)
                    .collect::<Vec<_>>(),
            )? != receipt.prefix_sha256
        {
            return Err(invalid().into());
        }
        Ok(Some(receipt.copied_message_count))
    }

    pub(super) fn branch_input_receipt(
        &self,
        source: (&str, u64, RunBranchContextMode),
        destination: (&str, &str),
        prefix: &[SessionMessage],
        actor: Actor,
    ) -> Result<Option<NewEvent>, RuntimeError> {
        let (source_session_id, source_message_count, context_mode) = source;
        let (session_id, run_id) = destination;
        let receipt = PreparedBranch {
            session_id: session_id.into(),
            run_id: run_id.into(),
            source_session_id: source_session_id.into(),
            source_message_count,
            context_mode,
            copied_message_count: prefix.len() as u64,
            prefix_sha256: prefix_digest(
                &prefix
                    .iter()
                    .map(|message| &message.message)
                    .collect::<Vec<_>>(),
            )?,
        };
        if let Some(existing) = self.prepared_branch(session_id)? {
            if existing != receipt {
                return Err(invalid().into());
            }
            self.session_branch_input_boundary(session_id, run_id)?;
            return Ok(None);
        }
        Ok(Some(NewEvent {
            event_version: 1,
            stream_id: stream(session_id),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: PREPARED_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: run_id.into(),
                session_id: Some(session_id.into()),
                run_id: Some(run_id.into()),
                ..Default::default()
            },
            payload: serde_json::to_value(receipt).map_err(|_| invalid())?,
        }))
    }
}
