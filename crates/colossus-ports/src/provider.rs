use super::*;

/// Explicit controls for one provider turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderTurnOptions {
    /// Capture a bounded non-success response for a trusted local diagnostic surface.
    pub include_response_diagnostics: bool,
    /// Safe continuation reference selected by context preparation.
    pub continuation: Option<colossus_contracts::ProviderContinuationPlan>,
    /// Configured per-step model preferences, bound into the provider effect.
    pub agent_options: colossus_contracts::WorkflowAgentOptions,
}

/// Exact transient image bytes resolved only inside a permit-bearing provider adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRunInputImage {
    /// Verified durable metadata bound into the provider effect and permit.
    pub reference: ModelImageReference,
    /// Exact verified artifact bytes. Callers must never persist or diagnose this field.
    pub bytes: Vec<u8>,
}

/// Exact transient document bytes, resolved only inside the generation permit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRunInputFile {
    /// Immutable metadata bound into the authorized provider request.
    pub reference: colossus_contracts::ModelFileReference,
    /// Exact verified bytes; never persist or include in diagnostics.
    pub bytes: Vec<u8>,
}

/// Safe failure from the application-owned encrypted run-input media service.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RunInputMediaError {
    /// The artifact was unavailable, not owner-authorized, or did not match its reference.
    #[error("run-input image is unavailable or does not match its verified reference")]
    Unavailable,
    /// The artifact bytes did not satisfy the bounded image contract.
    #[error("run-input image failed bounded validation: {0}")]
    Invalid(String),
}

/// Late-bound resolver for encrypted run-input image artifacts.
#[async_trait]
pub trait RunInputMediaResolver: Send + Sync {
    /// Resolve and re-verify an already owner-authorized document reference.
    async fn resolve_file(
        &self,
        _reference: &colossus_contracts::ModelFileReference,
    ) -> Result<ResolvedRunInputFile, RunInputMediaError> {
        Err(RunInputMediaError::Unavailable)
    }

    /// Resolve and re-verify exact bytes after the provider permit has been issued.
    async fn resolve_image(
        &self,
        reference: &ModelImageReference,
    ) -> Result<ResolvedRunInputImage, RunInputMediaError>;
}

