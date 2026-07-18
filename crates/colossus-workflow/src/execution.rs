use super::*;

impl WorkflowService {
    /// Supply structured input to a waiting run and resume it.
    pub async fn provide_input(
        &self,
        run_id: &str,
        input: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if run.status != WorkflowStatus::Waiting {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is not waiting"
            )));
        }
        let (definition, current_hash) = self
            .repository
            .definition(&run.workflow_name, &run.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(run.workflow_name.clone()))?;
        if current_hash != run.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "workflow definition changed; pinned run trust is invalid".into(),
            ));
        }
        let root_index = usize::try_from(run.completed_steps)
            .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
        let waiting_step_id = run.waiting_step_id.clone().ok_or_else(|| {
            WorkflowError::InvalidTransition("waiting step id is absent from the journal".into())
        })?;
        let waiting_execution_id = run
            .waiting_execution_id
            .clone()
            .unwrap_or_else(|| waiting_step_id.clone());
        let step = find_step(&definition.steps, &waiting_step_id).ok_or_else(|| {
            WorkflowError::InvalidTransition("waiting step is outside the definition".into())
        })?;
        match step {
            WorkflowStep::WaitForInput { schema, .. } => {
                validate_instance(schema, &input, "workflow input response")?;
            }
            WorkflowStep::Approval { .. } => {
                let approved = input == Value::Bool(true)
                    || input.get("approved").and_then(Value::as_bool) == Some(true);
                if !approved {
                    return Err(WorkflowError::InvalidTransition(
                        "approval input must explicitly contain approved: true".into(),
                    ));
                }
            }
            _ => {
                return Err(WorkflowError::InvalidTransition(
                    "the waiting root step does not accept operator input".into(),
                ));
            }
        }
        self.append_run_event(
            run_id,
            "workflow.input.provided.v1",
            json!({
                "step_id": step_id(step),
                "execution_id": waiting_execution_id,
                "input": input.clone(),
            }),
        )?;
        let is_root = definition.steps.get(root_index).is_some_and(|root| {
            step_id(root) == step_id(step) && waiting_execution_id == step_id(step)
        });
        let mut completion = json!({
            "step_id": step_id(step),
            "execution_id": waiting_execution_id,
            "output": input,
        });
        if is_root {
            completion["root_index"] = json!(root_index);
        }
        self.append_run_event(run_id, "workflow.step.completed.v1", completion)?;
        self.resume_run(run_id).await
    }

    /// Resume a waiting or interrupted run without silently retrying completed steps.
    pub async fn resume_run(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if !matches!(
            run.status,
            WorkflowStatus::Waiting | WorkflowStatus::Interrupted
        ) {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is not resumable"
            )));
        }
        let (definition, current_hash) = self
            .repository
            .definition(&run.workflow_name, &run.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(run.workflow_name.clone()))?;
        if current_hash != run.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "workflow definition changed; pinned run trust is invalid".into(),
            ));
        }
        if run.status == WorkflowStatus::Interrupted {
            let events = self
                .journal
                .read_stream(&format!("workflow-run:{run_id}"))?;
            let uncertain = events
                .iter()
                .rev()
                .find(|event| event.event_type == "workflow.step.outcome_unknown.v1")
                .map(|event| self.journal.decrypt_payload(event))
                .transpose()?;
            let uncertain_retryable = uncertain
                .as_ref()
                .and_then(|payload| payload.get("retry_allowed").and_then(Value::as_bool));
            if uncertain_retryable == Some(false) {
                return Err(WorkflowError::InvalidTransition(
                    "unknown non-idempotent effect cannot be retried by resume".into(),
                ));
            }
            if let Some(execution_id) = uncertain.as_ref().and_then(|payload| {
                payload
                    .get("execution_id")
                    .or_else(|| payload.get("step_id"))
                    .and_then(Value::as_str)
            }) && let Some(linked) = self.linked_child(run_id, execution_id)?
                && self
                    .repository
                    .run(&linked.run_id)?
                    .is_some_and(|child| child.status == WorkflowStatus::Interrupted)
            {
                return Err(WorkflowError::InvalidTransition(format!(
                    "interrupted child workflow {} must be resumed before parent {run_id}",
                    linked.run_id
                )));
            }
        }
        self.append_run_event(
            run_id,
            "workflow.run.resumed.v1",
            json!({"from_status": run.status}),
        )?;
        self.drive(
            run_id,
            definition,
            current_hash,
            run.inputs,
            run.completed_steps,
        )
        .await?;
        self.get_run(run_id)
    }

    /// Cancel a non-terminal run. Compensation, if configured later, is separate.
    pub fn cancel_run(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if matches!(
            run.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        ) {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is terminal"
            )));
        }
        if let Some(child_run_id) = run.waiting_child_run_id.as_deref()
            && let Ok(child) = self.get_run(child_run_id)
            && !matches!(
                child.status,
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
            )
        {
            self.cancel_run(child_run_id)?;
        }
        self.append_run_event(
            run_id,
            "workflow.run.cancelled.v1",
            json!({"reason": "operator requested cancellation"}),
        )?;
        self.get_run(run_id)
    }

    /// Mark abandoned running runs interrupted during startup recovery.
    pub fn recover_interrupted(&self) -> Result<Vec<WorkflowRun>, WorkflowError> {
        let running = self
            .list_runs(usize::MAX)?
            .into_iter()
            .filter(|run| run.status == WorkflowStatus::Running)
            .collect::<Vec<_>>();
        let mut recovered = Vec::with_capacity(running.len());
        for run in running {
            let events = self
                .journal
                .read_stream(&format!("workflow-run:{}", run.run_id))?;
            let latest_started = events.iter().enumerate().rev().find(|(_, event)| {
                matches!(
                    event.event_type.as_str(),
                    "workflow.step.started.v1" | "workflow.compensation.step.started.v1"
                )
            });
            if let Some((started_index, started_event)) = latest_started {
                let compensation =
                    started_event.event_type == "workflow.compensation.step.started.v1";
                let started = self.journal.decrypt_payload(started_event)?;
                let step_id = started
                    .get("step_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let execution_id = started
                    .get("execution_id")
                    .and_then(Value::as_str)
                    .unwrap_or(step_id);
                let completed_after = events[started_index.saturating_add(1)..]
                    .iter()
                    .filter(|event| {
                        matches!(
                            event.event_type.as_str(),
                            "workflow.step.completed.v1"
                                | "workflow.compensation.step.completed.v1"
                        )
                    })
                    .map(|event| self.journal.decrypt_payload(event))
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .any(|payload| {
                        payload
                            .get("execution_id")
                            .or_else(|| payload.get("step_id"))
                            .and_then(Value::as_str)
                            == Some(execution_id)
                    });
                if completed_after {
                    self.append_run_event(
                        &run.run_id,
                        "workflow.run.interrupted.v1",
                        json!({"reason": "startup found an abandoned run after a completed step"}),
                    )?;
                    recovered.push(self.get_run(&run.run_id)?);
                    continue;
                }
                // Compensation requires its own explicit operator path. Resuming the primary
                // sequence after an uncertain compensation would execute the wrong phase, so it
                // remains fail-closed even when the compensation declares idempotency.
                let retry_allowed = !compensation
                    && self
                        .repository
                        .definition(&run.workflow_name, &run.workflow_version)?
                        .and_then(|(definition, _)| {
                            find_step(&definition.steps, step_id).map(step_retryable)
                        })
                        .unwrap_or(false);
                self.append_run_event(
                    &run.run_id,
                    "workflow.step.outcome_unknown.v1",
                    json!({
                        "phase": if compensation { "compensation" } else { "primary" },
                        "step_id": step_id,
                        "execution_id": execution_id,
                        "attempt": started.get("attempt").cloned().unwrap_or(Value::Null),
                        "retry_allowed": retry_allowed,
                        "reason": "startup found an abandoned step attempt",
                    }),
                )?;
            }
            self.append_run_event(
                &run.run_id,
                "workflow.run.interrupted.v1",
                json!({"reason": "startup found an abandoned running attempt"}),
            )?;
            recovered.push(self.get_run(&run.run_id)?);
        }
        Ok(recovered)
    }

    /// Drain queued work without resuming waiting or interrupted attempts.
    pub async fn drain(&self) -> Result<Vec<WorkflowRun>, WorkflowError> {
        let queued = self
            .list_runs(usize::MAX)?
            .into_iter()
            .filter(|run| run.status == WorkflowStatus::Queued)
            .collect::<Vec<_>>();
        let mut completed = Vec::with_capacity(queued.len());
        for run in queued {
            completed.push(self.run_queued(&run.run_id).await?);
        }
        Ok(completed)
    }

    pub(super) fn run_version(&self, run_id: &str) -> Result<u64, StoreError> {
        u64::try_from(
            self.journal
                .read_stream(&format!("workflow-run:{run_id}"))?
                .len(),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
    }

    pub(super) fn append_run_event(
        &self,
        run_id: &str,
        event_type: &str,
        payload: Value,
    ) -> Result<(), StoreError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let expected_version = self.run_version(run_id)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: format!("workflow-run:{run_id}"),
            expected_stream_version: expected_version,
            classification: EventClassification::Workflow,
            event_type: event_type.into(),
            actor: Actor {
                actor_type: ActorType::Workflow,
                id: run_id.into(),
            },
            context: ExecutionContext {
                correlation_id: run_id.into(),
                run_id: Some(run_id.into()),
                workflow_id: Some(run_id.into()),
                ..ExecutionContext::default()
            },
            payload,
        })?;
        Ok(())
    }

    pub(super) async fn drive(
        &self,
        run_id: &str,
        definition: WorkflowDefinition,
        workflow_hash: String,
        inputs: Value,
        start_index: u32,
    ) -> Result<(), WorkflowError> {
        let events = self
            .journal
            .read_stream(&format!("workflow-run:{run_id}"))?;
        let mut context = json!({"inputs": inputs, "steps": {}, "executions": {}});
        for event in &events {
            if event.event_type == "workflow.step.completed.v1" {
                let payload = self.journal.decrypt_payload(event)?;
                if let (Some(step_id), Some(output)) = (
                    payload.get("step_id").and_then(Value::as_str),
                    payload.get("output").cloned(),
                ) {
                    let execution_id = payload
                        .get("execution_id")
                        .and_then(Value::as_str)
                        .unwrap_or(step_id);
                    context["executions"][execution_id] = output.clone();
                    if execution_id == step_id {
                        context["steps"][step_id] = output;
                    }
                }
            }
        }
        let attempts = events
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    "workflow.step.started.v1" | "workflow.compensation.step.started.v1"
                )
            })
            .count();
        let budget = Arc::new(AtomicU32::new(
            u32::try_from(attempts)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?,
        ));
        let semaphore = Arc::new(Semaphore::new(
            usize::try_from(definition.max_concurrency)
                .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?,
        ));
        for (index, step) in definition.steps.iter().enumerate().skip(
            usize::try_from(start_index)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?,
        ) {
            match self
                .execute_step(
                    run_id,
                    &workflow_hash,
                    step,
                    "",
                    &mut context,
                    Arc::clone(&budget),
                    definition.step_budget,
                    Arc::clone(&semaphore),
                )
                .await
            {
                Ok(StepState::Completed(output)) => {
                    context["steps"][step_id(step)] = output.clone();
                    self.append_run_event(
                        run_id,
                        "workflow.step.completed.v1",
                        json!({
                            "root_index": index,
                            "step_id": step_id(step),
                            "execution_id": step_id(step),
                            "output": output,
                        }),
                    )?;
                }
                Ok(StepState::Waiting {
                    step_id: waiting_step_id,
                    execution_id,
                    reason,
                    child_run_id,
                }) => {
                    self.append_run_event(
                        run_id,
                        "workflow.run.waiting.v1",
                        json!({
                            "step_id": waiting_step_id,
                            "execution_id": execution_id,
                            "reason": reason,
                            "child_run_id": child_run_id,
                        }),
                    )?;
                    return Ok(());
                }
                Err(WorkflowError::OutcomeUnknown(message)) => {
                    self.append_run_event(
                        run_id,
                        "workflow.step.outcome_unknown.v1",
                        json!({
                            "step_id": step_id(step),
                            "retry_allowed": step_retryable(step),
                            "reason": &message,
                        }),
                    )?;
                    self.append_run_event(
                        run_id,
                        "workflow.run.interrupted.v1",
                        json!({"step_id": step_id(step), "reason": message}),
                    )?;
                    return Ok(());
                }
                Err(error) => {
                    let compensation = self
                        .run_compensation(
                            run_id,
                            &workflow_hash,
                            &definition.compensation,
                            Arc::clone(&budget),
                            definition.step_budget,
                            Arc::clone(&semaphore),
                        )
                        .await;
                    if let Err(WorkflowError::OutcomeUnknown(message)) = &compensation {
                        self.append_run_event(
                            run_id,
                            "workflow.step.outcome_unknown.v1",
                            json!({
                                "phase": "compensation",
                                "retry_allowed": false,
                                "reason": message,
                            }),
                        )?;
                        self.append_run_event(
                            run_id,
                            "workflow.run.interrupted.v1",
                            json!({"phase": "compensation", "reason": message}),
                        )?;
                        return Ok(());
                    }
                    self.append_run_event(
                        run_id,
                        "workflow.run.failed.v1",
                        json!({
                            "step_id": step_id(step),
                            "reason": error.to_string(),
                            "compensation": compensation.err().map(|error| error.to_string()),
                        }),
                    )?;
                    return Ok(());
                }
            }
        }
        let outputs = context.get("steps").cloned().unwrap_or(Value::Null);
        if let Err(error) = validate_instance(&definition.outputs, &outputs, "output") {
            let compensation = self
                .run_compensation(
                    run_id,
                    &workflow_hash,
                    &definition.compensation,
                    Arc::clone(&budget),
                    definition.step_budget,
                    Arc::clone(&semaphore),
                )
                .await;
            if let Err(WorkflowError::OutcomeUnknown(message)) = &compensation {
                self.append_run_event(
                    run_id,
                    "workflow.step.outcome_unknown.v1",
                    json!({
                        "phase": "compensation",
                        "retry_allowed": false,
                        "reason": message,
                    }),
                )?;
                self.append_run_event(
                    run_id,
                    "workflow.run.interrupted.v1",
                    json!({"phase": "compensation", "reason": message}),
                )?;
                return Ok(());
            }
            self.append_run_event(
                run_id,
                "workflow.run.failed.v1",
                json!({
                    "reason": error.to_string(),
                    "phase": "output_validation",
                    "compensation": compensation.err().map(|error| error.to_string()),
                }),
            )?;
            return Ok(());
        }
        self.append_run_event(
            run_id,
            "workflow.run.completed.v1",
            json!({"outputs": outputs}),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[async_recursion]
    pub(super) async fn execute_step(
        &self,
        run_id: &str,
        workflow_hash: &str,
        step: &WorkflowStep,
        scope: &str,
        context: &mut Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        let execution_id = scoped_execution_id(scope, step_id(step));
        if let Some(output) = context
            .get("executions")
            .and_then(|executions| executions.get(&execution_id))
            .cloned()
        {
            return Ok(StepState::Completed(output));
        }
        if let WorkflowStep::Workflow { id, .. } = step
            && let Some(child) = self.linked_child(run_id, &execution_id)?
        {
            return self
                .observe_child_run(run_id, id, &execution_id, &child)
                .await;
        }
        let attempt = budget.fetch_add(1, Ordering::AcqRel).saturating_add(1);
        if attempt > step_budget {
            return Err(WorkflowError::InvalidTransition(
                "total step-attempt budget exhausted".into(),
            ));
        }
        self.append_run_event(
            run_id,
            "workflow.step.started.v1",
            json!({
                "step_id": step_id(step),
                "execution_id": execution_id,
                "attempt": attempt,
            }),
        )?;
        match step {
            WorkflowStep::Emit { value, .. } => Ok(StepState::Completed(value.clone())),
            WorkflowStep::WaitForInput { id, prompt, .. } => Ok(StepState::Waiting {
                step_id: id.clone(),
                execution_id: execution_id.clone(),
                reason: prompt.clone(),
                child_run_id: None,
            }),
            WorkflowStep::Approval { id, prompt, .. } => Ok(StepState::Waiting {
                step_id: id.clone(),
                execution_id: execution_id.clone(),
                reason: prompt.clone(),
                child_run_id: None,
            }),
            WorkflowStep::Agent {
                id,
                prompt,
                idempotency,
            } => {
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|error| WorkflowError::Effect(error.to_string()))?;
                self.run_effect_with_retry(
                    WorkflowEffect {
                        kind: "agent".into(),
                        action: "agent.run".into(),
                        content: json!({"prompt": prompt}),
                        idempotency: idempotency
                            .as_ref()
                            .map(|strategy| format!("{strategy}:{run_id}:{execution_id}")),
                        credential_references: Vec::new(),
                        run_id: run_id.into(),
                        step_id: execution_id.clone(),
                        definition_step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
                .await
                .map(StepState::Completed)
            }
            WorkflowStep::Tool {
                id,
                tool,
                arguments,
                idempotency,
            } => {
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|error| WorkflowError::Effect(error.to_string()))?;
                self.run_effect_with_retry(
                    WorkflowEffect {
                        kind: "tool".into(),
                        action: tool.clone(),
                        content: arguments.clone(),
                        idempotency: idempotency
                            .as_ref()
                            .map(|strategy| format!("{strategy}:{run_id}:{execution_id}")),
                        credential_references: Vec::new(),
                        run_id: run_id.into(),
                        step_id: execution_id.clone(),
                        definition_step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
                .await
                .map(StepState::Completed)
            }
            WorkflowStep::Workflow {
                id,
                workflow,
                version,
                inputs,
            } => {
                self.run_effect_with_retry(
                    WorkflowEffect {
                        kind: "workflow".into(),
                        action: "workflow.start".into(),
                        content: json!({
                            "workflow": workflow,
                            "version": version,
                            "inputs": inputs,
                        }),
                        idempotency: Some(format!("subworkflow:{run_id}:{execution_id}")),
                        credential_references: Vec::new(),
                        run_id: run_id.into(),
                        step_id: execution_id.clone(),
                        definition_step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
                .await?;
                let child_run_id = Uuid::now_v7().to_string();
                let parent = self.get_run(run_id)?;
                let call_depth = parent.call_depth.saturating_add(1);
                self.append_run_event(
                    run_id,
                    "workflow.subworkflow.linked.v1",
                    json!({
                        "step_id": id,
                        "execution_id": execution_id,
                        "child_run_id": child_run_id,
                        "workflow_name": workflow,
                        "workflow_version": version,
                        "inputs": inputs,
                        "call_depth": call_depth,
                    }),
                )?;
                let child = LinkedWorkflowCall {
                    run_id: child_run_id,
                    workflow_name: workflow.clone(),
                    workflow_version: version.clone(),
                    inputs: inputs.clone(),
                    call_depth,
                };
                self.observe_child_run(run_id, id, &execution_id, &child)
                    .await
            }
            WorkflowStep::Condition {
                expression,
                then,
                otherwise,
                ..
            } => {
                let condition = Condition::parse(expression)?;
                let selected = if condition.evaluate(context) {
                    then
                } else {
                    otherwise
                };
                self.execute_sequence(
                    run_id,
                    workflow_hash,
                    selected,
                    scope,
                    context,
                    budget,
                    step_budget,
                    semaphore,
                )
                .await
            }
            WorkflowStep::Parallel {
                branches,
                max_concurrency,
                ..
            } => {
                self.execute_parallel(
                    run_id,
                    workflow_hash,
                    branches,
                    *max_concurrency,
                    &execution_id,
                    context,
                    budget,
                    step_budget,
                    semaphore,
                )
                .await
            }
            WorkflowStep::Foreach {
                items,
                max_items,
                steps,
                ..
            } => {
                let values = context
                    .pointer(items)
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| {
                        WorkflowError::InvalidTransition(format!(
                            "foreach pointer {items} is not an array"
                        ))
                    })?;
                if values.len()
                    > usize::try_from(*max_items)
                        .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?
                {
                    return Err(WorkflowError::InvalidTransition(
                        "foreach input exceeds declared maximum".into(),
                    ));
                }
                let mut outputs = Vec::with_capacity(values.len());
                for (index, item) in values.into_iter().enumerate() {
                    let iteration_scope = format!("{execution_id}[{index}]");
                    let mut iteration = context.clone();
                    iteration["item"] = item;
                    iteration["index"] = json!(index);
                    let state = self
                        .execute_sequence(
                            run_id,
                            workflow_hash,
                            steps,
                            &iteration_scope,
                            &mut iteration,
                            Arc::clone(&budget),
                            step_budget,
                            Arc::clone(&semaphore),
                        )
                        .await?;
                    if let StepState::Waiting {
                        step_id,
                        execution_id,
                        reason,
                        child_run_id,
                    } = state
                    {
                        return Ok(StepState::Waiting {
                            step_id,
                            execution_id,
                            reason,
                            child_run_id,
                        });
                    }
                    if let Some(object) = iteration.as_object_mut() {
                        object.remove("executions");
                    }
                    outputs.push(iteration);
                }
                Ok(StepState::Completed(Value::Array(outputs)))
            }
        }
    }

    pub(super) fn linked_child(
        &self,
        parent_run_id: &str,
        execution_id: &str,
    ) -> Result<Option<LinkedWorkflowCall>, WorkflowError> {
        for event in self
            .journal
            .read_stream(&format!("workflow-run:{parent_run_id}"))?
            .iter()
            .rev()
            .filter(|event| event.event_type == "workflow.subworkflow.linked.v1")
        {
            let payload = self.journal.decrypt_payload(event)?;
            if payload
                .get("execution_id")
                .or_else(|| payload.get("step_id"))
                .and_then(Value::as_str)
                == Some(execution_id)
            {
                return Ok(Some(LinkedWorkflowCall {
                    run_id: string_field(&payload, "child_run_id")?,
                    workflow_name: string_field(&payload, "workflow_name")?,
                    workflow_version: string_field(&payload, "workflow_version")?,
                    inputs: payload.get("inputs").cloned().unwrap_or(Value::Null),
                    call_depth: payload
                        .get("call_depth")
                        .and_then(Value::as_u64)
                        .and_then(|depth| u16::try_from(depth).ok())
                        .ok_or_else(|| {
                            WorkflowError::InvalidTransition(
                                "linked child call depth is absent or invalid".into(),
                            )
                        })?,
                }));
            }
        }
        Ok(None)
    }

    #[async_recursion]
    pub(super) async fn observe_child_run(
        &self,
        parent_run_id: &str,
        parent_step_id: &str,
        parent_execution_id: &str,
        linked: &LinkedWorkflowCall,
    ) -> Result<StepState, WorkflowError> {
        if self.repository.run(&linked.run_id)?.is_none() {
            self.queue_run_with_lineage(
                &linked.run_id,
                &linked.workflow_name,
                &linked.workflow_version,
                linked.inputs.clone(),
                Some(parent_run_id),
                Some(parent_step_id),
                Some(parent_execution_id),
                linked.call_depth,
            )?;
        }
        let mut child = self.get_run(&linked.run_id)?;
        if child.status == WorkflowStatus::Queued {
            child = self.run_queued(&linked.run_id).await?;
        }
        match child.status {
            WorkflowStatus::Completed => {
                let output = json!({
                    "run_id": child.run_id,
                    "workflow_hash": child.workflow_hash,
                    "outputs": child.outputs,
                });
                self.append_run_event(
                    parent_run_id,
                    "workflow.subworkflow.completed.v1",
                    json!({
                        "step_id": parent_step_id,
                        "execution_id": parent_execution_id,
                        "child_run_id": linked.run_id,
                        "output": output,
                    }),
                )?;
                Ok(StepState::Completed(output))
            }
            WorkflowStatus::Queued | WorkflowStatus::Running | WorkflowStatus::Waiting => {
                Ok(StepState::Waiting {
                    step_id: parent_step_id.into(),
                    execution_id: parent_execution_id.into(),
                    reason: format!("waiting for child workflow run {}", linked.run_id),
                    child_run_id: Some(linked.run_id.clone()),
                })
            }
            WorkflowStatus::Failed | WorkflowStatus::Cancelled | WorkflowStatus::Interrupted => {
                Err(WorkflowError::Effect(format!(
                    "child workflow run {} reached {}",
                    linked.run_id,
                    workflow_status_name(child.status)
                )))
            }
        }
    }

    pub(super) async fn run_effect_with_retry(
        &self,
        mut effect: WorkflowEffect,
        budget: Arc<AtomicU32>,
        step_budget: u32,
    ) -> Result<Value, WorkflowError> {
        match self.effects.run(effect.clone()).await {
            Err(WorkflowError::Effect(first_error)) if effect.idempotency.is_some() => {
                let retry_attempt = budget.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                if retry_attempt > step_budget {
                    return Err(WorkflowError::InvalidTransition(
                        "total step-attempt budget exhausted before idempotent retry".into(),
                    ));
                }
                self.append_run_event(
                    &effect.run_id,
                    "workflow.step.retrying.v1",
                    json!({
                        "step_id": effect.definition_step_id,
                        "execution_id": effect.step_id,
                        "failed_attempt": effect.attempt,
                        "next_attempt": retry_attempt,
                        "reason": first_error,
                        "idempotency": effect.idempotency,
                    }),
                )?;
                effect.attempt = retry_attempt;
                self.append_run_event(
                    &effect.run_id,
                    "workflow.step.started.v1",
                    json!({
                        "step_id": effect.definition_step_id,
                        "execution_id": effect.step_id,
                        "attempt": retry_attempt,
                        "retry": true,
                    }),
                )?;
                self.effects.run(effect).await
            }
            result => result,
        }
    }

    pub(super) async fn run_compensation(
        &self,
        run_id: &str,
        workflow_hash: &str,
        steps: &[WorkflowStep],
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<(), WorkflowError> {
        for step in steps {
            let attempt = budget.fetch_add(1, Ordering::AcqRel).saturating_add(1);
            if attempt > step_budget {
                return Err(WorkflowError::InvalidTransition(
                    "total step-attempt budget exhausted during compensation".into(),
                ));
            }
            self.append_run_event(
                run_id,
                "workflow.compensation.step.started.v1",
                json!({"step_id": step_id(step), "attempt": attempt}),
            )?;
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|error| WorkflowError::Effect(error.to_string()))?;
            let effect = match step {
                WorkflowStep::Agent {
                    id,
                    prompt,
                    idempotency,
                } => WorkflowEffect {
                    kind: "agent".into(),
                    action: "agent.run".into(),
                    content: json!({"prompt": prompt}),
                    idempotency: idempotency.clone(),
                    credential_references: Vec::new(),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    definition_step_id: id.clone(),
                    workflow_hash: workflow_hash.into(),
                    attempt,
                    compensation: true,
                },
                WorkflowStep::Tool {
                    id,
                    tool,
                    arguments,
                    idempotency,
                } => WorkflowEffect {
                    kind: "tool".into(),
                    action: tool.clone(),
                    content: arguments.clone(),
                    idempotency: idempotency.clone(),
                    credential_references: Vec::new(),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    definition_step_id: id.clone(),
                    workflow_hash: workflow_hash.into(),
                    attempt,
                    compensation: true,
                },
                _ => {
                    return Err(WorkflowError::InvalidDefinition(
                        "validated compensation contains an unsupported step".into(),
                    ));
                }
            };
            match self
                .run_effect_with_retry(effect, Arc::clone(&budget), step_budget)
                .await
            {
                Ok(output) => self.append_run_event(
                    run_id,
                    "workflow.compensation.step.completed.v1",
                    json!({"step_id": step_id(step), "output": output}),
                )?,
                Err(error) => {
                    self.append_run_event(
                        run_id,
                        "workflow.compensation.step.failed.v1",
                        json!({"step_id": step_id(step), "reason": error.to_string()}),
                    )?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_parallel(
        &self,
        run_id: &str,
        workflow_hash: &str,
        branches: &[Vec<WorkflowStep>],
        max_concurrency: u32,
        scope: &str,
        context: &Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        let concurrency = usize::try_from(max_concurrency)
            .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
        let base_context = context.clone();
        let owned_branches = branches.to_vec();
        let scope = scope.to_owned();
        let results = stream::iter(owned_branches.into_iter().enumerate())
            .map(|(index, branch)| {
                let mut branch_context = base_context.clone();
                let budget = Arc::clone(&budget);
                let semaphore = Arc::clone(&semaphore);
                let branch_scope = format!("{scope}.branch[{index}]");
                async move {
                    let state = self
                        .execute_sequence(
                            run_id,
                            workflow_hash,
                            &branch,
                            &branch_scope,
                            &mut branch_context,
                            budget,
                            step_budget,
                            semaphore,
                        )
                        .await?;
                    Ok::<_, WorkflowError>((index, state, branch_context))
                }
            })
            .buffer_unordered(concurrency)
            .try_collect::<Vec<_>>()
            .await?;
        let mut ordered = results;
        ordered.sort_by_key(|(index, _, _)| *index);
        if let Some((step_id, execution_id, reason, child_run_id)) =
            ordered.iter().find_map(|(_, state, _)| match state {
                StepState::Waiting {
                    step_id,
                    execution_id,
                    reason,
                    child_run_id,
                } => Some((
                    step_id.clone(),
                    execution_id.clone(),
                    reason.clone(),
                    child_run_id.clone(),
                )),
                StepState::Completed(_) => None,
            })
        {
            return Ok(StepState::Waiting {
                step_id,
                execution_id,
                reason,
                child_run_id,
            });
        }
        Ok(StepState::Completed(Value::Array(
            ordered
                .into_iter()
                .map(|(_, _, branch_context)| branch_context)
                .collect(),
        )))
    }

    #[allow(clippy::too_many_arguments)]
    #[async_recursion]
    pub(super) async fn execute_sequence(
        &self,
        run_id: &str,
        workflow_hash: &str,
        steps: &[WorkflowStep],
        scope: &str,
        context: &mut Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        for step in steps {
            let execution_id = scoped_execution_id(scope, step_id(step));
            let already_completed = context
                .get("executions")
                .and_then(|executions| executions.get(&execution_id))
                .is_some();
            match self
                .execute_step(
                    run_id,
                    workflow_hash,
                    step,
                    scope,
                    context,
                    Arc::clone(&budget),
                    step_budget,
                    Arc::clone(&semaphore),
                )
                .await?
            {
                StepState::Completed(output) => {
                    context["executions"][&execution_id] = output.clone();
                    context["steps"][step_id(step)] = output;
                    if !already_completed {
                        self.append_run_event(
                            run_id,
                            "workflow.step.completed.v1",
                            json!({
                                "step_id": step_id(step),
                                "execution_id": execution_id,
                                "output": context["steps"][step_id(step)],
                            }),
                        )?;
                    }
                }
                waiting @ StepState::Waiting { .. } => return Ok(waiting),
            }
        }
        Ok(StepState::Completed(
            context.get("steps").cloned().unwrap_or(Value::Null),
        ))
    }
}

pub(super) enum StepState {
    Completed(Value),
    Waiting {
        step_id: String,
        execution_id: String,
        reason: String,
        child_run_id: Option<String>,
    },
}

pub(super) struct LinkedWorkflowCall {
    run_id: String,
    workflow_name: String,
    workflow_version: String,
    inputs: Value,
    call_depth: u16,
}

fn workflow_status_name(status: WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Queued => "queued",
        WorkflowStatus::Running => "running",
        WorkflowStatus::Waiting => "waiting",
        WorkflowStatus::Completed => "completed",
        WorkflowStatus::Failed => "failed",
        WorkflowStatus::Cancelled => "cancelled",
        WorkflowStatus::Interrupted => "interrupted",
    }
}

pub(super) fn validate_instance(
    schema: &Value,
    instance: &Value,
    label: &str,
) -> Result<(), WorkflowError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| WorkflowError::Schema(error.to_string()))?;
    let errors = validator
        .iter_errors(instance)
        .take(8)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkflowError::Schema(format!(
            "{label}: {}",
            errors.join("; ")
        )))
    }
}

/// Effect runner for validation-only/offline workflows containing only pure steps.
pub struct DenyWorkflowEffects;

#[async_trait]
impl WorkflowEffectRunner for DenyWorkflowEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError> {
        Err(WorkflowError::Effect(format!(
            "no runtime adapter is configured for {} step {}",
            effect.kind, effect.step_id
        )))
    }
}
