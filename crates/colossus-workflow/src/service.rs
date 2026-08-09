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
    /// Exact tool ceiling inherited from the pinned workflow definition.
    ///
    /// This is authority metadata, not a grant. The runtime still applies normal
    /// tool policy and sandbox checks to every invocation.
    pub allowed_tools: Vec<String>,
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
    pub(super) observability_spans: Mutex<BTreeMap<String, WorkflowObservation>>,
}

#[derive(Clone)]
pub(super) struct WorkflowObservation {
    pub(super) span: tracing::Span,
    pub(super) started: Instant,
    pub(super) prior_elapsed_seconds: f64,
}

pub(super) struct WorkflowObservationLease<'a> {
    owner: &'a Mutex<BTreeMap<String, WorkflowObservation>>,
    run_id: String,
    observation: WorkflowObservation,
    retain: bool,
}

impl WorkflowObservationLease<'_> {
    pub(super) fn span(&self) -> &tracing::Span {
        &self.observation.span
    }

    pub(super) fn elapsed_seconds(&self) -> f64 {
        self.observation.prior_elapsed_seconds + self.observation.started.elapsed().as_secs_f64()
    }

    pub(super) fn retain(&mut self) {
        self.retain = true;
    }
}

impl Drop for WorkflowObservationLease<'_> {
    fn drop(&mut self) {
        if !self.retain {
            let _ = self
                .owner
                .lock()
                .map(|mut observations| observations.remove(&self.run_id));
        }
    }
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
            observability_spans: Mutex::new(BTreeMap::new()),
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
        let observation = WorkflowObservation {
            span: workflow_span(name, run_id, "active"),
            started: Instant::now(),
            prior_elapsed_seconds: 0.0,
        };
        let trace_context = colossus_observability::trace_context_for_span(&observation.span);
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
                "trace_context": trace_context,
            }),
        )?;
        let run = self.get_run(run_id)?;
        self.observability_spans
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?
            .insert(run_id.into(), observation);
        Ok(run)
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
        let mut observation = self.workflow_observation(&run, "active")?;
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
        if let Err(error) = self
            .drive(run_id, definition, current_hash, run.inputs, 0)
            .instrument(observation.span().clone())
            .await
        {
            observation.span().record("otel.status_code", "ERROR");
            observation.span().record("error.type", "workflow.failed");
            colossus_observability::record_workflow(
                &run.workflow_name,
                observation.elapsed_seconds(),
                Some("workflow.failed"),
            );
            return Err(error);
        }
        let result = self.get_run(run_id)?;
        if result.status == WorkflowStatus::Waiting {
            observation.retain();
        } else {
            observation.span().record("otel.status_code", "OK");
            colossus_observability::record_workflow(
                &run.workflow_name,
                observation.elapsed_seconds(),
                None,
            );
        }
        Ok(result)
    }

    pub(super) fn workflow_observation<'a>(
        &'a self,
        run: &WorkflowRun,
        segment: &str,
    ) -> Result<WorkflowObservationLease<'a>, WorkflowError> {
        let existing = self
            .observability_spans
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?
            .get(&run.run_id)
            .cloned();
        let observation = if let Some(existing) = existing {
            existing
        } else {
            let span = workflow_span(&run.workflow_name, &run.run_id, segment);
            let events = self
                .journal
                .read_stream(&format!("workflow-run:{}", run.run_id))?;
            let queued = events
                .iter()
                .find(|event| event.event_type == "workflow.run.queued.v1");
            let trace_context = queued
                .map(|event| self.journal.decrypt_payload(event))
                .transpose()?
                .and_then(|payload| payload.get("trace_context").cloned())
                .and_then(|value| serde_json::from_value(value).ok());
            if let Some(trace_context) = trace_context.as_ref() {
                let _ = colossus_observability::add_remote_link(&span, trace_context);
            }
            let prior_elapsed_seconds = queued
                .and_then(|event| OffsetDateTime::parse(&event.occurred_at, &Rfc3339).ok())
                .map(|started| {
                    (OffsetDateTime::now_utc() - started)
                        .as_seconds_f64()
                        .max(0.0)
                })
                .unwrap_or_default();
            let recovered = WorkflowObservation {
                span,
                started: Instant::now(),
                prior_elapsed_seconds,
            };
            self.observability_spans
                .lock()
                .map_err(|error| StoreError::Adapter(error.to_string()))?
                .insert(run.run_id.clone(), recovered.clone());
            recovered
        };
        Ok(WorkflowObservationLease {
            owner: &self.observability_spans,
            run_id: run.run_id.clone(),
            observation,
            retain: false,
        })
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

fn workflow_span(name: &str, run_id: &str, segment: &str) -> tracing::Span {
    tracing::info_span!(
        target: "colossus.gen_ai",
        "invoke_workflow",
        otel.name = %format_args!("invoke_workflow {name}"),
        otel.kind = "internal",
        otel.status_code = tracing::field::Empty,
        error.type = tracing::field::Empty,
        gen_ai.operation.name = "invoke_workflow",
        gen_ai.workflow.name = name,
        colossus.workflow.run.id = run_id,
        colossus.workflow.segment = segment,
    )
}
