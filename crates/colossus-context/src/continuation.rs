use super::*;
use colossus_contracts::ProviderContinuation;
use colossus_ports::ProviderContinuationRepository;
use std::{collections::BTreeMap, sync::Mutex};

/// Bounded private provider state with protected durability and a keyless fallback.
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
        let request_bytes = state
            .replay_request
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| StoreError::Adapter("invalid Messages replay request".into()))?
            .map_or(0, |bytes| bytes.len());
        let mut bounded_state = state.clone();
        bounded_state.replay_request = None;
        let state_bytes = serde_json::to_vec(&bounded_state)
            .map_err(|_| StoreError::Adapter("invalid Responses state".into()))?
            .len()
            // Canonical settlement replaces the empty hash with 64 hex bytes.
            .saturating_add(if state.settled_hash.is_empty() { 64 } else { 0 });
        if state.hidden_reasoning.len() > 512
            || bytes > 512 * 1024
            || state_bytes > 512 * 1024
            || request_bytes > 896 * 1024
            || (state.replay_request.is_some() && !state.view.replay)
            || (!state.route_binding.is_empty() && state.route_binding.len() != 64)
            || bytes != state.view.bytes
            || state.view.reserved_tokens != (bytes as u64).div_ceil(3).saturating_add(64)
            || state.view.id.is_empty()
            || state.view.id.len() > 256
            || state.plan.session_id.is_empty()
            || state.plan.session_id.len() > 256
            || state.plan.context_binding_hash.len() != 64
            || state.view.context_binding_hash != state.plan.context_binding_hash
            || state.view.covered_count != state.plan.source_count.saturating_add(1)
            || !state.hidden_reasoning.first().is_some_and(|item| {
                if state.view.replay {
                    item["type"] == "anthropic_replay"
                        && state.hidden_reasoning.len() > 1
                        && state.hidden_reasoning[1..].iter().all(|entry| {
                            serde_json::from_value::<ModelMessage>(entry["message"].clone())
                                .is_ok_and(|m| {
                                    m.role == ModelMessageRole::Assistant
                                        && !m.tool_calls.is_empty()
                                })
                                && entry["content"].is_array()
                        })
                } else {
                    item["type"] == "compaction"
                        && item["encrypted_content"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                }
            })
        {
            return Err(StoreError::Verification(
                "Responses state violates its bounded contract".into(),
            ));
        }
        Ok(())
    }

    fn state_event(
        &self,
        state: &ProviderContinuation,
        context: &ExecutionContext,
        candidate: bool,
    ) -> Result<NewEvent, StoreError> {
        let journal = self
            .journal
            .as_ref()
            .ok_or_else(|| StoreError::Adapter("protected provider state unavailable".into()))?;
        let stream_id = if candidate {
            format!("provider-continuation-candidate:{}", state.plan.session_id)
        } else {
            format!("provider-continuation:{}", state.plan.session_id)
        };
        let tail = journal.read_stream_backwards(&stream_id, None, 1)?;
        Ok(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: tail.first().map_or(0, |event| event.stream_version),
            classification: EventClassification::Domain,
            event_type: if candidate {
                "provider.continuation.admitted.v1"
            } else {
                "provider.continuation.settled.v1"
            }
            .into(),
            actor: context_actor(context),
            context: context.clone(),
            payload: serde_json::to_value(state)
                .map_err(|_| StoreError::Adapter("invalid provider state".into()))?,
        })
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
        // Quarantined staging is process-local. Only explicit released admission
        // may persist a candidate alongside its session replay guard.
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

    fn candidate(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<Option<ProviderContinuation>, StoreError> {
        if let Some(state) = self
            .staged
            .lock()
            .map_err(|_| StoreError::Adapter("Responses state lock poisoned".into()))?
            .get(id)
            .filter(|state| state.plan.session_id == session_id)
            .cloned()
        {
            return Ok(Some(state));
        }
        let Some(journal) = &self.journal else {
            return Ok(None);
        };
        let tail = journal.read_stream_backwards(
            &format!("provider-continuation-candidate:{session_id}"),
            None,
            1,
        )?;
        let Some(event) = tail
            .first()
            .filter(|event| event.event_type == "provider.continuation.admitted.v1")
        else {
            return Ok(None);
        };
        let state: ProviderContinuation =
            serde_json::from_value(journal.decrypt_payload(event)?)
                .map_err(|_| StoreError::Verification("invalid admitted provider state".into()))?;
        Self::validate(&state)?;
        Ok((state.plan.session_id == session_id && state.view.id == id).then_some(state))
    }

    fn admission_events(
        &self,
        session_id: &str,
        id: &str,
        context: &ExecutionContext,
    ) -> Result<Vec<NewEvent>, StoreError> {
        let state = self.candidate(session_id, id)?.ok_or_else(|| {
            StoreError::Verification("released provider candidate unavailable".into())
        })?;
        Self::validate(&state)?;
        if self.journal.is_none() {
            return Ok(Vec::new());
        }
        Ok(vec![self.state_event(&state, context, true)?])
    }

    fn settlement_events(
        &self,
        state: &ProviderContinuation,
        context: &ExecutionContext,
    ) -> Result<Vec<NewEvent>, StoreError> {
        Self::validate(state)?;
        if state.settled_hash.is_empty() {
            return Err(StoreError::Verification(
                "Responses state is not settled".into(),
            ));
        }
        let Some(journal) = &self.journal else {
            return Ok(Vec::new());
        };
        let mut events = vec![self.state_event(state, context, false)?];
        let stream_id = format!("provider-continuation-candidate:{}", state.plan.session_id);
        let tail = journal.read_stream_backwards(&stream_id, None, 1)?;
        if let Some(candidate) = tail
            .first()
            .filter(|event| event.event_type == "provider.continuation.admitted.v1")
        {
            let admitted: ProviderContinuation =
                serde_json::from_value(journal.decrypt_payload(candidate)?).map_err(|_| {
                    StoreError::Verification("invalid admitted provider state".into())
                })?;
            if admitted.view.id != state.view.id {
                return Err(StoreError::Verification(
                    "provider candidate changed before settlement".into(),
                ));
            }
            events.push(NewEvent {
                event_version: 1,
                stream_id,
                expected_stream_version: candidate.stream_version,
                classification: EventClassification::Domain,
                event_type: "provider.continuation.consumed.v1".into(),
                actor: context_actor(context),
                context: context.clone(),
                payload: Value::Null,
            });
        }
        Ok(events)
    }

    fn retired_replay(&self, session_id: &str) -> Result<Option<ProviderContinuation>, StoreError> {
        let Some(journal) = &self.journal else {
            return Ok(None);
        };
        let tail = journal.read_stream_backwards(
            &format!("provider-continuation:{session_id}"),
            None,
            2,
        )?;
        if tail.len() != 2
            || tail[0].event_type != "provider.continuation.retired.v1"
            || tail[1].event_type != "provider.continuation.settled.v1"
        {
            return Ok(None);
        }
        let state: ProviderContinuation =
            serde_json::from_value(journal.decrypt_payload(&tail[1])?)
                .map_err(|_| StoreError::Verification("invalid retired provider state".into()))?;
        Self::validate(&state)?;
        Ok((state.view.replay && state.plan.session_id == session_id).then_some(state))
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
        journal.append(self.state_event(&state, context, false)?)?;
        Ok(())
    }
}
