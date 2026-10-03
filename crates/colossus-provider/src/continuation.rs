use super::*;
use colossus_contracts::{
    ProviderContinuation, ProviderContinuationPlan, ProviderContinuationView,
};

/// Safe adapter output extension. Opaque state stays in the private repository.
#[derive(Serialize, Deserialize)]
pub struct ProviderAdapterTurn {
    /// Existing provider-neutral output.
    #[serde(flatten)]
    pub turn: ProviderTurn,
    /// Candidate reference, reusable only after durable canonical settlement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_id: Option<String>,
}

/// Safe stream extension owned by the Responses adapter.
#[derive(Serialize, Deserialize)]
pub struct ProviderAdapterStreamItem {
    /// Existing provider-neutral stream item.
    #[serde(flatten)]
    pub item: ProviderStreamItem,
    /// Present only on a successfully completed turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_id: Option<String>,
}

impl ProviderExecutor {
    pub(super) fn resolve_continuation(
        &self,
        plan: Option<&ProviderContinuationPlan>,
        session_id: Option<&str>,
    ) -> Result<Option<ProviderContinuation>, ProviderError> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        if self.profile.kind != ProviderKind::OpenAiResponses
            || session_id != Some(plan.session_id.as_str())
        {
            return Err(ProviderError::Configuration(
                "Responses state does not belong to this provider session".into(),
            ));
        }
        let Some(view) = &plan.selected else {
            return Ok(None);
        };
        let state = self
            .continuations
            .as_ref()
            .ok_or_else(|| {
                ProviderError::Configuration("Responses state repository unavailable".into())
            })?
            .load(&plan.session_id)
            .map_err(|_| ProviderError::Configuration("Responses state unavailable".into()))?
            .filter(|state| {
                state.view == *view
                    && state.plan.binding == plan.binding
                    && state.plan.context_binding_hash == plan.context_binding_hash
                    && state.plan.snapshot_epoch == plan.snapshot_epoch
            })
            .ok_or_else(|| {
                ProviderError::Configuration("Responses state reference changed".into())
            })?;
        Ok(Some(state))
    }

    pub(super) fn stage_continuation(
        &self,
        id: &str,
        plan: Option<&ProviderContinuationPlan>,
        payload: &Value,
        output: &[Value],
        turn: &ProviderTurn,
    ) -> Result<Option<String>, ProviderError> {
        if self.profile.kind != ProviderKind::OpenAiResponses {
            return Ok(None);
        }
        let (Some(repository), Some(plan)) = (&self.continuations, plan) else {
            return Ok(None);
        };
        if plan.context_binding_hash.len() != 64 {
            return Ok(None);
        }
        let mut items = payload["input"].as_array().cloned().unwrap_or_default();
        items.extend_from_slice(output);
        let Some(start) = items.iter().rposition(|item| {
            item["type"] == "compaction"
                && item["encrypted_content"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
        }) else {
            return Ok(None);
        };
        let items = items.split_off(start);
        // Never retain resolved image bytes in provider continuation state.
        if contains_image_data(&items) {
            return Ok(None);
        }
        let bytes = serde_json::to_vec(&items)
            .map_err(|_| ProviderError::Malformed("invalid Responses state".into()))?
            .len();
        if items.len() > 512 || bytes > 512 * 1024 {
            return Ok(None);
        }
        let mut text = String::new();
        let mut final_text = None;
        let mut calls = Vec::new();
        for event in &turn.events {
            match event {
                ProviderEvent::ModelDelta { text: delta } => text.push_str(delta),
                ProviderEvent::FinalOutput { text } => final_text = Some(text.clone()),
                ProviderEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments,
                } => calls.push(ModelToolCall {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => {}
            }
        }
        let assistant = ModelMessage {
            role: ModelMessageRole::Assistant,
            content: if calls.is_empty() {
                final_text.unwrap_or(text)
            } else {
                text
            }
            .into(),
            tool_calls: calls,
            tool_call_id: None,
        };
        let state = ProviderContinuation {
            view: ProviderContinuationView {
                id: id.into(),
                covered_count: plan.source_count.saturating_add(1),
                context_binding_hash: plan.context_binding_hash.clone(),
                bytes,
                // Include opaque bytes conservatively; never infer their plaintext contents.
                reserved_tokens: (bytes as u64).div_ceil(3).saturating_add(64),
            },
            plan: plan.clone(),
            settled_hash: String::new(),
            assistant,
            hidden_reasoning: items,
        };
        if serde_json::to_vec(&state)
            .map_err(|_| ProviderError::Malformed("invalid Responses state".into()))?
            .len()
            .saturating_add(64)
            > 512 * 1024
        {
            return Ok(None);
        }
        repository.stage(state).map_err(|_| {
            ProviderError::Configuration("unable to stage bounded Responses state".into())
        })?;
        Ok(Some(id.into()))
    }
}

fn contains_image_data(items: &[Value]) -> bool {
    fn contains(value: &Value) -> bool {
        match value {
            Value::String(text) => text.starts_with("data:image/"),
            Value::Array(values) => values.iter().any(contains),
            Value::Object(values) => values.values().any(contains),
            _ => false,
        }
    }
    items.iter().any(contains)
}
