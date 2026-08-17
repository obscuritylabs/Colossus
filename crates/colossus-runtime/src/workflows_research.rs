use super::*;

/// Caller authority and conversation ownership captured for one Research execution.
#[derive(Clone, Debug)]
pub struct ResearchRunContext {
    /// Authenticated actor responsible for the Research operation.
    pub actor: Actor,
    /// Exact caller-owned tool ceiling for internal evidence collectors.
    pub allowed_tools: Vec<String>,
    /// Optional public run that owns the synthesized assistant message.
    pub message_run_id: Option<String>,
}

fn research_source_tool(kind: ResearchSourceKind) -> &'static str {
    match kind {
        ResearchSourceKind::Repo => "filesystem.search",
        ResearchSourceKind::Web => "web.search",
        ResearchSourceKind::Mcp => "mcp.call",
    }
}

fn research_source_tools(source_kinds: &[ResearchSourceKind]) -> Vec<String> {
    source_kinds
        .iter()
        .copied()
        .map(research_source_tool)
        .map(str::to_owned)
        .collect()
}

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
        let allowed_tools = research_source_tools(&source_kinds);
        self.run_research_as(
            session_id,
            question,
            depth,
            source_kinds,
            ResearchRunContext {
                actor: terminal_actor(),
                allowed_tools,
                message_run_id: None,
            },
        )
        .await
    }

    /// Run bounded durable research with an exact tool ceiling and message owner.
    pub async fn run_research_as(
        &self,
        session_id: &str,
        question: &str,
        depth: ResearchDepth,
        source_kinds: Vec<ResearchSourceKind>,
        context: ResearchRunContext,
    ) -> Result<ResearchRun, RuntimeError> {
        let operation = ResearchOperation::Run {
            session_id: session_id.into(),
            question: question.into(),
            depth,
            source_kinds,
            message_run_id: context.message_run_id.clone(),
        };
        let mut request = effect_request(
            context.actor,
            operation.action(),
            format!("session:{session_id}"),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        request.context.session_id = Some(session_id.into());
        request.context.run_id = context.message_run_id;
        request.context.offered_tools = context.allowed_tools;
        let result = self
            .gateway
            .execute(request, self.research_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
