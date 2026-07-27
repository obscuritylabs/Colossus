use super::*;

pub(super) struct GatewayModelProvider {
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) providers: Arc<ProviderRegistry>,
}

pub(super) struct GatewaySearchProvider {
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) searches: Arc<SearchRegistry>,
}

pub(super) fn search_gateway_error(error: GatewayError) -> SearchError {
    match error {
        GatewayError::Denied(message) | GatewayError::Approval(message) => {
            SearchError::Denied(message)
        }
        GatewayError::OutcomeUnknown(message) => SearchError::OutcomeUnknown(message),
        GatewayError::Safety(message) => SearchError::Configuration(message),
        error => SearchError::Failed(error.to_string()),
    }
}

#[async_trait]
impl SearchProvider for GatewaySearchProvider {
    fn route(&self, role: &str) -> Result<SearchRoute, SearchError> {
        let profile = self
            .searches
            .resolve(role)
            .map_err(|error| SearchError::Unavailable(error.to_string()))?;
        Ok(SearchRoute {
            role: role.into(),
            profile: profile.profile().name().into(),
            provider: profile.profile().kind().as_str().into(),
        })
    }

    fn profiles(&self) -> Vec<SearchProfileSummary> {
        self.searches.profiles()
    }

    async fn search(
        &self,
        role: &str,
        actor: Actor,
        request: SearchRequest,
        context: ExecutionContext,
    ) -> Result<SearchResponse, SearchError> {
        let executor = self
            .searches
            .resolve(role)
            .map_err(|error| SearchError::Unavailable(error.to_string()))?;
        let profile = executor.profile();
        let mut effect = effect_request(
            actor,
            "web.search",
            profile.endpoint(),
            serde_json::to_value(SearchEffectInput {
                profile: profile.name().into(),
                request,
            })
            .map_err(|error| SearchError::Configuration(error.to_string()))?,
        );
        effect.capabilities = vec!["web.search".into()];
        effect.context = context;
        effect.credential_references = profile.credential_reference().into_iter().collect();
        let released = self
            .gateway
            .execute(effect, executor.as_ref())
            .await
            .map_err(search_gateway_error)?;
        serde_json::from_slice(&released.bytes).map_err(|error| {
            SearchError::Failed(format!("invalid normalized search output: {error}"))
        })
    }
}

pub(super) struct GatewayRiskEvaluator {
    pub(super) provider: Arc<dyn ModelProvider>,
}

fn strict_risk_assessment_json(output: &str) -> Result<&str, RiskEvaluationError> {
    let output = output.trim();
    if !output.starts_with("```") {
        return Ok(output);
    }
    let (opening, fenced) = output.split_once('\n').ok_or_else(|| {
        RiskEvaluationError::InvalidAssessment("fenced risk assessment has no JSON body".into())
    })?;
    if !matches!(opening, "```json" | "```JSON" | "```") {
        return Err(RiskEvaluationError::InvalidAssessment(
            "risk assessment used an unsupported code fence".into(),
        ));
    }
    let fenced = fenced.strip_suffix("```").ok_or_else(|| {
        RiskEvaluationError::InvalidAssessment("fenced risk assessment is not terminated".into())
    })?;
    let fenced = fenced.trim();
    if fenced.is_empty() || fenced.contains("```") {
        return Err(RiskEvaluationError::InvalidAssessment(
            "fenced risk assessment must contain exactly one JSON document".into(),
        ));
    }
    Ok(fenced)
}

