use super::*;

/// Shared automatic and manual context-compaction application service.
pub struct ContextService {
    config: ContextConfig,
    sessions: Arc<dyn SessionRepository>,
    snapshots: Arc<dyn ContextRepository>,
    provider: Arc<dyn ModelProvider>,
    work: Option<Arc<dyn WorkRepository>>,
    memories: Option<Arc<dyn MemoryRetriever>>,
}

impl ContextService {
    fn replay_request_overhead(
        &self,
        view: &colossus_contracts::ProviderContinuationView,
        instructions: &str,
        messages: &[ModelMessage],
        tools: &[ModelToolDefinition],
        context: &ExecutionContext,
    ) -> Result<usize, ContextError> {
        self.provider
            .continuation_request_overhead(
                &ModelRequest {
                    instructions: instructions.into(),
                    messages: messages.to_vec(),
                    tools: tools.to_vec(),
                    max_output_tokens: None,
                },
                view,
                context,
            )
            .map_err(|error| ContextError::Configuration(error.to_string()))
    }

    /// Compose the service from replaceable session, snapshot, and provider ports.
    pub fn new(
        config: ContextConfig,
        sessions: Arc<dyn SessionRepository>,
        snapshots: Arc<dyn ContextRepository>,
        provider: Arc<dyn ModelProvider>,
    ) -> Result<Self, ContextError> {
        config.validate()?;
        Ok(Self {
            config,
            sessions,
            snapshots,
            provider,
            work: None,
            memories: None,
        })
    }

    /// Attach durable key decisions as binding context ahead of snapshots.
    pub fn with_work_repository(mut self, work: Arc<dyn WorkRepository>) -> Self {
        self.work = Some(work);
        self
    }

    /// Attach policy-aware relevant-memory retrieval after binding decisions.
    pub fn with_memory_retriever(mut self, memories: Arc<dyn MemoryRetriever>) -> Self {
        self.memories = Some(memories);
        self
    }

    /// Return budget status for the active canonical session history.
    pub fn status(&self, session_id: &str) -> Result<ContextStatus, ContextError> {
        self.status_for_role(session_id, "primary")
    }

    /// Return budget status using the model profile resolved for one logical role.
    pub fn status_for_role(
        &self,
        session_id: &str,
        role: &str,
    ) -> Result<ContextStatus, ContextError> {
        let budget = self
            .provider
            .route(role)
            .map_err(|error| ContextError::Configuration(error.to_string()))?;
        let records = self.sessions.list_messages(session_id)?;
        let mut messages = records
            .iter()
            .map(|record| record.message.clone())
            .collect::<Vec<_>>();
        if !budget.capabilities.image_inputs {
            colossus_tools::omit_unsupported_model_tool_images(&mut messages);
        }
        let messages = project_model_tool_observations(&messages);
        let binding = self.decision_message()?;
        let original_messages =
            prepend_bindings(binding.clone().into_iter().collect(), messages.clone());
        let raw = estimate_tokens_for_model(&budget.model, "", &original_messages, &[]);
        let active = self.snapshots.active(session_id)?;
        let prepared = active.as_ref().map_or_else(
            || messages.clone(),
            |snapshot| apply_snapshot(snapshot, &messages),
        );
        let mut prepared = prepend_bindings(binding.into_iter().collect(), prepared);
        if active.is_some() {
            bound_summary_to_target(
                &budget.model,
                "",
                &mut prepared,
                &[],
                self.config.target_tokens(budget.limits.input_budget_tokens),
            );
            bound_summary_to_byte_limit("", &mut prepared, &[], MAX_PREPARED_MODEL_REQUEST_BYTES);
        }
        Ok(ContextStatus {
            session_id: session_id.into(),
            message_count: records.len().try_into().unwrap_or(u64::MAX),
            raw_token_estimate: raw,
            token_estimate: estimate_tokens_for_model(&budget.model, "", &prepared, &[]),
            model_profile: budget.model_profile,
            context_window_tokens: budget.limits.context_window_tokens,
            max_output_tokens: budget.limits.max_output_tokens,
            safety_margin_tokens: budget.limits.safety_margin_tokens,
            input_budget_tokens: budget.limits.input_budget_tokens,
            threshold_tokens: self
                .config
                .threshold_tokens(budget.limits.input_budget_tokens),
            target_tokens: self.config.target_tokens(budget.limits.input_budget_tokens),
            active_snapshot_id: active.as_ref().map(|snapshot| snapshot.id.clone()),
            compacted: active.is_some(),
            auto_compaction: self.config.auto_compaction,
        })
    }

