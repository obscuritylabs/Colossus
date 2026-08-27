use super::*;

/// Explicit controls for one provider turn.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProviderTurnOptions {
    /// Capture a bounded non-success response for a trusted local diagnostic surface.
    pub include_response_diagnostics: bool,
}

/// Exact transient image bytes resolved only inside a permit-bearing provider adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRunInputImage {
    /// Verified durable metadata bound into the provider effect and permit.
    pub reference: ModelImageReference,
    /// Exact verified artifact bytes. Callers must never persist or diagnose this field.
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