pub(super) fn redacted_risk_metadata(
    request: &EffectRequest,
    decision: &colossus_contracts::PolicyDecision,
) -> Value {
    let mut content = request.content.clone();
    if let Some(object) = content.as_object_mut() {
        if let Some(environment) = object.remove("environment") {
            let names = environment
                .as_object()
                .map(|values| values.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            object.insert("environment_names".into(), json!(names));
        }
        if let Some(stdin) = object.remove("stdin_base64")
            && let Some(stdin) = stdin.as_str()
        {
            object.insert(
                "stdin".into(),
                json!({
                    "present": true,
                    "encoded_size": stdin.len(),
                    "sha256": hex::encode(Sha256::digest(stdin.as_bytes())),
                }),
            );
        }
        if let Some(arguments) = object.get_mut("args").and_then(Value::as_array_mut) {
            let mut redact_next = false;
            for argument in arguments {
                let Some(value) = argument.as_str() else {
                    continue;
                };
                if redact_next {
                    *argument = Value::String("[REDACTED]".into());
                    redact_next = false;
                    continue;
                }
                let lower = value.to_ascii_lowercase();
                let sensitive = [
                    "password",
                    "passwd",
                    "token",
                    "secret",
                    "api-key",
                    "apikey",
                    "authorization",
                ]
                .iter()
                .any(|marker| lower.contains(marker));
                if sensitive {
                    if let Some((name, _)) = value.split_once('=') {
                        *argument = Value::String(format!("{name}=[REDACTED]"));
                    } else {
                        redact_next = true;
                    }
                }
            }
        }
    }
    json!({
        "action": request.action,
        "resource": request.resource,
        "capabilities": request.capabilities,
        "actor": request.actor,
        "workflow_id": request.context.workflow_id,
        "workflow_hash": request.context.workflow_hash,
        "step_id": request.context.step_id,
        "attempt": request.context.attempt,
        "policy_decision_id": decision.decision_id,
        "policy_revision": decision.policy_revision,
        "policy_reason": decision.reason,
        "proposed_effect": content,
    })
}

#[async_trait]
impl RiskEvaluator for GatewayRiskEvaluator {
    async fn evaluate(
        &self,
        request: &EffectRequest,
        decision: &colossus_contracts::PolicyDecision,
    ) -> Result<RiskAssessment, RiskEvaluationError> {
        self.provider
            .route("risk_evaluator")
            .map_err(|error| RiskEvaluationError::Unavailable(error.to_string()))?;
        let metadata = redacted_risk_metadata(request, decision);
        let prompt = serde_json::to_string(&metadata)
            .map_err(|error| RiskEvaluationError::InvalidAssessment(error.to_string()))?;
        let turn = self
            .provider
            .turn(
                "risk_evaluator",
                ModelRequest {
                    instructions: concat!(
                        "Assess the proposed effect conservatively. Return only one JSON object with exactly ",
                        "risk_level (low, medium, or high), recommended_decision (allow, deny, or require_approval), ",
                        "and reason (a short non-secret explanation). Do not use tools or Markdown. Ordinary read-only ",
                        "web searches and bodyless HTTP GET requests to configured destinations may be low risk. Treat ",
                        "uncertainty, sensitive disclosure, destructive operations, credential access, privilege changes, ",
                        "persistence, non-read-only network methods, or broad network/file impact as requiring approval or denial."
                    )
                    .into(),
                    messages: vec![ModelMessage {
                        role: ModelMessageRole::User,
                        content: prompt,
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    }],
                    tools: Vec::new(),
                    max_output_tokens: None,
                },
                request.context.clone(),
            )
            .await
            .map_err(|error| RiskEvaluationError::Unavailable(error.to_string()))?;
        let output = turn
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                ProviderEvent::FinalOutput { text } => Some(text.trim()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                RiskEvaluationError::InvalidAssessment(
                    "provider returned no final JSON output".into(),
                )
            })?;
        let assessment =
            serde_json::from_str::<RiskAssessment>(strict_risk_assessment_json(output)?)
                .map_err(|error| RiskEvaluationError::InvalidAssessment(error.to_string()))?;
        if assessment.reason.trim().is_empty() || assessment.reason.chars().count() > 1_000 {
            return Err(RiskEvaluationError::InvalidAssessment(
                "reason must contain 1 to 1000 characters".into(),
            ));
        }
        Ok(assessment)
    }
}