    /// List immutable snapshots in creation order.
    pub fn list_snapshots(&self, session_id: &str) -> Result<Vec<ContextSnapshot>, ContextError> {
        self.snapshots.list(session_id).map_err(Into::into)
    }

    /// Explicitly restore an older snapshot for future provider turns.
    pub fn restore(
        &self,
        session_id: &str,
        snapshot_id: &str,
    ) -> Result<ContextSnapshot, ContextError> {
        self.restore_as(session_id, snapshot_id, user_actor())
    }

    /// Explicitly restore a snapshot with immutable caller provenance.
    pub fn restore_as(
        &self,
        session_id: &str,
        snapshot_id: &str,
        actor: Actor,
    ) -> Result<ContextSnapshot, ContextError> {
        self.snapshots
            .activate(session_id, snapshot_id, actor)
            .map_err(Into::into)
    }

    /// Force a new snapshot even when the automatic threshold is not crossed.
    pub async fn compact(
        &self,
        session_id: &str,
        instructions: &str,
        tools: &[ModelToolDefinition],
    ) -> Result<PreparedContext, ContextError> {
        self.compact_for_role_with_context(
            session_id,
            "primary",
            instructions,
            tools,
            ExecutionContext {
                correlation_id: Uuid::now_v7().to_string(),
                session_id: Some(session_id.into()),
                ..ExecutionContext::default()
            },
        )
        .await
    }

    /// Force a snapshot while retaining the initiating execution provenance.
    pub async fn compact_with_context(
        &self,
        session_id: &str,
        instructions: &str,
        tools: &[ModelToolDefinition],
        context: ExecutionContext,
    ) -> Result<PreparedContext, ContextError> {
        self.compact_for_role_with_context(session_id, "primary", instructions, tools, context)
            .await
    }

    /// Force a snapshot using the budget resolved for one logical model role.
    pub async fn compact_for_role_with_context(
        &self,
        session_id: &str,
        role: &str,
        instructions: &str,
        tools: &[ModelToolDefinition],
        context: ExecutionContext,
    ) -> Result<PreparedContext, ContextError> {
        let budget = self
            .provider
            .route(role)
            .map_err(|error| ContextError::Configuration(error.to_string()))?;
        let records = self.sessions.list_messages(session_id)?;
        if records.is_empty() {
            return Err(ContextError::Configuration(format!(
                "session has no messages: {session_id}"
            )));
        }
        let messages = records.into_iter().map(|record| record.message).collect();
        self.prepare(ContextPreparationRequest {
            continuation: None,
            session_id: session_id.into(),
            instructions: instructions.into(),
            messages,
            tools: tools.to_vec(),
            route: budget,
            context,
            force: true,
        })
        .await
    }

    async fn draft_snapshot(
        &self,
        mut snapshot: ContextSnapshot,
        source: &[ModelMessage],
        context: ExecutionContext,
    ) -> Result<ContextSnapshot, ContextError> {
        if source.is_empty() {
            return Err(ContextError::Configuration(
                "cannot compact an empty message range".into(),
            ));
        }
        if self.config.model_assisted
            && self
                .provider
                .route("context_summarizer")
                .is_ok_and(|route| route.provider != "echo")
            && let Some(summary) = self.model_summary(source, context).await
        {
            snapshot.summary = truncate_bytes(&summary, MAX_SUMMARY_BYTES);
            snapshot.strategy = "hybrid_model".into();
        }
        Ok(snapshot)
    }

