use super::*;

/// Policy-controlled workflow effect request handed to the runtime gateway.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowEffect {
    /// Effect class (`agent`, `tool`, or `workflow`).
    pub kind: String,
    /// Action or registered tool/workflow name.
    pub action: String,
    /// Proposed logical content.
    pub content: Value,
    /// Optional explicit idempotency strategy.
    pub idempotency: Option<String>,
    /// Late-bound credential references whose values are deliberately absent.
    pub credential_references: Vec<CredentialReference>,
    /// Workflow run identifier.
    pub run_id: String,
    /// Workflow step identifier.
    pub step_id: String,
    /// Static step identifier from the pinned definition.
    pub definition_step_id: String,
    /// Pinned definition hash.
    pub workflow_hash: String,
    /// One-based attempt number.
    pub attempt: u32,
    /// Whether this is an explicit compensation effect.
    pub compensation: bool,
}

/// Application/runtime bridge that routes every effectful step through the gateway.
#[async_trait]
pub trait WorkflowEffectRunner: Send + Sync {
    /// Run one policy-controlled step and return structured released output.
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError>;
}

/// Durable workflow application API.
pub struct WorkflowService {
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) repository: Arc<dyn WorkflowRepository>,
    pub(super) effects: Arc<dyn WorkflowEffectRunner>,
    pub(super) event_writer: Mutex<()>,
}

impl WorkflowService {
    /// Compose the event-sourced service.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        repository: Arc<dyn WorkflowRepository>,
        effects: Arc<dyn WorkflowEffectRunner>,
    ) -> Self {
        Self {
            journal,
            repository,
            effects,
            event_writer: Mutex::new(()),
        }
    }

    /// Validate and register an exact YAML definition and provenance.
    pub fn register_definition(
        &self,
        yaml: &str,
        provenance: &str,
    ) -> Result<ValidatedWorkflow, WorkflowError> {
        let validated = validate_definition(yaml)?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        validate_call_graph(self.repository.as_ref(), &validated.definition, false)?;
        self.repository
            .register(&validated.definition, &validated.content_hash, provenance)?;
        Ok(validated)
    }

    /// Queue a validated, hash-pinned run for a worker or embedded caller.
    pub fn queue_run(
        &self,
        name: &str,
        version: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        self.queue_run_with_lineage(
            &Uuid::now_v7().to_string(),
            name,
            version,
            inputs,
            None,
            None,
            None,
            1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn queue_run_with_lineage(
        &self,
        run_id: &str,
        name: &str,
        version: &str,
        inputs: Value,
        parent_run_id: Option<&str>,
        parent_step_id: Option<&str>,
        parent_execution_id: Option<&str>,
        call_depth: u16,
    ) -> Result<WorkflowRun, WorkflowError> {
        if usize::from(call_depth) > MAX_WORKFLOW_CALL_DEPTH {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow call depth exceeds {MAX_WORKFLOW_CALL_DEPTH}"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(name, version)?
            .ok_or_else(|| WorkflowError::NotFound(format!("{name}:{version}")))?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        validate_instance(&definition.inputs, &inputs, "input")?;
        self.append_run_event(
            run_id,
            "workflow.run.queued.v1",
            json!({
                "workflow_name": name,
                "workflow_version": version,
                "workflow_hash": workflow_hash,
                "inputs": inputs,
                "parent_run_id": parent_run_id,
                "parent_step_id": parent_step_id,
                "parent_execution_id": parent_execution_id,
                "call_depth": call_depth,
            }),
        )?;
        self.get_run(run_id)
    }

    /// Start and drive a run until it waits or reaches a terminal state.
    pub async fn start_run(
        &self,
        name: &str,
        version: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        let queued = self.queue_run(name, version, inputs)?;
        self.run_queued(&queued.run_id).await
    }

    pub(super) async fn run_queued(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if run.status != WorkflowStatus::Queued {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is not queued"
            )));
        }
        let (definition, current_hash) = self
            .repository
            .definition(&run.workflow_name, &run.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(run.workflow_name.clone()))?;
        if current_hash != run.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "workflow definition changed; queued run trust is invalid".into(),
            ));
        }
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        self.append_run_event(
            run_id,
            "workflow.run.started.v1",
            json!({"from_status": "queued"}),
        )?;
        self.drive(run_id, definition, current_hash, run.inputs, 0)
            .await?;
        self.get_run(run_id)
    }

    /// Reconstruct one durable run.
    pub fn get_run(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        self.repository
            .run(run_id)?
            .ok_or_else(|| WorkflowError::NotFound(run_id.into()))
    }

    /// List bounded durable runs.
    pub fn list_runs(&self, limit: usize) -> Result<Vec<WorkflowRun>, WorkflowError> {
        self.repository.runs(limit).map_err(Into::into)
    }
}