impl GatewayModelProvider {
    async fn turn_with_options(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        options: ProviderTurnOptions,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let resolved = self
            .providers
            .resolve(role)
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        let route = resolved.route();
        let provider = resolved.provider();
        let max_output_tokens = resolved_output_limit(&route, &request)?;
        let endpoint = provider
            .profile()
            .generation_endpoint()
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        let mut effect = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            provider.profile().kind.generation_action(),
            endpoint,
            serde_json::to_value(ProviderEffectInput {
                provider_profile: route.provider_profile,
                model_profile: Some(route.model_profile),
                model: Some(route.model),
                max_output_tokens: Some(max_output_tokens),
                request: Some(request),
                include_response_diagnostics: options.include_response_diagnostics,
            })
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?,
        );
        effect.capabilities = vec!["provider.call".into()];
        effect.context = context;
        effect.credential_references = provider.credential_reference().into_iter().collect();
        let released = self
            .gateway
            .execute(effect, provider.as_ref())
            .await
            .map_err(model_gateway_error)?;
        if options.include_response_diagnostics
            && let Ok(diagnostic) =
                serde_json::from_slice::<ProviderResponseDiagnostic>(&released.bytes)
        {
            return Err(ModelProviderError::ResponseDiagnostic {
                diagnostic: Box::new(diagnostic),
            });
        }
        serde_json::from_slice(&released.bytes).map_err(|_| {
            ModelProviderError::Failed(
                "released provider output violated the normalized turn contract".into(),
            )
        })
    }
}

#[async_trait]
impl ModelProvider for GatewayModelProvider {
    fn route(&self, role: &str) -> Result<ModelRoute, ModelProviderError> {
        let resolved = self
            .providers
            .resolve(role)
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        Ok(resolved.route())
    }

    async fn turn(
        &self,
        role: &str,
        request: colossus_contracts::ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.turn_with_options(role, request, context, ProviderTurnOptions::default())
            .await
    }

    async fn turn_stream(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        observer: &mut dyn ProviderEventObserver,
    ) -> Result<ProviderTurn, ModelProviderError> {
        self.turn_stream_with_options(
            role,
            request,
            context,
            ProviderTurnOptions::default(),
            observer,
        )
        .await
    }

    async fn turn_stream_with_options(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        options: ProviderTurnOptions,
        observer: &mut dyn ProviderEventObserver,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let route = self.route(role)?;
        if !route.capabilities.streaming {
            let turn = self
                .turn_with_options(role, request, context, options)
                .await?;
            for event in &turn.events {
                observer.observe(event.clone()).await?;
            }
            return Ok(turn);
        }
        let resolved = self
            .providers
            .resolve(role)
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        let route = resolved.route();
        let provider = resolved.provider();
        let max_output_tokens = resolved_output_limit(&route, &request)?;
        let endpoint = provider
            .profile()
            .generation_endpoint()
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?;
        let mut effect = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            provider.profile().kind.generation_action(),
            endpoint,
            serde_json::to_value(ProviderEffectInput {
                provider_profile: route.provider_profile,
                model_profile: Some(route.model_profile),
                model: Some(route.model),
                max_output_tokens: Some(max_output_tokens),
                request: Some(request),
                include_response_diagnostics: options.include_response_diagnostics,
            })
            .map_err(|error| ModelProviderError::Configuration(error.to_string()))?,
        );
        effect.capabilities = vec!["provider.call".into()];
        effect.context = context;
        effect.credential_references = provider.credential_reference().into_iter().collect();
        let mut bridge = ReleasedProviderStream::new(observer);
        let terminal = self
            .gateway
            .execute_stream(effect, provider.as_ref(), &mut bridge)
            .await
            .map_err(model_gateway_error)?;
        bridge.finish(&terminal.bytes, options.include_response_diagnostics)
    }
}

pub(super) struct ReleasedProviderStream<'a> {
    pub(super) observer: &'a mut dyn ProviderEventObserver,
    pub(super) events: Vec<ProviderEvent>,
    pub(super) completed: Option<(String, String, String, String, Option<String>)>,
    pub(super) diagnostic: Option<ProviderResponseDiagnostic>,
}

impl<'a> ReleasedProviderStream<'a> {
    fn new(observer: &'a mut dyn ProviderEventObserver) -> Self {
        Self {
            observer,
            events: Vec::new(),
            completed: None,
            diagnostic: None,
        }
    }

