use super::*;
use colossus_contracts::ProviderContinuation;
use colossus_ports::ProviderContinuationRepository;
use std::{collections::BTreeMap, sync::Mutex};

/// Bounded Responses state with protected durability and a keyless memory fallback.
pub struct EventSourcedProviderContinuations {
    journal: Option<Arc<dyn EventJournal>>,
    staged: Mutex<BTreeMap<String, ProviderContinuation>>,
    memory: Mutex<BTreeMap<String, ProviderContinuation>>,
}

impl EventSourcedProviderContinuations {
    /// Never write opaque content to a keyless journal.
    pub fn new(journal: Arc<dyn EventJournal>, protected: bool) -> Self {
        Self {
            journal: protected.then_some(journal),
            staged: Mutex::default(),
            memory: Mutex::default(),
        }
    }

    fn validate(state: &ProviderContinuation) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(&state.hidden_reasoning)
            .map_err(|_| StoreError::Adapter("invalid Responses state".into()))?
            .len();
        if state.hidden_reasoning.len() > 512
            || bytes > 512 * 1024
            || bytes != state.view.bytes
            || state.view.reserved_tokens != (bytes as u64).div_ceil(3).saturating_add(64)
            || state.view.id.is_empty()
            || state.view.id.len() > 256
            || state.plan.session_id.is_empty()
            || state.plan.session_id.len() > 256
            || state.view.covered_count != state.plan.source_count.saturating_add(1)
            || !state.hidden_reasoning.first().is_some_and(|item| {
                item["type"] == "compaction"
                    && item["encrypted_content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
            })
        {
            return Err(StoreError::Verification(
                "Responses state violates its bounded contract".into(),
            ));
        }
        Ok(())
    }
}

impl ProviderContinuationRepository for EventSourcedProviderContinuations {
    fn clear(&self, session_id: &str, context: &ExecutionContext) -> Result<(), StoreError> {
        self.staged
            .lock()
            .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?
            .retain(|_, state| state.plan.session_id != session_id);
        let Some(journal) = &self.journal else {
            self.memory
                .lock()
                .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?
                .remove(session_id);
            return Ok(());
        };
        let stream_id = format!("provider-continuation:{session_id}");
        let tail = journal.read_stream_backwards(&stream_id, None, 1)?;
        if tail.is_empty() || tail[0].event_type == "provider.continuation.retired.v1" {
            return Ok(());
        }
        journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: tail[0].stream_version,
            classification: EventClassification::Domain,
            event_type: "provider.continuation.retired.v1".into(),
            actor: context_actor(context),
            context: context.clone(),
            payload: Value::Null,
        })?;
        Ok(())
    }
    fn load(&self, session_id: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        let Some(journal) = &self.journal else {
            return Ok(self
                .memory
                .lock()
                .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?
                .get(session_id)
                .cloned());
        };
        let events = journal.read_stream_backwards(
            &format!("provider-continuation:{session_id}"),
            None,
            1,
        )?;
        if events
            .first()
            .is_some_and(|e| e.event_type == "provider.continuation.retired.v1")
        {
            return Ok(None);
        }
        let state = events
            .first()
            .map(|event| {
                serde_json::from_value::<ProviderContinuation>(journal.decrypt_payload(event)?)
                    .map_err(|_| {
                        StoreError::Verification("invalid protected Responses state".into())
                    })
            })
            .transpose()?;
        if let Some(state) = &state {
            Self::validate(state)?;
        }
        Ok(state)
    }

    fn stage(&self, state: ProviderContinuation) -> Result<(), StoreError> {
        Self::validate(&state)?;
        let mut staged = self
            .staged
            .lock()
            .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?;
        // One candidate per session; incomplete turns never accumulate or persist.
        staged.retain(|_, prior| prior.plan.session_id != state.plan.session_id);
        if staged.len() >= 64 {
            staged.pop_first();
        }
        staged.insert(state.view.id.clone(), state);
        Ok(())
    }

    fn take_staged(&self, id: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        Ok(self
            .staged
            .lock()
            .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?
            .remove(id))
    }

    fn save(
        &self,
        state: ProviderContinuation,
        context: &ExecutionContext,
    ) -> Result<(), StoreError> {
        Self::validate(&state)?;
        if state.settled_hash.is_empty() {
            return Err(StoreError::Verification(
                "Responses state is not settled".into(),
            ));
        }
        let Some(journal) = &self.journal else {
            let mut memory = self
                .memory
                .lock()
                .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?;
            if memory.len() >= 64 && !memory.contains_key(&state.plan.session_id) {
                memory.pop_first();
            }
            memory.insert(state.plan.session_id.clone(), state);
            return Ok(());
        };
        let stream_id = format!("provider-continuation:{}", state.plan.session_id);
        let tail = journal.read_stream_backwards(&stream_id, None, 1)?;
        journal.append(NewEvent {
            event_version: 1,
            expected_stream_version: tail.first().map_or(0, |e| e.stream_version),
            stream_id,
            classification: EventClassification::Domain,
            event_type: "provider.continuation.settled.v1".into(),
            actor: context_actor(context),
            context: context.clone(),
            payload: serde_json::to_value(state)
                .map_err(|_| StoreError::Adapter("invalid Responses state".into()))?,
        })?;
        Ok(())
    }
}