/// Role-routed, policy-bound model provider used by the application loop.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Resolve role metadata without performing an effect.
    fn route(&self, role: &str) -> Result<ModelRoute, ModelProviderError>;

    /// Resolve explicit configured model preferences; unsupported adapters reject overrides.
    fn route_with_options(
        &self,
        role: &str,
        options: &colossus_contracts::WorkflowAgentOptions,
    ) -> Result<ModelRoute, ModelProviderError> {
        if options != &colossus_contracts::WorkflowAgentOptions::default() {
            return Err(ModelProviderError::Configuration(
                "model overrides are unsupported by this adapter".into(),
            ));
        }
        self.route(role)
    }

    /// Resolve safe continuation metadata against the exact canonical history.
    fn continuation_plan(
        &self,
        _role: &str,
        _request: &ModelRequest,
        _context: &ExecutionContext,
    ) -> Result<Option<colossus_contracts::ProviderContinuationPlan>, ModelProviderError> {
        Ok(None)
    }

    /// Resolve continuation metadata for the same model preferences used by the turn.
    /// Adapters without an override-aware planner retain role-default planning only.
    fn continuation_plan_with_options(
        &self,
        role: &str,
        request: &ModelRequest,
        context: &ExecutionContext,
        options: &colossus_contracts::WorkflowAgentOptions,
    ) -> Result<Option<colossus_contracts::ProviderContinuationPlan>, ModelProviderError> {
        if options == &colossus_contracts::WorkflowAgentOptions::default() {
            self.continuation_plan(role, request, context)
        } else {
            Ok(None)
        }
    }

    /// Promote private state only after the assistant and tool results are durable.
    fn settle_continuation(&self, _context: &ExecutionContext) -> Result<(), ModelProviderError> {
        Ok(())
    }
    /// Private released-candidate writes to commit with the pending tool marker.
    fn tool_turn_admission_events(
        &self,
        _context: &ExecutionContext,
    ) -> Result<Vec<NewEvent>, ModelProviderError> {
        Ok(Vec::new())
    }
    /// Private settlement writes to commit with the complete canonical tool transcript.
    fn tool_turn_settlement_events(
        &self,
        _context: &ExecutionContext,
        _canonical_after: &[ModelMessage],
    ) -> Result<Vec<NewEvent>, ModelProviderError> {
        Ok(Vec::new())
    }
    /// Finalize process-local candidate custody after the tool transaction commits.
    fn tool_turn_committed(&self, context: &ExecutionContext) -> Result<(), ModelProviderError> {
        self.settle_continuation(context)
    }
    /// Release only process-local candidates owned by a finished or dropped run.
    /// Durable admitted effects and canonical continuation state remain intact.
    fn finish_run_candidates(&self, _run_id: &str) {}
    /// Rebuild an unfinished provider exchange with newly appended human guidance.
    fn replay_request_with_options(
        &self,
        _role: &str,
        _request: &ModelRequest,
        _context: &ExecutionContext,
        _options: &colossus_contracts::WorkflowAgentOptions,
    ) -> Result<
        Option<(ModelRequest, colossus_contracts::ProviderContinuationPlan)>,
        ModelProviderError,
    > {
        Ok(None)
    }
    /// Conservatively budget private continuation bytes absent from this exact request.
    /// Implementations may exclude duplicated canonical metadata only after verifying
    /// the selected private state. Persisted state sizes and token reservations stay
    /// unchanged; adapters without that proof retain the complete byte reservation.
    fn continuation_request_overhead(
        &self,
        _request: &ModelRequest,
        view: &colossus_contracts::ProviderContinuationView,
        _context: &ExecutionContext,
    ) -> Result<usize, ModelProviderError> {
        Ok(view.bytes)
    }
    /// Abandon uncommitted output after failure, retaining required canonical replay.
    fn discard_continuation(&self, _context: &ExecutionContext) -> Result<(), ModelProviderError> {
        Ok(())
    }

    /// Execute one normalized provider turn through the effect boundary.
    async fn turn(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError>;

    /// Execute one provider turn while observing safe events as they are released.
    async fn turn_stream(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        observer: &mut dyn ProviderEventObserver,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let turn = self.turn(role, request, context).await?;
        for event in &turn.events {
            observer.observe(event.clone()).await?;
        }
        Ok(turn)
    }

    /// Execute one provider turn with explicit local diagnostic controls.
    async fn turn_stream_with_options(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        options: ProviderTurnOptions,
        observer: &mut dyn ProviderEventObserver,
    ) -> Result<ProviderTurn, ModelProviderError> {
        if options.agent_options != colossus_contracts::WorkflowAgentOptions::default() {
            return Err(ModelProviderError::Configuration(
                "model overrides are unsupported by this adapter".into(),
            ));
        }
        if options.include_response_diagnostics {
            return Err(ModelProviderError::Configuration(
                "provider response diagnostics are unsupported by this adapter".into(),
            ));
        }
        self.turn_stream(role, request, context, observer).await
    }
}

/// Role-routed, policy-bound provider-neutral web search.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// Resolve route metadata without performing an external effect.
    fn route(&self, role: &str) -> Result<SearchRoute, SearchError>;

    /// Return safe configured profile summaries without resolving credentials.
    fn profiles(&self) -> Vec<SearchProfileSummary>;

    /// Execute one normalized search through the effect boundary.
    async fn search(
        &self,
        role: &str,
        actor: Actor,
        request: SearchRequest,
        context: ExecutionContext,
    ) -> Result<SearchResponse, SearchError>;
}

/// Application observer for provider events released through policy.
#[async_trait]
pub trait ProviderEventObserver: Send {
    /// Persist or render one safe ordered event.
    async fn observe(&mut self, event: ProviderEvent) -> Result<(), ModelProviderError>;
}

/// Application observer for ordered policy-released provider and harness activity.
#[async_trait]
pub trait RunEventObserver: Send {
    /// Render or transport one safe event after its authoritative event is durable.
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError>;
}