    fn finish(
        self,
        terminal: &[u8],
        include_response_diagnostics: bool,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let expected: ProviderStreamItem = serde_json::from_slice(terminal).map_err(|_| {
            ModelProviderError::Failed(
                "released provider stream terminal violated its contract".into(),
            )
        })?;
        if let ProviderStreamItem::Diagnostic { diagnostic } = expected {
            if include_response_diagnostics
                && self.events.is_empty()
                && self.completed.is_none()
                && self.diagnostic.as_ref() == Some(&diagnostic)
            {
                return Err(ModelProviderError::ResponseDiagnostic {
                    diagnostic: Box::new(diagnostic),
                });
            }
            return Err(ModelProviderError::Failed(
                "released provider stream diagnostic violated its contract".into(),
            ));
        }
        let ProviderStreamItem::Completed {
            profile,
            model_profile,
            provider_profile,
            provider,
            model,
            response_id,
        } = expected
        else {
            return Err(ModelProviderError::Failed(
                "released provider stream did not end with completion metadata".into(),
            ));
        };
        if profile != model_profile
            || self.completed.as_ref()
                != Some(&(
                    model_profile.clone(),
                    provider_profile.clone(),
                    provider.clone(),
                    model.clone(),
                    response_id.clone(),
                ))
        {
            return Err(ModelProviderError::Failed(
                "released provider stream completion metadata did not match".into(),
            ));
        }
        Ok(ProviderTurn {
            profile,
            model_profile,
            provider_profile,
            provider,
            model,
            response_id,
            events: self.events,
        })
    }
}

#[async_trait]
impl ReleasedEffectObserver for ReleasedProviderStream<'_> {
    async fn observe(&mut self, result: ReleasedEffectResult) -> Result<(), ExecutionError> {
        let item: ProviderStreamItem = serde_json::from_slice(&result.bytes).map_err(|_| {
            ExecutionError::Failed("released provider stream item violated its contract".into())
        })?;
        match item {
            ProviderStreamItem::Event { event } => {
                self.observer
                    .observe(event.clone())
                    .await
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?;
                self.events.push(event);
            }
            ProviderStreamItem::Diagnostic { diagnostic } => {
                if self.diagnostic.is_some() || self.completed.is_some() || !self.events.is_empty()
                {
                    return Err(ExecutionError::Failed(
                        "provider stream diagnostic violated its terminal contract".into(),
                    ));
                }
                self.diagnostic = Some(diagnostic);
            }
            ProviderStreamItem::Completed {
                profile,
                model_profile,
                provider_profile,
                provider,
                model,
                response_id,
            } => {
                if self.completed.is_some() {
                    return Err(ExecutionError::Failed(
                        "provider stream completed more than once".into(),
                    ));
                }
                if profile != model_profile {
                    return Err(ExecutionError::Failed(
                        "provider stream compatibility profile did not match model profile".into(),
                    ));
                }
                self.completed = Some((
                    model_profile,
                    provider_profile,
                    provider,
                    model,
                    response_id,
                ));
            }
        }
        Ok(())
    }
}

fn resolved_output_limit(
    route: &ModelRoute,
    request: &ModelRequest,
) -> Result<u64, ModelProviderError> {
    if !route.capabilities.tool_calls
        && (!request.tools.is_empty()
            || request.messages.iter().any(|message| {
                message.role == ModelMessageRole::Tool || !message.tool_calls.is_empty()
            }))
    {
        return Err(ModelProviderError::Configuration(format!(
            "model profile {} does not support tool calls or structured tool history",
            route.model_profile
        )));
    }
    match request.max_output_tokens {
        Some(0) => Err(ModelProviderError::Configuration(
            "max_output_tokens must be greater than zero".into(),
        )),
        Some(limit) if limit > route.limits.max_output_tokens => {
            Err(ModelProviderError::Configuration(format!(
                "requested output limit {limit} exceeds model profile {} maximum {}",
                route.model_profile, route.limits.max_output_tokens
            )))
        }
        Some(limit) => Ok(limit),
        None => Ok(route.limits.max_output_tokens),
    }
}

pub(super) fn model_gateway_error(error: GatewayError) -> ModelProviderError {
    match error {
        GatewayError::RecoverableExecution {
            code,
            message,
            http_status,
            retry_after_ms,
        } => ModelProviderError::Recoverable {
            code,
            message,
            http_status,
            retry_after_ms,
        },
        GatewayError::HttpStatus { status, message } => {
            ModelProviderError::HttpStatus { status, message }
        }
        GatewayError::OutcomeUnknown(message) => ModelProviderError::OutcomeUnknown(message),
        error => ModelProviderError::Failed(error.to_string()),
    }
}
