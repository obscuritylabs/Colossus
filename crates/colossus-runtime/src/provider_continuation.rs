use super::*;
use colossus_contracts::ProviderContinuationPlan;
use colossus_provider::{ProviderFeature, ProviderFeatureRejection};

fn fingerprint(value: &impl Serialize) -> Result<String, ModelProviderError> {
    serde_json::to_vec(value)
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
        .map_err(|_| ModelProviderError::Configuration("unable to bind provider state".into()))
}

pub(super) fn projection_prefix_changed(
    source_count: usize,
    canonical: &[ModelMessage],
    projected: &[ModelMessage],
    image_inputs: bool,
) -> bool {
    projected.len() != canonical.len()
        || canonical.get(..source_count).is_none_or(|source| {
            projected.get(..source_count).is_none_or(|prefix| {
                let mut source = source.to_vec();
                if !image_inputs {
                    colossus_tools::omit_unsupported_model_tool_images(&mut source);
                }
                colossus_tools::project_model_tool_observations(&source) != prefix
            })
        })
}

impl GatewayModelProvider {
    pub(super) fn plan(
        &self,
        role: &str,
        request: &ModelRequest,
        context: &ExecutionContext,
        options: &colossus_contracts::WorkflowAgentOptions,
    ) -> Result<Option<ProviderContinuationPlan>, ModelProviderError> {
        let resolved = self.resolve_workflow_model(role, options)?;
        let route = self.route_with_options(role, options)?;
        if !self.providers.server_compaction_enabled(&resolved)
            && resolved.provider().profile().kind
                != colossus_provider::ProviderKind::AnthropicMessages
        {
            return Ok(None);
        }
        let Some(session_id) = &context.session_id else {
            return Ok(None);
        };
        let canonical = self
            .sessions
            .list_messages(session_id)
            .map_err(|e| ModelProviderError::Failed(e.to_string()))?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        // Transient recovery prompts have no durable watermark, so use local context.
        let mut projection_source = canonical.clone();
        if !route.capabilities.image_inputs {
            colossus_tools::omit_unsupported_model_tool_images(&mut projection_source);
        }
        let projected = colossus_tools::project_model_tool_observations(&projection_source);
        if projected != request.messages
            && colossus_tools::project_model_tool_observations_preserving_current_turn(
                &projection_source,
            ) != request.messages
        {
            return Ok(None);
        }
        let binding = fingerprint(&(
            resolved.provider().profile().kind.as_str(),
            &resolved.provider().profile().base_url,
            &resolved.provider().profile().name,
            &resolved.provider().profile().credential_reference,
            &route.model,
            &request.instructions,
            &request.tools,
            route.reasoning_effort,
        ))?;
        let snapshot_epoch = self
            .snapshots
            .activation_epoch(session_id)
            .map_err(|e| ModelProviderError::Failed(e.to_string()))?;
        let mut previous = self.continuations.load(session_id).map_err(|_| {
            ModelProviderError::Failed("unable to load protected Responses state".into())
        })?;
        // Encrypted Responses compaction may contain observations superseded by
        // a later read. Retire that opaque prefix rather than reintroducing it.
        if previous.as_ref().is_some_and(|state| {
            !state.view.replay
                && projection_prefix_changed(
                    state.plan.source_count,
                    &canonical,
                    &projected,
                    route.capabilities.image_inputs,
                )
        }) {
            self.continuations.clear(session_id, context).map_err(|_| {
                ModelProviderError::Failed("unable to retire superseded Responses state".into())
            })?;
            previous = None;
        }
        let selected = previous
            .filter(|state| {
                state.view.replay
                    == (resolved.provider().profile().kind
                        == colossus_provider::ProviderKind::AnthropicMessages)
                    && (!state.view.replay
                        || !canonical[state.view.covered_count.min(canonical.len())..]
                            .iter()
                            .any(|m| m.begins_user_turn()))
                    && state.plan.binding == binding
                    && state.plan.snapshot_epoch == snapshot_epoch
                    && state.view.covered_count <= canonical.len()
                    && fingerprint(&canonical[..state.view.covered_count].to_vec())
                        .is_ok_and(|hash| hash == state.settled_hash)
            })
            .map(|state| state.view);
        Ok(Some(ProviderContinuationPlan {
            session_id: session_id.clone(),
            binding,
            // Exact no-preparer binding. A context service replaces this with
            // the digest of its actual decision/memory binding messages.
            context_binding_hash: fingerprint(&Vec::<ModelMessage>::new())?,
            source_count: canonical.len(),
            source_hash: fingerprint(&canonical)?,
            snapshot_epoch,
            selected,
        }))
    }

