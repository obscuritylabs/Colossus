use super::*;
use colossus_contracts::{ProviderContinuation, ProviderContinuationPlan, ReasoningEffort};
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
            && !(resolved.provider().profile().kind
                == colossus_provider::ProviderKind::AnthropicMessages
                && (request.messages.starts_with(&projected)
                    || request.messages.starts_with(
                        &colossus_tools::project_model_tool_observations_preserving_current_turn(
                            &projection_source,
                        ),
                    )))
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
        run_id: Option<&str>,
        id: Option<String>,
    ) -> Result<(), ModelProviderError> {
        let Some(session) = session else {
            return Ok(());
        };
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| ModelProviderError::Failed("provider state lock poisoned".into()))?;
        let owner = (session.to_owned(), run_id.map(str::to_owned));
        if pending
            .keys()
            .any(|(prior_session, prior_run)| prior_session == session && *prior_run != owner.1)
        {
            return Err(ModelProviderError::Configuration(
                "provider candidate belongs to another active run".into(),
            ));
        }
        if let Some(old) = pending.remove(&owner) {
            let _ = self.continuations.take_staged(&old);
        }
        if let Some(id) = id {
            if pending.len() >= 64 {
                return Err(ModelProviderError::Configuration(
                    "too many active provider continuation candidates".into(),
                ));
            }
            pending.insert(owner, id);
        }
        Ok(())
    }

    fn route_binding(
        &self,
        resolved: &colossus_provider::ResolvedModel,
        effort: Option<ReasoningEffort>,
    ) -> Result<String, ModelProviderError> {
        let profile = resolved.provider().profile();
        fingerprint(&(
            profile.kind.as_str(),
            &profile.base_url,
            &profile.name,
            &profile.credential_reference,
            &resolved.route().model,
            effort,
            resolved.route().capabilities.image_inputs,
        ))
    }

    fn request_binding(
        &self,
        resolved: &colossus_provider::ResolvedModel,
        effort: Option<ReasoningEffort>,
        request: &ModelRequest,
    ) -> Result<String, ModelProviderError> {
        let profile = resolved.provider().profile();
        fingerprint(&(
            profile.kind.as_str(),
            &profile.base_url,
            &profile.name,
            &profile.credential_reference,
            &resolved.route().model,
            &request.instructions,
            &request.tools,
            effort,
        ))
    }

    pub(super) fn remember_released_candidate(
        &self,
        session: Option<&str>,
        run_id: Option<&str>,
        id: Option<String>,
        request: &ModelRequest,
        resolved: &colossus_provider::ResolvedModel,
        effort: Option<ReasoningEffort>,
    ) -> Result<(), ModelProviderError> {
        if let (Some(session), Some(id)) = (session, id.as_deref()) {
            if let Some(mut state) = self.continuations.take_staged(id).map_err(|_| {
                ModelProviderError::Failed("unable to adopt released provider state".into())
            })? {
                if state.plan.session_id != session {
                    return Err(ModelProviderError::Configuration(
                        "provider candidate belongs to another session".into(),
                    ));
                }
                if state.view.replay {
                    state.replay_request = Some(request.clone());
                    state.route_binding = self.route_binding(resolved, effort)?;
                }
                self.continuations.stage(state).map_err(|_| {
                    ModelProviderError::Failed(
                        "unable to retain bounded released provider state".into(),
                    )
                })?;
            } else {
                return Err(ModelProviderError::Failed(
                    "released provider candidate unavailable".into(),
                ));
            }
        }
        self.remember_candidate(session, run_id, id)
    }

    pub(super) fn replay_request(
        &self,
        role: &str,
        request: &ModelRequest,
        context: &ExecutionContext,
        options: &colossus_contracts::WorkflowAgentOptions,
    ) -> Result<Option<(ModelRequest, ProviderContinuationPlan)>, ModelProviderError> {
        let resolved = self.resolve_workflow_model(role, options)?;
        if resolved.provider().profile().kind != colossus_provider::ProviderKind::AnthropicMessages
        {
            if let Some(session) = &context.session_id {
                let state = self
                    .continuations
                    .load(session)
                    .map_err(|_| {
                        ModelProviderError::Failed(
                            "unable to inspect required provider replay".into(),
                        )
                    })?
                    .or(self.continuations.retired_replay(session).map_err(|_| {
                        ModelProviderError::Failed(
                            "unable to inspect retired provider replay".into(),
                        )
                    })?);
                if state.is_some_and(|state| state.view.replay)
                    && self
                        .sessions
                        .list_messages(session)
                        .map_err(|error| ModelProviderError::Failed(error.to_string()))?
                        .iter()
                        .rev()
                        .find(|record| record.message.role == ModelMessageRole::Assistant)
                        .is_some_and(|record| !record.message.tool_calls.is_empty())
                {
                    return Err(ModelProviderError::Configuration(
                        "Messages continuation requires its original provider protocol and route"
                            .into(),
                    ));
                }
            }
            return Ok(None);
        }
        let Some(session) = &context.session_id else {
            return Ok(None);
        };
        let canonical = self
            .sessions
            .list_messages(session)
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let Some(assistant) = canonical
            .iter()
            .rev()
            .find(|message| message.role == ModelMessageRole::Assistant)
            .filter(|message| !message.tool_calls.is_empty())
        else {
            return Ok(None);
        };
        if !request.messages.starts_with(&canonical) {
            return Err(ModelProviderError::Configuration(
                "Messages recovery input does not extend canonical session history".into(),
            ));
        }
        if self
            .sessions
            .pending_tool_turn(session)
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?
            .is_some()
        {
            return Err(ModelProviderError::Configuration(
                "Messages recovery requires reconciliation of its pending tool effects".into(),
            ));
        }
        let current = self.continuations.load(session).map_err(|_| {
            ModelProviderError::Failed("unable to load protected Messages replay".into())
        })?;
        let retired = current.is_none();
        let state = match current {
            Some(state) => Some(state),
            None => self.continuations.retired_replay(session).map_err(|_| {
                ModelProviderError::Failed("unable to inspect retired Messages replay".into())
            })?,
        };
        let Some(state) = state.filter(|state| state.view.replay) else {
            return Err(ModelProviderError::Configuration("Messages continuation has no recoverable private replay; its tool evidence is retained".into()));
        };
        let route = self.route_with_options(role, options)?;
        let epoch = self
            .snapshots
            .activation_epoch(session)
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
        if state.plan.session_id != *session
            || state.assistant != *assistant
            || state.view.covered_count > canonical.len()
            || state.plan.source_count >= canonical.len()
            || canonical.get(state.plan.source_count) != Some(&state.assistant)
            || fingerprint(&canonical[..state.plan.source_count].to_vec())?
                != state.plan.source_hash
            || fingerprint(&canonical[..state.view.covered_count].to_vec())? != state.settled_hash
            || state.plan.snapshot_epoch != epoch
            || validate_model_transcript(&canonical).is_err()
        {
            return Err(ModelProviderError::Configuration("Messages replay no longer matches canonical session history or its context snapshot".into()));
        }
        let (mut replay, protected_count) = if let Some(saved) = &state.replay_request {
            if state.route_binding != self.route_binding(&resolved, route.reasoning_effort)?
                || state.plan.binding
                    != self.request_binding(&resolved, route.reasoning_effort, saved)?
                || saved
                    .tools
                    .iter()
                    .any(|saved_tool| !request.tools.iter().any(|current| current == saved_tool))
            {
                return Err(ModelProviderError::Configuration("Messages continuation requires its original provider route, reasoning effort and authorized tool schemas".into()));
            }
            let mut replay = saved.clone();
            replay.messages.push(state.assistant.clone());
            let protected_count = replay.messages.len();
            replay
                .messages
                .extend_from_slice(&canonical[state.view.covered_count..]);
            replay
                .messages
                .extend_from_slice(&request.messages[canonical.len()..]);
            (replay, protected_count)
        } else {
            if state.plan.binding
                != self.request_binding(&resolved, route.reasoning_effort, request)?
                || state.plan.context_binding_hash != fingerprint(&Vec::<ModelMessage>::new())?
            {
                return Err(ModelProviderError::Configuration(
                    "legacy Messages replay cannot recover its original prepared request bindings"
                        .into(),
                ));
            }
            let mut replay = request.clone();
            let mut source = canonical[..state.plan.source_count].to_vec();
            if !route.capabilities.image_inputs {
                colossus_tools::omit_unsupported_model_tool_images(&mut source);
            }
            replay.messages =
                colossus_tools::project_model_tool_observations_preserving_current_turn(&source);
            replay.messages.push(state.assistant.clone());
            let protected_count = replay.messages.len();
            replay
                .messages
                .extend_from_slice(&canonical[state.view.covered_count..]);
            replay
                .messages
                .extend_from_slice(&request.messages[canonical.len()..]);
            (replay, protected_count)
        };
        // The frozen prefix already has its text-only image receipts. Newly
        // appended canonical screenshot carriers need that same projection before
        // joining the exact wire exchange; existing receipt slots stay unchanged.
        if !route.capabilities.image_inputs {
            let mut text_only = replay.messages.clone();
            colossus_tools::omit_unsupported_model_tool_images(&mut text_only);
            if text_only[..protected_count] != replay.messages[..protected_count] {
                return Err(ModelProviderError::Configuration("Messages continuation requires its original image input capability and unchanged observation prefix".into()));
            }
            replay.messages[protected_count..].clone_from_slice(&text_only[protected_count..]);
        }
        // New caller ceilings can narrow the frozen request, never expand authority.
        replay.max_output_tokens = match (replay.max_output_tokens, request.max_output_tokens) {
            (Some(saved), Some(current)) => Some(saved.min(current)),
            (saved, None) => saved,
            (None, current) => current,
        };
        if !colossus_tools::model_tool_observations_within_bounds(&replay.messages) {
            return Err(ModelProviderError::Configuration("Messages tool replay requires its unchanged observation prefix; observation or image bounds prevent continuation".into()));
        }
        validate_model_transcript(&replay.messages).map_err(|_| {
            ModelProviderError::Configuration(
                "reconstructed Messages replay has invalid tool pairing".into(),
            )
        })?;
        if retired {
            self.continuations
                .save(state.clone(), context)
                .map_err(|_| {
                    ModelProviderError::Failed("unable to restore verified Messages replay".into())
                })?;
        }
        let plan = ProviderContinuationPlan {
            session_id: session.clone(),
            binding: state.plan.binding.clone(),
            context_binding_hash: state.plan.context_binding_hash.clone(),
            source_count: canonical.len(),
            source_hash: fingerprint(&canonical)?,
            snapshot_epoch: epoch,
            selected: Some(state.view),
        };
        Ok(Some((replay, plan)))
    }

    fn pending_candidate(
        &self,
        context: &ExecutionContext,
    ) -> Result<Option<ProviderContinuation>, ModelProviderError> {
        let Some(session) = &context.session_id else {
            return Ok(None);
        };
        let pending = self
            .pending
            .lock()
            .map_err(|_| ModelProviderError::Failed("provider state lock poisoned".into()))?;
        if pending.keys().any(|(prior_session, prior_run)| {
            prior_session == session && *prior_run != context.run_id
        }) {
            return Err(ModelProviderError::Configuration(
                "provider candidate belongs to another active run".into(),
            ));
        }
        let id = pending
            .get(&(session.clone(), context.run_id.clone()))
            .cloned();
        drop(pending);
        let Some(id) = id else {
            return Ok(None);
        };
        self.continuations
            .candidate(session, &id)
            .map_err(|_| {
                ModelProviderError::Failed("released provider candidate unavailable".into())
            })?
            .map(Some)
            .ok_or_else(|| {
                ModelProviderError::Failed("released provider candidate unavailable".into())
            })
    }

    pub(super) fn admission_events(
        &self,
        context: &ExecutionContext,
    ) -> Result<Vec<NewEvent>, ModelProviderError> {
        let Some(state) = self.pending_candidate(context)? else {
            return Ok(Vec::new());
        };
        self.continuations
            .admission_events(&state.plan.session_id, &state.view.id, context)
            .map_err(|_| {
                ModelProviderError::Failed("unable to prepare protected provider admission".into())
            })
    }

    pub(super) fn settlement_events(
        &self,
        context: &ExecutionContext,
        canonical: &[ModelMessage],
    ) -> Result<Vec<NewEvent>, ModelProviderError> {
        let Some(mut state) = self.pending_candidate(context)? else {
            return Ok(Vec::new());
        };
        if let Err(error) = self.validate_settlement(&mut state, canonical) {
            if state.view.replay {
                return Err(error);
            }
            return Ok(Vec::new());
        }
        self.continuations
            .settlement_events(&state, context)
            .map_err(|_| {
                ModelProviderError::Failed("unable to prepare protected provider settlement".into())
            })
    }

    fn validate_settlement(
        &self,
        state: &mut ProviderContinuation,
        canonical: &[ModelMessage],
    ) -> Result<(), ModelProviderError> {
        if state.view.covered_count > canonical.len()
            || state.plan.source_count >= canonical.len()
            || canonical.get(state.plan.source_count) != Some(&state.assistant)
            || fingerprint(&canonical[..state.plan.source_count].to_vec())?
                != state.plan.source_hash
            || self
                .snapshots
                .activation_epoch(&state.plan.session_id)
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?
                != state.plan.snapshot_epoch
            || validate_model_transcript(canonical).is_err()
        {
            return Err(ModelProviderError::Configuration(
                "provider candidate does not match the complete canonical tool turn".into(),
            ));
        }
        state.settled_hash = fingerprint(&canonical[..state.view.covered_count].to_vec())?;
        Ok(())
    }

    pub(super) fn settle(&self, context: &ExecutionContext) -> Result<(), ModelProviderError> {
        let Some(session) = &context.session_id else {
            return Ok(());
        };
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| ModelProviderError::Failed("provider state lock poisoned".into()))?;
        if pending.keys().any(|(prior_session, prior_run)| {
            prior_session == session && *prior_run != context.run_id
        }) {
            return Err(ModelProviderError::Configuration(
                "provider candidate belongs to another active run".into(),
            ));
        }
        let id = pending.remove(&(session.clone(), context.run_id.clone()));
        drop(pending);
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
        let staged = self
            .continuations
            .take_staged(&id)
            .map_err(|_| ModelProviderError::Failed("unable to settle Responses state".into()))?;
        let mut state = match staged {
            Some(state) => state,
            None => self
                .continuations
                .load(session)
                .map_err(|_| {
                    ModelProviderError::Failed("unable to confirm protected provider state".into())
                })?
                .filter(|state| state.view.id == id)
                .or(self.continuations.candidate(session, &id).map_err(|_| {
                    ModelProviderError::Failed("unable to recover admitted provider state".into())
                })?)
                .ok_or_else(|| {
                    ModelProviderError::Failed(
                        "admitted provider candidate unavailable after tool commit".into(),
                    )
                })?,
        };
        let canonical = self
            .sessions
            .list_messages(session)
            .map_err(|e| ModelProviderError::Failed(e.to_string()))?
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        if let Err(error) = self.validate_settlement(&mut state, &canonical) {
            if state.view.replay {
                return Err(error);
            }
            return Ok(());
        }
        if self
            .continuations
            .load(session)
            .map_err(|_| {
                ModelProviderError::Failed("unable to confirm protected provider settlement".into())
            })?
            .is_some_and(|settled| {
                settled.view.id == state.view.id && settled.settled_hash == state.settled_hash
            })
        {
            return Ok(());
        }
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
