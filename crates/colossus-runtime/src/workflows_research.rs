use super::*;

impl Runtime {
    /// Durable workflow application API.
    pub fn workflows(&self) -> Arc<WorkflowService> {
        Arc::clone(&self.workflows)
    }

    /// Exact workflow definition repository for list/show surfaces.
    pub fn workflow_repository(&self) -> Arc<dyn WorkflowRepository> {
        Arc::clone(&self.workflow_repository)
    }

    /// Resolve a persisted webhook credential reference at the last responsible moment and ingest.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_workflow_webhook(
        &self,
        webhook_id: &str,
        delivery_id: &str,
        timestamp: &str,
        signature: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<WorkflowWebhookDispatch, RuntimeError> {
        let webhook = self.workflows.get_webhook(webhook_id)?;
        let variable = webhook
            .secret_reference
            .strip_prefix("env:")
            .ok_or_else(|| {
                RuntimeError::Config("webhook credential reference is invalid".into())
            })?;
        let secret = std::env::var(variable).map_err(|_| {
            RuntimeError::Config(format!(
                "webhook credential environment variable {variable} is unavailable"
            ))
        })?;
        self.workflows
            .ingest_webhook(
                webhook_id,
                delivery_id,
                timestamp,
                signature,
                headers,
                body,
                secret.as_bytes(),
            )
            .await
            .map_err(Into::into)
    }

    /// Current session snapshots served by the disposable session projection.
    pub fn session_repository(&self) -> Arc<dyn SessionRepository> {
        Arc::clone(&self.sessions)
    }

    /// Canonical research repository for embedded read-only inspection surfaces.
    pub fn research_repository(&self) -> Arc<dyn ResearchRepository> {
        Arc::clone(&self.research)
    }

    /// Reconstruct one canonical research run.
    pub fn get_research_run(&self, id: &str) -> Result<Option<ResearchRun>, RuntimeError> {
        self.research.get_run(id).map_err(Into::into)
    }

    /// List bounded canonical research runs.
    pub fn list_research_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResearchRun>, RuntimeError> {
        self.research
            .list_runs(session_id, limit)
            .map_err(Into::into)
    }

    /// List canonical evidence sources for one run.
    pub fn research_sources(&self, run_id: &str) -> Result<Vec<ResearchSource>, RuntimeError> {
        self.research.list_sources(run_id).map_err(Into::into)
    }

    /// List canonical source-backed claims for one run.
    pub fn research_claims(&self, run_id: &str) -> Result<Vec<ResearchClaim>, RuntimeError> {
        self.research.list_claims(run_id).map_err(Into::into)
    }

    /// Run bounded durable research through the policy gateway.
    pub async fn run_research(
        &self,
        session_id: &str,
        question: &str,
        depth: ResearchDepth,
        source_kinds: Vec<ResearchSourceKind>,
    ) -> Result<ResearchRun, RuntimeError> {
        let operation = ResearchOperation::Run {
            session_id: session_id.into(),
            question: question.into(),
            depth,
            source_kinds,
        };
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            format!("session:{session_id}"),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        request.context.session_id = Some(session_id.into());
        let result = self
            .gateway
            .execute(request, self.research_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