    pub(super) fn remember_candidate(
        &self,
        session: Option<&str>,
        id: Option<String>,
    ) -> Result<(), ModelProviderError> {
        let Some(session) = session else {
            return Ok(());
        };
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| ModelProviderError::Failed("provider state lock poisoned".into()))?;
        if let Some(old) = pending.remove(session) {
            let _ = self.continuations.take_staged(&old);
        }
        if let Some(id) = id {
            if pending.len() >= 64
                && let Some((_, old)) = pending.pop_first()
            {
                let _ = self.continuations.take_staged(&old);
            }
            pending.insert(session.into(), id);
        }
        Ok(())
    }

    pub(super) fn settle(&self, context: &ExecutionContext) -> Result<(), ModelProviderError> {
        let Some(session) = &context.session_id else {
            return Ok(());
        };
        let id = self
            .pending
            .lock()
            .map_err(|_| ModelProviderError::Failed("provider state lock poisoned".into()))?
            .remove(session);
        let Some(id) = id else {
            // A final native response has no new replay candidate. The agent
            // calls settlement only after its canonical assistant append.
            if let Some(state) = self
                .continuations
                .load(session)
                .map_err(|_| {
                    ModelProviderError::Failed("unable to load protected Messages replay".into())
                })?
                .filter(|state| state.view.replay)
            {
                let canonical = self
                    .sessions
                    .list_messages(session)
                    .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
                if canonical.len() > state.view.covered_count
                    && canonical.last().is_some_and(|record| {
                        record.message.role == ModelMessageRole::Assistant
                            && record.message.tool_calls.is_empty()
                    })
                {
                    self.continuations.clear(session, context).map_err(|_| {
                        ModelProviderError::Failed(
                            "unable to retire completed Messages replay".into(),
                        )
                    })?;
                }
            }
            return Ok(());
        };
        let Some(mut state) = self
            .continuations
            .take_staged(&id)
            .map_err(|_| ModelProviderError::Failed("unable to settle Responses state".into()))?
        else {
            return Ok(());
        };
        let canonical = self
            .sessions
            .list_messages(session)
            .map_err(|e| ModelProviderError::Failed(e.to_string()))?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        if state.view.covered_count > canonical.len()
            || canonical.get(state.plan.source_count) != Some(&state.assistant)
            || fingerprint(&canonical[..state.plan.source_count].to_vec())?
                != state.plan.source_hash
            || self
                .snapshots
                .activation_epoch(session)
                .map_err(|e| ModelProviderError::Failed(e.to_string()))?
                != state.plan.snapshot_epoch
            || validate_model_transcript(&canonical).is_err()
        {
            return Ok(());
        }
        state.settled_hash = fingerprint(&canonical[..state.view.covered_count].to_vec())?;
        self.continuations.save(state, context).map_err(|_| {
            ModelProviderError::Failed("unable to persist protected Responses state".into())
        })
    }

    pub(super) fn rejection(
        &self,
        resolved: &colossus_provider::ResolvedModel,
        rejection: &ProviderFeatureRejection,
    ) -> ModelProviderError {
        let behavior = if rejection
            .provider_feature_rejected
            .mode(resolved.model_profile().feature_settings)
            == colossus_contracts::ModelFeatureMode::On
        {
            "On remains selected and subsequent requests will still attempt this feature"
        } else {
            "Auto will suppress this feature for this route for 15 minutes"
        };
        self.providers.observe_feature(
            resolved,
            rejection.provider_feature_rejected,
            "unsupported",
        );
        ModelProviderError::HttpStatus {
            status: rejection.status,
            message: format!(
                "provider rejected {:?}; {behavior}",
                rejection.provider_feature_rejected
            ),
        }
    }

    pub(super) fn accepted(
        &self,
        resolved: &colossus_provider::ResolvedModel,
        request: &ModelRequest,
        streamed: bool,
        verified: bool,
    ) {
        if !request.tools.is_empty() {
            self.providers
                .observe_feature(resolved, ProviderFeature::Tools, "accepted");
        }
        if request
            .messages
            .iter()
            .any(|message| message.content.images().next().is_some())
        {
            self.providers
                .observe_feature(resolved, ProviderFeature::Images, "accepted");
        }
        if streamed {
            self.providers
                .observe_feature(resolved, ProviderFeature::Streaming, "verified");
        }
        if self.providers.server_compaction_enabled(resolved) {
            self.providers.observe_feature(
                resolved,
                ProviderFeature::ServerCompaction,
                if verified { "verified" } else { "accepted" },
            );
        }
    }
}