    fn decision_message(&self) -> Result<Option<ModelMessage>, ContextError> {
        let Some(work) = &self.work else {
            return Ok(None);
        };
        // The work repository is already isolated to the selected workspace state.
        // Keep the originating session on each record for provenance, but apply its
        // active commitments to every session in that workspace.
        let mut decisions = work.list_decisions(None, Some(DecisionStatus::Active), 1_000)?;
        decisions.sort_by_key(|decision| match decision.priority {
            DecisionPriority::Critical => 0,
            DecisionPriority::High => 1,
            DecisionPriority::Normal => 2,
        });
        if decisions.is_empty() {
            return Ok(None);
        }
        let mut content = String::from(
            "[Binding active workspace key decisions]\nApply these durable commitments when their conditions hold unless the current user explicitly supersedes them. They are stronger than summaries and memories.\n",
        );
        for decision in decisions {
            let item = decision_line(&decision);
            if content.len().saturating_add(item.len()) > MAX_DECISION_CONTEXT_BYTES {
                content.push_str("- Additional active decisions omitted from this bounded context block; inspect canonical decision state before changing established commitments.\n");
                break;
            }
            content.push_str(&item);
        }
        Ok(Some(ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::System,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }))
    }

    async fn binding_messages(
        &self,
        session_id: &str,
        messages: &[ModelMessage],
        context: ExecutionContext,
    ) -> Result<Vec<ModelMessage>, ContextError> {
        let mut bindings = Vec::new();
        if let Some(decision) = self.decision_message()? {
            bindings.push(decision);
        }
        let query = messages
            .iter()
            .rev()
            .find(|message| message.begins_user_turn())
            .map(|message| message.content.plain_text())
            .unwrap_or_default();
        if !query.trim().is_empty()
            && let Some(retriever) = &self.memories
        {
            let records = retriever.relevant(&query, session_id, context, 6).await?;
            if let Some(memory) = memory_message(&records) {
                bindings.push(memory);
            }
        }
        Ok(bindings)
    }

    async fn model_summary(
        &self,
        source: &[ModelMessage],
        context: ExecutionContext,
    ) -> Option<String> {
        let route = self.provider.route("context_summarizer").ok()?;
        let request = bounded_summary_request(source, route.limits.input_budget_tokens)?;
        let turn = self
            .provider
            .turn("context_summarizer", request, context)
            .await
            .ok()?;
        let final_text = turn.events.iter().rev().find_map(|event| match event {
            ProviderEvent::FinalOutput { text } if !text.trim().is_empty() => Some(text.trim()),
            _ => None,
        });
        final_text.map(str::to_owned).or_else(|| {
            let text = turn
                .events
                .iter()
                .filter_map(|event| match event {
                    ProviderEvent::ModelDelta { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();
            (!text.trim().is_empty()).then(|| text.trim().to_owned())
        })
    }
}

fn bounded_summary_request(
    source: &[ModelMessage],
    input_budget_tokens: u64,
) -> Option<ModelRequest> {
    const PREFIX: &str = "Compact this Colossus session history into durable future context.\n\n";
    const OMITTED: &str = "[Additional source messages omitted to fit the context-summarizer model's effective input budget; deterministic metadata still covers the complete source range.]\n";

    let request_for = |history: &str| ModelRequest {
        instructions: SUMMARY_INSTRUCTIONS.into(),
        messages: vec![ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::User,
            content: format!("{PREFIX}{history}").into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        tools: Vec::new(),
        max_output_tokens: None,
    };
    let fits = |request: &ModelRequest| {
        request.messages[0].content.text_bytes() <= MAX_SUMMARY_PROMPT_BYTES
            && estimate_tokens(&request.instructions, &request.messages, &request.tools)
                <= input_budget_tokens
    };

    let base = request_for("");
    if !fits(&base) {
        return None;
    }

    let mut history = String::new();
    let mut omitted = false;
    for (index, message) in source.iter().enumerate() {
        let item = format!(
            "{}. {:?}: {}\n\n",
            index + 1,
            message.role,
            truncate_chars(&message.content.plain_text(), 1_000)
        );
        let checkpoint = history.len();
        history.push_str(&item);
        if !fits(&request_for(&history)) {
            history.truncate(checkpoint);
            omitted = true;
            break;
        }
    }

    if omitted {
        let checkpoint = history.len();
        history.push_str(OMITTED);
        if !fits(&request_for(&history)) {
            history.truncate(checkpoint);
        }
    }

    let request = request_for(&history);
    fits(&request).then_some(request)
}

#[async_trait]
impl ContextPreparer for ContextService {
    async fn prepare_replay(
        &self,
        request: ContextPreparationRequest,
    ) -> Result<PreparedContext, ContextError> {
        let view = request
            .continuation
            .filter(|view| view.replay)
            .ok_or_else(|| {
                ContextError::Configuration(
                    "unfinished provider replay reference is missing".into(),
                )
            })?;
        if request.force {
            return Err(ContextError::Configuration(
                "an unfinished provider exchange cannot be compacted".into(),
            ));
        }
        if !colossus_tools::model_tool_observations_within_bounds(&request.messages) {
            return Err(ContextError::Configuration(
                "Messages tool replay requires its unchanged observation prefix; observation or image bounds prevent continuation".into(),
            ));
        }
        validate_newest_image_turn(&request.messages)?;
        let tokens = estimate_tokens_for_model(
            &request.route.model,
            &request.instructions,
            &request.messages,
            &request.tools,
        )
        .saturating_add(view.reserved_tokens);
        let overhead = self.replay_request_overhead(
            &view,
            &request.instructions,
            &request.messages,
            &request.tools,
            &request.context,
        )?;
        let bytes = model_request_bytes(&request.instructions, &request.messages, &request.tools)
            .saturating_add(overhead);
        if tokens > request.route.limits.input_budget_tokens {
            return Err(ContextBudgetExceeded::tokens(
                tokens,
                request.route.limits.input_budget_tokens,
                ContextBudgetScope::PreservedMessages,
            )
            .into());
        }
        if bytes > MAX_PREPARED_MODEL_REQUEST_BYTES {
            return Err(ContextBudgetExceeded::request_bytes(
                bytes,
                MAX_PREPARED_MODEL_REQUEST_BYTES,
                ContextBudgetScope::PreservedMessages,
            )
            .into());
        }
        Ok(PreparedContext {
            context_binding_hash: view.context_binding_hash,
            continuation_id: Some(view.id),
            messages: request.messages,
            token_estimate: tokens,
            original_token_estimate: tokens,
            model_profile: request.route.model_profile,
            context_window_tokens: request.route.limits.context_window_tokens,
            max_output_tokens: request.route.limits.max_output_tokens,
            safety_margin_tokens: request.route.limits.safety_margin_tokens,
            input_budget_tokens: request.route.limits.input_budget_tokens,
            threshold_tokens: self
                .config
                .threshold_tokens(request.route.limits.input_budget_tokens),
            target_tokens: self
                .config
                .target_tokens(request.route.limits.input_budget_tokens),
            snapshot_id: None,
            compacted: false,
            snapshot_created: false,
            strategy: Some("anthropic_replay".into()),
        })
    }

    async fn prepare_tool_evidence(
        &self,
        request: ContextPreparationRequest,
    ) -> Result<PreparedContext, ContextError> {
        if request.continuation.is_some() || !request.tools.is_empty() || request.force {
            return Err(ContextError::Configuration(
                "tool evidence recovery requires a fresh tool-free request".into(),
            ));
        }
        match self.prepare_context(request.clone(), true).await {
            Err(ContextError::BudgetExceeded(error)) => {
                self.prepare_bounded_tool_evidence(request, error)
            }
            result => result,
        }
    }

    async fn prepare(
        &self,
        request: ContextPreparationRequest,
    ) -> Result<PreparedContext, ContextError> {
        self.prepare_context(request, false).await
    }
}

impl ContextService {
    fn prepare_bounded_tool_evidence(
        &self,
        request: ContextPreparationRequest,
        mut failure: ContextBudgetExceeded,
    ) -> Result<PreparedContext, ContextError> {
        let ContextPreparationRequest {
            session_id,
            instructions,
            messages,
            route,
            ..
        } = request;
        let active = self.snapshots.active(&session_id)?;
        let cutoff = active
            .as_ref()
            .map_or(0, |snapshot| {
                usize::try_from(snapshot.source_end_sequence).unwrap_or(usize::MAX)
            })
            .min(messages.len());
        let suffix = &messages[cutoff..];
        let Some(newest_input) = suffix
            .iter()
            .rposition(ModelMessage::begins_user_turn)
            .or_else(|| {
                suffix
                    .iter()
                    .rposition(|message| message.role == ModelMessageRole::User)
            })
        else {
            return Err(failure.into());
        };
        let Some(evidence_end) = last_tool_evidence(suffix).map(|index| index + 1) else {
            return Err(failure.into());
        };
        let evidence = &suffix[..evidence_end];
        let evidence_start = evidence
            .iter()
            .rposition(ModelMessage::begins_user_turn)
            .unwrap_or(0);
        let prior_prompt = (evidence_start < newest_input
            && evidence[evidence_start].begins_user_turn())
        .then_some(&evidence[evidence_start].content);
        let saved = SavedToolEvidence::collect(&evidence[evidence_start..], prior_prompt);
        let newest_guidance = suffix[newest_input..]
            .iter()
            .filter(|message| message.role == ModelMessageRole::User)
            .cloned()
            .collect::<Vec<_>>();
        // Tool images are represented by omission counts. Current user input
        // keeps its exact attachments and still obeys the existing image bound.
        validate_newest_image_turn(&newest_guidance)?;
        let bindings = self.decision_message()?.into_iter().collect::<Vec<_>>();
        let context_binding_hash = context_binding_hash(&bindings)?;
        let threshold = self
            .config
            .threshold_tokens(route.limits.input_budget_tokens);
        let target = self.config.target_tokens(route.limits.input_budget_tokens);
        let original = estimate_tokens_for_model(&route.model, &instructions, &messages, &[]);
        for with_previews in [true, false] {
            let mut prepared = bindings.clone();
            if let Some(snapshot) = &active {
                let mut prefix = snapshot_message(snapshot, &messages[..cutoff]);
                prefix.role = ModelMessageRole::User;
                prepared.push(prefix);
            }
            prepared.push(saved.to_message(with_previews));
            prepared.extend(newest_guidance.iter().cloned());
            if active.is_some() {
                bound_summary_to_target(&route.model, &instructions, &mut prepared, &[], target);
                bound_summary_to_byte_limit(
                    &instructions,
                    &mut prepared,
                    &[],
                    MAX_PREPARED_MODEL_REQUEST_BYTES,
                );
            }
            let token_estimate =
                estimate_tokens_for_model(&route.model, &instructions, &prepared, &[]);
            let bytes = model_request_bytes(&instructions, &prepared, &[]);
            if bytes > MAX_PREPARED_MODEL_REQUEST_BYTES {
                failure = ContextBudgetExceeded::request_bytes(
                    bytes,
                    MAX_PREPARED_MODEL_REQUEST_BYTES,
                    ContextBudgetScope::CompactedRequest,
                );
                continue;
            }
            if token_estimate > route.limits.input_budget_tokens {
                failure = ContextBudgetExceeded::tokens(
                    token_estimate,
                    route.limits.input_budget_tokens,
                    ContextBudgetScope::CompactedRequest,
                );
                continue;
            }
            return Ok(PreparedContext {
                context_binding_hash,
                continuation_id: None,
                messages: prepared,
                token_estimate,
                original_token_estimate: original,
                model_profile: route.model_profile,
                context_window_tokens: route.limits.context_window_tokens,
                max_output_tokens: route.limits.max_output_tokens,
                safety_margin_tokens: route.limits.safety_margin_tokens,
                input_budget_tokens: route.limits.input_budget_tokens,
                threshold_tokens: threshold,
                target_tokens: target,
                snapshot_id: active.as_ref().map(|snapshot| snapshot.id.clone()),
                compacted: true,
                snapshot_created: false,
                strategy: Some("bounded_tool_evidence_recovery".into()),
            });
        }
        Err(failure.into())
    }

    async fn prepare_context(
        &self,
        request: ContextPreparationRequest,
        tool_evidence: bool,
    ) -> Result<PreparedContext, ContextError> {
        let ContextPreparationRequest {
            continuation,
            session_id,
            instructions,
            messages,
            tools,
            route: budget,
            context,
            force,
        } = request;
        if force
            && (continuation.as_ref().is_some_and(|view| view.replay)
                || (budget.provider == "anthropic_messages" && unfinished_tool_exchange(&messages)))
        {
            return Err(ContextError::Configuration(
                "Messages tool replay cannot be compacted before the assistant finishes; finish or restart the turn first".into(),
            ));
        }
        let mut messages = messages;
        if !budget.capabilities.image_inputs {
            colossus_tools::omit_unsupported_model_tool_images(&mut messages);
        }
        validate_newest_image_turn(&messages)?;
        let messages = compact_excess_images(&messages);
        let messages = if tool_evidence {
            project_recovery_observations(&messages)
        } else if continuation.as_ref().is_some_and(|view| view.replay) {
            colossus_tools::project_model_tool_observations_preserving_current_turn(&messages)
        } else {
            project_model_tool_observations(&messages)
        };
        let bindings = if tool_evidence {
            // Recovery can also run as preflight, before accepting human input.
            // Bind decisions without starting a journaled memory retrieval effect.
            self.decision_message()?.into_iter().collect()
        } else {
            self.binding_messages(&session_id, &messages, context.clone())
                .await?
        };
        let context_binding_hash = context_binding_hash(&bindings)?;
        // Apply snapshots to the original slots before changing the provider-facing roles.
        let with_snapshot = |snapshot: &ContextSnapshot| {
            let mut prepared = apply_snapshot(snapshot, &messages);
            if tool_evidence {
                prepared[0].role = ModelMessageRole::User;
            }
            prepared
        };
        let bind_request = |bindings, messages| {
            let messages = if tool_evidence {
                quote_tool_evidence(messages)
            } else {
                messages
            };
            prepend_bindings(bindings, messages)
        };
        let recovery_strategy = tool_evidence.then(|| "tool_evidence_recovery".to_owned());
        let original_messages = bind_request(bindings.clone(), messages.clone());
        let original =
            estimate_tokens_for_model(&budget.model, &instructions, &original_messages, &tools);
        let original_bytes = model_request_bytes(&instructions, &original_messages, &tools);
        let threshold = self
            .config
            .threshold_tokens(budget.limits.input_budget_tokens);
        let target = self.config.target_tokens(budget.limits.input_budget_tokens);
        let active = self.snapshots.active(&session_id)?;
        let active_messages = active
            .as_ref()
            .map_or_else(|| messages.clone(), with_snapshot);
        let mut active_messages = bind_request(bindings.clone(), active_messages);
        if active.is_some() {
            bound_summary_to_target(
                &budget.model,
                &instructions,
                &mut active_messages,
                &tools,
                target,
            );
            bound_summary_to_byte_limit(
                &instructions,
                &mut active_messages,
                &tools,
                MAX_PREPARED_MODEL_REQUEST_BYTES,
            );
        }
        let active_estimate =
            estimate_tokens_for_model(&budget.model, &instructions, &active_messages, &tools);
        let active_bytes = model_request_bytes(&instructions, &active_messages, &tools);
        if !force
            && let Some(view) = continuation.filter(|view| {
                view.covered_count <= messages.len()
                    && view.context_binding_hash == context_binding_hash
            })
        {
            if view.replay {
                let estimate = active_estimate.saturating_add(view.reserved_tokens);
                let overhead = self.replay_request_overhead(
                    &view,
                    &instructions,
                    &active_messages,
                    &tools,
                    &context,
                )?;
                if estimate > budget.limits.input_budget_tokens
                    || active_bytes.saturating_add(overhead) > MAX_PREPARED_MODEL_REQUEST_BYTES
                {
                    return Err(ContextError::Configuration("Messages tool exchange exceeds its replay budget; finish or restart the turn before compacting".into()));
                }
                return Ok(PreparedContext {
                    context_binding_hash,
                    continuation_id: Some(view.id),
                    messages: active_messages,
                    token_estimate: estimate,
                    original_token_estimate: original,
                    model_profile: budget.model_profile.clone(),
                    context_window_tokens: budget.limits.context_window_tokens,
                    max_output_tokens: budget.limits.max_output_tokens,
                    safety_margin_tokens: budget.limits.safety_margin_tokens,
                    input_budget_tokens: budget.limits.input_budget_tokens,
                    threshold_tokens: threshold,
                    target_tokens: target,
                    snapshot_id: active.as_ref().map(|snapshot| snapshot.id.clone()),
                    compacted: active.is_some(),
                    snapshot_created: false,
                    strategy: Some("anthropic_replay".into()),
                });
            }
            // Settle tool calls represented by the opaque prefix before adding
            // new developer bindings; preserve result ordering and exact IDs.
            let mut suffix = messages[view.covered_count..].to_vec();
            let after_results = suffix
                .iter()
                .take_while(|message| {
                    matches!(
                        message.role,
                        ModelMessageRole::Tool | ModelMessageRole::ToolObservation
                    )
                })
                .count();
            suffix.splice(after_results..after_results, bindings.clone());
            let estimate = estimate_tokens_for_model(&budget.model, &instructions, &suffix, &tools)
                .saturating_add(view.reserved_tokens);
            let bytes =
                model_request_bytes(&instructions, &suffix, &tools).saturating_add(view.bytes);
            if estimate <= threshold.min(budget.limits.input_budget_tokens)
                && bytes <= MAX_PREPARED_MODEL_REQUEST_BYTES
            {
                return Ok(PreparedContext {
                    context_binding_hash,
                    continuation_id: Some(view.id),
                    messages: suffix,
                    token_estimate: estimate,
                    original_token_estimate: original,
                    model_profile: budget.model_profile.clone(),
                    context_window_tokens: budget.limits.context_window_tokens,
                    max_output_tokens: budget.limits.max_output_tokens,
                    safety_margin_tokens: budget.limits.safety_margin_tokens,
                    input_budget_tokens: budget.limits.input_budget_tokens,
                    threshold_tokens: threshold,
                    target_tokens: target,
                    snapshot_id: active.as_ref().map(|snapshot| snapshot.id.clone()),
                    compacted: true,
                    snapshot_created: false,
                    strategy: Some("responses_server".into()),
                });
            }
        }
        let newest_turn_start = messages
            .iter()
            .rposition(ModelMessage::begins_user_turn)
            .unwrap_or(0);
        let latest_evidence_start = if tool_evidence {
            last_tool_evidence(&messages).map(|last_result| {
                messages[..last_result]
                    .iter()
                    .rposition(ModelMessage::begins_user_turn)
                    .unwrap_or(0)
            })
        } else {
            None
        };
        let protected_start = latest_evidence_start.unwrap_or(newest_turn_start);
        let active_over_budget = active_estimate > budget.limits.input_budget_tokens
            || active_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES;
        let should_create = force
            || (self.config.auto_compaction
                // A recovery request must honor a retained snapshot's cutoff.
                && (!tool_evidence || active.is_none())
                && (original > threshold || original_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES)
                && (active.is_none()
                    || active_estimate > threshold
                    || active_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES));
        let mut source_end = 0;
        if should_create {
            let preserve = self.config.preserve_recent_messages.min(messages.len());
            source_end = messages.len().saturating_sub(preserve);
            if preserve > 0 {
                while source_end > 0 && !messages[source_end].begins_user_turn() {
                    source_end -= 1;
                }
            }
            if force && source_end == 0 {
                source_end = messages.len();
            }
            if !force {
                source_end = source_end.min(protected_start);
                if source_end == 0 && active_over_budget {
                    source_end = protected_start;
                }
            }
        }
        if source_end == 0 {
            let scope = if should_create && active.is_none() {
                ContextBudgetScope::NewestTurn
            } else {
                ContextBudgetScope::PreparedRequest
            };
            let token_error = (active_estimate > budget.limits.input_budget_tokens).then(|| {
                ContextBudgetExceeded::tokens(
                    active_estimate,
                    budget.limits.input_budget_tokens,
                    scope,
                )
            });
            let byte_error = (active_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES).then(|| {
                ContextBudgetExceeded::request_bytes(
                    active_bytes,
                    MAX_PREPARED_MODEL_REQUEST_BYTES,
                    scope,
                )
            });
            let budget_error = if should_create {
                byte_error.or(token_error)
            } else {
                token_error.or(byte_error)
            };
            if let Some(error) = budget_error {
                return Err(error.into());
            }
            return Ok(PreparedContext {
                context_binding_hash,
                continuation_id: None,
                messages: active_messages,
                token_estimate: active_estimate,
                original_token_estimate: original,
                model_profile: budget.model_profile.clone(),
                context_window_tokens: budget.limits.context_window_tokens,
                max_output_tokens: budget.limits.max_output_tokens,
                safety_margin_tokens: budget.limits.safety_margin_tokens,
                input_budget_tokens: budget.limits.input_budget_tokens,
                threshold_tokens: threshold,
                target_tokens: target,
                snapshot_id: active.as_ref().map(|snapshot| snapshot.id.clone()),
                compacted: active.is_some(),
                snapshot_created: false,
                strategy: recovery_strategy.or_else(|| active.map(|snapshot| snapshot.strategy)),
            });
        }
        let snapshot_candidate = |source_end| -> Result<_, ContextBudgetExceeded> {
            let preserved = bind_request(bindings.clone(), messages[source_end..].to_vec());
            let preserved_bytes = model_request_bytes(&instructions, &preserved, &tools);
            if preserved_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES {
                return Err(ContextBudgetExceeded::request_bytes(
                    preserved_bytes,
                    MAX_PREPARED_MODEL_REQUEST_BYTES,
                    ContextBudgetScope::PreservedMessages,
                ));
            }
            let preserved_estimate =
                estimate_tokens_for_model(&budget.model, &instructions, &preserved, &tools);
            if preserved_estimate.saturating_add(64) > budget.limits.input_budget_tokens {
                return Err(ContextBudgetExceeded::tokens(
                    preserved_estimate.saturating_add(64),
                    budget.limits.input_budget_tokens,
                    ContextBudgetScope::SnapshotEnvelope,
                ));
            }
            let snapshot = deterministic_snapshot(&session_id, &messages[..source_end]);
            let mut minimum_snapshot = snapshot.clone();
            minimum_snapshot.summary = "...".into();
            let minimum_prepared = bind_request(bindings.clone(), with_snapshot(&minimum_snapshot));
            let minimum_bytes = model_request_bytes(&instructions, &minimum_prepared, &tools);
            if minimum_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES {
                return Err(ContextBudgetExceeded::request_bytes(
                    minimum_bytes,
                    MAX_PREPARED_MODEL_REQUEST_BYTES,
                    ContextBudgetScope::SnapshotEnvelope,
                ));
            }
            let minimum_estimate =
                estimate_tokens_for_model(&budget.model, &instructions, &minimum_prepared, &tools);
            if minimum_estimate > budget.limits.input_budget_tokens {
                return Err(ContextBudgetExceeded::tokens(
                    minimum_estimate,
                    budget.limits.input_budget_tokens,
                    ContextBudgetScope::SnapshotEnvelope,
                ));
            }
            Ok(snapshot)
        };
        let draft_snapshot = match snapshot_candidate(source_end) {
            Err(_) if !force && source_end < protected_start => {
                // The preferred recent window did not leave enough hard-budget
                // headroom. Compact only whole turns before the protected one.
                source_end = protected_start;
                snapshot_candidate(source_end)?
            }
            candidate => candidate?,
        };
        let actor = context_actor(&context);
        let snapshot = if tool_evidence {
            // This projection is only for the recovery request. Keep the snapshot
            // transient and use deterministic compaction without auxiliary inference.
            draft_snapshot
        } else {
            self.draft_snapshot(draft_snapshot, &messages[..source_end], context)
                .await?
        };
        let mut prepared = bind_request(bindings, with_snapshot(&snapshot));
        bound_summary_to_target(&budget.model, &instructions, &mut prepared, &tools, target);
        bound_summary_to_byte_limit(
            &instructions,
            &mut prepared,
            &tools,
            MAX_PREPARED_MODEL_REQUEST_BYTES,
        );
        let estimate = estimate_tokens_for_model(&budget.model, &instructions, &prepared, &tools);
        if estimate > budget.limits.input_budget_tokens {
            return Err(ContextBudgetExceeded::tokens(
                estimate,
                budget.limits.input_budget_tokens,
                ContextBudgetScope::CompactedRequest,
            )
            .into());
        }
        let prepared_bytes = model_request_bytes(&instructions, &prepared, &tools);
        if prepared_bytes > MAX_PREPARED_MODEL_REQUEST_BYTES {
            return Err(ContextBudgetExceeded::request_bytes(
                prepared_bytes,
                MAX_PREPARED_MODEL_REQUEST_BYTES,
                ContextBudgetScope::CompactedRequest,
            )
            .into());
        }
        let (snapshot_id, strategy) = if tool_evidence {
            (None, recovery_strategy)
        } else {
            let snapshot = self.snapshots.create(snapshot, actor)?;
            (Some(snapshot.id), Some(snapshot.strategy))
        };
        Ok(PreparedContext {
            context_binding_hash,
            continuation_id: None,
            messages: prepared,
            token_estimate: estimate,
            original_token_estimate: original,
            model_profile: budget.model_profile,
            context_window_tokens: budget.limits.context_window_tokens,
            max_output_tokens: budget.limits.max_output_tokens,
            safety_margin_tokens: budget.limits.safety_margin_tokens,
            input_budget_tokens: budget.limits.input_budget_tokens,
            threshold_tokens: threshold,
            target_tokens: target,
            snapshot_id,
            compacted: true,
            snapshot_created: !tool_evidence,
            strategy,
        })
    }
}

fn context_binding_hash(bindings: &[ModelMessage]) -> Result<String, ContextError> {
    let bytes = serde_json::to_vec(bindings)
        .map_err(|_| ContextError::Configuration("unable to bind context messages".into()))?;
    Ok(hex::encode(<sha2::Sha256 as sha2::Digest>::digest(bytes)))
}

fn unfinished_tool_exchange(messages: &[ModelMessage]) -> bool {
    let start = messages
        .iter()
        .rposition(ModelMessage::begins_user_turn)
        .unwrap_or(0);
    messages[start..]
        .iter()
        .rev()
        .find(|message| message.role == ModelMessageRole::Assistant)
        .is_some_and(|message| !message.tool_calls.is_empty())
}
