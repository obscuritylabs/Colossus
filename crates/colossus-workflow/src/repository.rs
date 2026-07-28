use super::*;

/// Event-sourced workflow definition and run repository.
pub struct EventSourcedWorkflowRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedWorkflowRepository {
    /// Create a repository over the canonical journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl WorkflowRepository for EventSourcedWorkflowRepository {
    fn register(
        &self,
        definition: &WorkflowDefinition,
        content_hash: &str,
        provenance: &str,
    ) -> Result<(), StoreError> {
        let stream_id = format!(
            "workflow-definition:{}:{}",
            definition.metadata.name, definition.metadata.version
        );
        let existing = self.journal.read_stream(&stream_id)?;
        if let Some(last) = existing.last() {
            let payload = self.journal.decrypt_payload(last)?;
            if payload.get("content_hash").and_then(Value::as_str) == Some(content_hash) {
                return Ok(());
            }
        }
        let event_type = if existing.is_empty() {
            "workflow.definition.registered.v1"
        } else {
            "workflow.definition.changed.v1"
        };
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: u64::try_from(existing.len())
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
            classification: EventClassification::Workflow,
            event_type: event_type.into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "workflow-registrar".into(),
            },
            context: ExecutionContext {
                correlation_id: Uuid::now_v7().to_string(),
                ..ExecutionContext::default()
            },
            payload: json!({
                "definition": definition,
                "content_hash": content_hash,
                "provenance": provenance,
                "trust_invalidated": !existing.is_empty(),
            }),
        })?;
        Ok(())
    }

    fn definition(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<(WorkflowDefinition, String)>, StoreError> {
        let stream_id = format!("workflow-definition:{name}:{version}");
        let Some(last) = self.journal.read_stream(&stream_id)?.last().cloned() else {
            return Ok(None);
        };
        let payload = self.journal.decrypt_payload(&last)?;
        let definition = serde_json::from_value(
            payload
                .get("definition")
                .cloned()
                .ok_or_else(|| StoreError::Verification("definition payload is absent".into()))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;
        let content_hash = payload
            .get("content_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Verification("definition hash is absent".into()))?;
        Ok(Some((definition, content_hash.into())))
    }

    fn run(&self, run_id: &str) -> Result<Option<WorkflowRun>, StoreError> {
        fold_run(self.journal.as_ref(), run_id)
    }

    fn runs(&self, limit: usize) -> Result<Vec<WorkflowRun>, StoreError> {
        let events = self.journal.read_global(1, usize::MAX)?;
        let mut run_ids = Vec::new();
        let mut seen = BTreeSet::new();
        for event in events {
            if matches!(
                event.event_type.as_str(),
                "workflow.run.queued.v1" | "workflow.run.started.v1"
            ) && let Some(run_id) = event.stream_id.strip_prefix("workflow-run:")
                && seen.insert(run_id.to_owned())
            {
                run_ids.push(run_id.to_owned());
            }
        }
        run_ids
            .into_iter()
            .rev()
            .take(limit)
            .map(|run_id| {
                fold_run(self.journal.as_ref(), &run_id)?.ok_or_else(|| {
                    StoreError::Verification(format!("run {run_id} start event is unreadable"))
                })
            })
            .collect()
    }

    fn create_schedule(
        &self,
        schedule: &WorkflowSchedule,
        actor: Actor,
    ) -> Result<WorkflowSchedule, StoreError> {
        let stream_id = schedule_stream(&schedule.schedule_id);
        if !self.journal.read_stream(&stream_id)?.is_empty() {
            return Err(StoreError::Adapter(format!(
                "workflow schedule {} already exists",
                schedule.schedule_id
            )));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: "workflow.schedule.registered.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: schedule.schedule_id.clone(),
                workflow_id: Some(schedule.schedule_id.clone()),
                workflow_hash: Some(schedule.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": schedule}),
        })?;
        Ok(schedule.clone())
    }

    fn set_schedule_enabled(
        &self,
        schedule_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowSchedule, StoreError> {
        let mut schedule = self
            .schedule(schedule_id)?
            .ok_or_else(|| StoreError::NotFound(format!("workflow schedule {schedule_id}")))?;
        if schedule.enabled == enabled {
            return Ok(schedule);
        }
        schedule.enabled = enabled;
        schedule.updated_at = updated_at.into();
        if enabled {
            schedule.blocked_reason = None;
        }
        let stream_id = schedule_stream(schedule_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: if enabled {
                "workflow.schedule.enabled.v1"
            } else {
                "workflow.schedule.disabled.v1"
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: schedule_id.into(),
                workflow_id: Some(schedule_id.into()),
                workflow_hash: Some(schedule.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": &schedule}),
        })?;
        Ok(schedule)
    }

    fn schedule(&self, schedule_id: &str) -> Result<Option<WorkflowSchedule>, StoreError> {
        fold_schedule(self.journal.as_ref(), schedule_id)
    }

    fn schedules(&self, limit: usize) -> Result<Vec<WorkflowSchedule>, StoreError> {
        let mut schedule_ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "workflow.schedule.registered.v1"
                && let Some(schedule_id) = event.stream_id.strip_prefix("workflow-schedule:")
            {
                schedule_ids.insert(schedule_id.to_owned());
            }
        }
        schedule_ids
            .into_iter()
            .take(limit)
            .map(|schedule_id| {
                fold_schedule(self.journal.as_ref(), &schedule_id)?.ok_or_else(|| {
                    StoreError::Verification(format!(
                        "workflow schedule {schedule_id} cannot be reconstructed"
                    ))
                })
            })
            .collect()
    }

    fn create_webhook(
        &self,
        webhook: &WorkflowWebhook,
        actor: Actor,
    ) -> Result<WorkflowWebhook, StoreError> {
        let stream_id = webhook_stream(&webhook.webhook_id);
        if !self.journal.read_stream(&stream_id)?.is_empty() {
            return Err(StoreError::Adapter(format!(
                "workflow webhook {} already exists",
                webhook.webhook_id
            )));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: "workflow.webhook.registered.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: webhook.webhook_id.clone(),
                workflow_id: Some(webhook.webhook_id.clone()),
                workflow_hash: Some(webhook.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": webhook}),
        })?;
        Ok(webhook.clone())
    }

    fn set_webhook_enabled(
        &self,
        webhook_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowWebhook, StoreError> {
        let mut webhook = self
            .webhook(webhook_id)?
            .ok_or_else(|| StoreError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if webhook.enabled == enabled {
            return Ok(webhook);
        }
        webhook.enabled = enabled;
        webhook.updated_at = updated_at.into();
        if enabled {
            webhook.blocked_reason = None;
        }
        let stream_id = webhook_stream(webhook_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: if enabled {
                "workflow.webhook.enabled.v1"
            } else {
                "workflow.webhook.disabled.v1"
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: webhook_id.into(),
                workflow_id: Some(webhook_id.into()),
                workflow_hash: Some(webhook.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": &webhook}),
        })?;
        Ok(webhook)
    }

    fn webhook(&self, webhook_id: &str) -> Result<Option<WorkflowWebhook>, StoreError> {
        fold_webhook(self.journal.as_ref(), webhook_id)
    }

    fn webhooks(&self, limit: usize) -> Result<Vec<WorkflowWebhook>, StoreError> {
        let mut webhook_ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "workflow.webhook.registered.v1"
                && let Some(webhook_id) = event.stream_id.strip_prefix("workflow-webhook:")
            {
                webhook_ids.insert(webhook_id.to_owned());
            }
        }
        webhook_ids
            .into_iter()
            .take(limit)
            .map(|webhook_id| {
                fold_webhook(self.journal.as_ref(), &webhook_id)?.ok_or_else(|| {
                    StoreError::Verification(format!(
                        "workflow webhook {webhook_id} cannot be reconstructed"
                    ))
                })
            })
            .collect()
    }

    fn webhook_delivery(
        &self,
        webhook_id: &str,
        delivery_id: &str,
    ) -> Result<Option<WorkflowWebhookDelivery>, StoreError> {
        fold_webhook_delivery(self.journal.as_ref(), webhook_id, delivery_id)
    }

    fn create_subscription(
        &self,
        subscription: &WorkflowSubscription,
        actor: Actor,
    ) -> Result<WorkflowSubscription, StoreError> {
        let stream_id = subscription_stream(&subscription.subscription_id);
        if !self.journal.read_stream(&stream_id)?.is_empty() {
            return Err(StoreError::Adapter(format!(
                "workflow subscription {} already exists",
                subscription.subscription_id
            )));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: "workflow.subscription.registered.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: subscription.subscription_id.clone(),
                workflow_id: Some(subscription.subscription_id.clone()),
                workflow_hash: Some(subscription.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": subscription}),
        })?;
        Ok(subscription.clone())
    }

    fn set_subscription_enabled(
        &self,
        subscription_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowSubscription, StoreError> {
        let mut subscription = self.subscription(subscription_id)?.ok_or_else(|| {
            StoreError::NotFound(format!("workflow subscription {subscription_id}"))
        })?;
        if subscription.enabled == enabled {
            return Ok(subscription);
        }
        subscription.enabled = enabled;
        subscription.updated_at = updated_at.into();
        if enabled {
            subscription.blocked_reason = None;
        }
        let stream_id = subscription_stream(subscription_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: if enabled {
                "workflow.subscription.enabled.v1"
            } else {
                "workflow.subscription.disabled.v1"
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: subscription_id.into(),
                workflow_id: Some(subscription_id.into()),
                workflow_hash: Some(subscription.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": &subscription}),
        })?;
        Ok(subscription)
    }

    fn subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<WorkflowSubscription>, StoreError> {
        fold_subscription(self.journal.as_ref(), subscription_id)
    }

    fn subscriptions(&self, limit: usize) -> Result<Vec<WorkflowSubscription>, StoreError> {
        let mut subscription_ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "workflow.subscription.registered.v1"
                && let Some(subscription_id) =
                    event.stream_id.strip_prefix("workflow-subscription:")
            {
                subscription_ids.insert(subscription_id.to_owned());
            }
        }
        subscription_ids
            .into_iter()
            .take(limit)
            .map(|subscription_id| {
                fold_subscription(self.journal.as_ref(), &subscription_id)?.ok_or_else(|| {
                    StoreError::Verification(format!(
                        "workflow subscription {subscription_id} cannot be reconstructed"
                    ))
                })
            })
            .collect()
    }

    fn subscription_delivery(
        &self,
        subscription_id: &str,
        source_event_id: &str,
    ) -> Result<Option<WorkflowSubscriptionDelivery>, StoreError> {
        fold_subscription_delivery(self.journal.as_ref(), subscription_id, source_event_id)
    }
}

pub(super) fn schedule_stream(schedule_id: &str) -> String {
    format!("workflow-schedule:{schedule_id}")
}

pub(super) fn fold_schedule(
    journal: &dyn EventJournal,
    schedule_id: &str,
) -> Result<Option<WorkflowSchedule>, StoreError> {
    let events = journal.read_stream(&schedule_stream(schedule_id))?;
    let Some(last) = events.last() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(last)?;
    let schedule: WorkflowSchedule = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("schedule record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if schedule.schedule_id != schedule_id {
        return Err(StoreError::Verification(format!(
            "schedule stream {schedule_id} contains record {}",
            schedule.schedule_id
        )));
    }
    Ok(Some(schedule))
}

pub(super) fn webhook_stream(webhook_id: &str) -> String {
    format!("workflow-webhook:{webhook_id}")
}

pub(super) fn webhook_delivery_stream(webhook_id: &str, delivery_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(delivery_id.as_bytes()));
    format!("workflow-webhook-delivery:{webhook_id}:{digest}")
}

pub(super) fn fold_webhook(
    journal: &dyn EventJournal,
    webhook_id: &str,
) -> Result<Option<WorkflowWebhook>, StoreError> {
    let events = journal.read_stream(&webhook_stream(webhook_id))?;
    let Some(last) = events.last() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(last)?;
    let webhook: WorkflowWebhook = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("webhook record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if webhook.webhook_id != webhook_id {
        return Err(StoreError::Verification(format!(
            "webhook stream {webhook_id} contains record {}",
            webhook.webhook_id
        )));
    }
    Ok(Some(webhook))
}

pub(super) fn fold_webhook_delivery(
    journal: &dyn EventJournal,
    webhook_id: &str,
    delivery_id: &str,
) -> Result<Option<WorkflowWebhookDelivery>, StoreError> {
    let events = journal.read_stream(&webhook_delivery_stream(webhook_id, delivery_id))?;
    let Some(first) = events.first() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(first)?;
    let delivery: WorkflowWebhookDelivery = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("webhook delivery record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if delivery.webhook_id != webhook_id || delivery.delivery_id != delivery_id {
        return Err(StoreError::Verification(
            "webhook delivery stream identity does not match its record".into(),
        ));
    }
    Ok(Some(delivery))
}

pub(super) fn subscription_stream(subscription_id: &str) -> String {
    format!("workflow-subscription:{subscription_id}")
}

pub(super) fn subscription_delivery_stream(subscription_id: &str, source_event_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(source_event_id.as_bytes()));
    format!("workflow-subscription-delivery:{subscription_id}:{digest}")
}

pub(super) fn fold_subscription(
    journal: &dyn EventJournal,
    subscription_id: &str,
) -> Result<Option<WorkflowSubscription>, StoreError> {
    let events = journal.read_stream(&subscription_stream(subscription_id))?;
    let Some(last) = events.last() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(last)?;
    let subscription: WorkflowSubscription = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("subscription record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if subscription.subscription_id != subscription_id {
        return Err(StoreError::Verification(format!(
            "subscription stream {subscription_id} contains record {}",
            subscription.subscription_id
        )));
    }
    Ok(Some(subscription))
}

pub(super) fn fold_subscription_delivery(
    journal: &dyn EventJournal,
    subscription_id: &str,
    source_event_id: &str,
) -> Result<Option<WorkflowSubscriptionDelivery>, StoreError> {
    let events = journal.read_stream(&subscription_delivery_stream(
        subscription_id,
        source_event_id,
    ))?;
    let Some(first) = events.first() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(first)?;
    let delivery: WorkflowSubscriptionDelivery =
        serde_json::from_value(payload.get("record").cloned().ok_or_else(|| {
            StoreError::Verification("subscription delivery record is absent".into())
        })?)
        .map_err(|error| StoreError::Verification(error.to_string()))?;
    if delivery.subscription_id != subscription_id || delivery.source_event_id != source_event_id {
        return Err(StoreError::Verification(
            "subscription delivery stream identity does not match its record".into(),
        ));
    }
    Ok(Some(delivery))
}

pub(super) fn fold_run(
    journal: &dyn EventJournal,
    run_id: &str,
) -> Result<Option<WorkflowRun>, StoreError> {
    let events = journal.read_stream(&format!("workflow-run:{run_id}"))?;
    let Some(first) = events.first() else {
        return Ok(None);
    };
    let start = journal.decrypt_payload(first)?;
    let mut run = WorkflowRun {
        run_id: run_id.into(),
        workflow_name: string_field(&start, "workflow_name")?,
        workflow_version: string_field(&start, "workflow_version")?,
        workflow_hash: string_field(&start, "workflow_hash")?,
        parent_run_id: start
            .get("parent_run_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parent_step_id: start
            .get("parent_step_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parent_execution_id: start
            .get("parent_execution_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        trigger_kind: start
            .get("trigger_kind")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| StoreError::Verification(error.to_string()))?,
        trigger_id: start
            .get("trigger_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        trigger_occurrence: start
            .get("trigger_occurrence")
            .and_then(Value::as_str)
            .map(str::to_owned),
        call_depth: start
            .get("call_depth")
            .and_then(Value::as_u64)
            .and_then(|depth| u16::try_from(depth).ok())
            .unwrap_or(1),
        status: if first.event_type == "workflow.run.queued.v1" {
            WorkflowStatus::Queued
        } else {
            WorkflowStatus::Running
        },
        inputs: start.get("inputs").cloned().unwrap_or(Value::Null),
        outputs: None,
        failure_reason: None,
        completed_steps: 0,
        waiting_step_id: None,
        waiting_execution_id: None,
        waiting_reason: None,
        waiting_child_run_id: None,
    };
    for event in events.iter().skip(1) {
        let payload = journal.decrypt_payload(event)?;
        match event.event_type.as_str() {
            "workflow.run.queued.v1" => run.status = WorkflowStatus::Queued,
            "workflow.run.started.v1" => {
                run.status = WorkflowStatus::Running;
                run.failure_reason = None;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.step.completed.v1" => {
                run.completed_steps = payload
                    .get("root_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index.saturating_add(1)).ok())
                    .unwrap_or(run.completed_steps);
            }
            "workflow.run.waiting.v1" => {
                run.status = WorkflowStatus::Waiting;
                run.waiting_step_id = payload
                    .get("step_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_execution_id = payload
                    .get("execution_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_reason = payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_child_run_id = payload
                    .get("child_run_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "workflow.run.resumed.v1" => {
                run.status = WorkflowStatus::Running;
                run.failure_reason = None;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.completed.v1" => {
                run.status = WorkflowStatus::Completed;
                run.outputs = payload.get("outputs").cloned();
                run.failure_reason = None;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.failed.v1" => {
                run.status = WorkflowStatus::Failed;
                run.failure_reason = payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.cancelled.v1" => {
                run.status = WorkflowStatus::Cancelled;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.interrupted.v1" => {
                run.status = WorkflowStatus::Interrupted;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            _ => {}
        }
    }
    Ok(Some(run))
}

pub(super) fn string_field(value: &Value, field: &str) -> Result<String, StoreError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Verification(format!("run field {field} is absent")))
}

pub(super) fn parse_schedule_time(
    value: &str,
    label: &str,
) -> Result<OffsetDateTime, WorkflowError> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
        WorkflowError::InvalidDefinition(format!("{label} must be UTC RFC3339: {error}"))
    })?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(WorkflowError::InvalidDefinition(format!(
            "{label} must use the UTC Z offset"
        )));
    }
    Ok(parsed)
}

pub(super) fn format_schedule_time(value: OffsetDateTime) -> Result<String, WorkflowError> {
    value
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))
}

pub(super) fn add_schedule_occurrences(
    base: OffsetDateTime,
    cadence_seconds: u64,
    occurrences: u64,
) -> Result<OffsetDateTime, WorkflowError> {
    let total_seconds = cadence_seconds
        .checked_mul(occurrences)
        .ok_or_else(|| WorkflowError::InvalidTransition("schedule cadence overflow".into()))?;
    let total_seconds = i64::try_from(total_seconds)
        .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
    base.checked_add(TimeDuration::seconds(total_seconds))
        .ok_or_else(|| WorkflowError::InvalidTransition("schedule timestamp overflow".into()))
}

pub(super) fn scheduled_run_id(schedule_id: &str, occurrence: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{schedule_id}\0{occurrence}").as_bytes(),
    ));
    format!("schedule-{}", digest.chars().take(32).collect::<String>())
}

pub(super) fn schedule_event(
    schedule: &WorkflowSchedule,
    expected_stream_version: u64,
    event_type: &str,
    payload: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: schedule_stream(&schedule.schedule_id),
        expected_stream_version,
        classification: EventClassification::Workflow,
        event_type: event_type.into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: schedule.schedule_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: schedule.schedule_id.clone(),
            workflow_id: Some(schedule.schedule_id.clone()),
            workflow_hash: Some(schedule.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload,
    }
}

pub(super) fn scheduled_run_event(
    schedule: &WorkflowSchedule,
    run_id: &str,
    occurrence: &str,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("workflow-run:{run_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.run.queued.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: schedule.schedule_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: run_id.into(),
            run_id: Some(run_id.into()),
            workflow_id: Some(run_id.into()),
            workflow_hash: Some(schedule.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "workflow_name": schedule.workflow_name,
            "workflow_version": schedule.workflow_version,
            "workflow_hash": schedule.workflow_hash,
            "inputs": schedule.inputs,
            "parent_run_id": Value::Null,
            "parent_step_id": Value::Null,
            "parent_execution_id": Value::Null,
            "trigger_kind": WorkflowTriggerKind::Schedule,
            "trigger_id": schedule.schedule_id,
            "trigger_occurrence": occurrence,
            "call_depth": 1,
        }),
    }
}

pub(super) fn valid_environment_reference(reference: &str) -> bool {
    reference.strip_prefix("env:").is_some_and(|variable| {
        !variable.is_empty()
            && variable.len() <= 128
            && variable
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            && variable
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_uppercase)
    })
}

pub(super) fn validate_webhook_headers(
    headers: &BTreeMap<String, String>,
) -> Result<(), WorkflowError> {
    if headers.len() > MAX_WEBHOOK_HEADERS {
        return Err(WorkflowError::InvalidDefinition(format!(
            "webhook headers exceed the {MAX_WEBHOOK_HEADERS} field limit"
        )));
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(
                        byte,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                    )
            })
        {
            return Err(WorkflowError::InvalidDefinition(
                "webhook header names must be lowercase HTTP field names".into(),
            ));
        }
        if value.len() > 8 * 1024 || value.chars().any(|character| character.is_control()) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook header {name} contains an invalid or oversized value"
            )));
        }
        total = total.checked_add(name.len() + value.len()).ok_or_else(|| {
            WorkflowError::InvalidDefinition("webhook header size overflow".into())
        })?;
    }
    if total > MAX_WEBHOOK_HEADER_BYTES {
        return Err(WorkflowError::InvalidDefinition(format!(
            "webhook headers exceed {MAX_WEBHOOK_HEADER_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(super) fn verify_webhook_signature(
    timestamp: &str,
    delivery_id: &str,
    body: &[u8],
    signature: &str,
    secret: &[u8],
) -> Result<(), WorkflowError> {
    let signature = signature.strip_prefix("sha256=").unwrap_or(signature);
    if signature.len() != 64
        || !signature
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(WorkflowError::InvalidDefinition(
            "webhook signature must be sha256=<64 lowercase hex characters>".into(),
        ));
    }
    let decoded = hex::decode(signature).map_err(|_| {
        WorkflowError::InvalidDefinition("webhook signature is not valid hexadecimal".into())
    })?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
    mac.update(timestamp.as_bytes());
    mac.update(b"\n");
    mac.update(delivery_id.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac.verify_slice(&decoded)
        .map_err(|_| WorkflowError::InvalidTransition("webhook signature is invalid".into()))
}

pub(super) fn webhook_run_id(webhook_id: &str, delivery_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{webhook_id}\0{delivery_id}").as_bytes(),
    ));
    format!("webhook-{}", digest.chars().take(32).collect::<String>())
}

pub(super) fn webhook_delivery_event(
    webhook: &WorkflowWebhook,
    delivery: &WorkflowWebhookDelivery,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: webhook_delivery_stream(&webhook.webhook_id, &delivery.delivery_id),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.webhook.delivery.accepted.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: webhook.webhook_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: delivery.run_id.clone(),
            run_id: Some(delivery.run_id.clone()),
            workflow_id: Some(webhook.webhook_id.clone()),
            workflow_hash: Some(webhook.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({"record": delivery}),
    }
}

pub(super) fn webhook_run_event(
    webhook: &WorkflowWebhook,
    run_id: &str,
    delivery_id: &str,
    inputs: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("workflow-run:{run_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.run.queued.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: webhook.webhook_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: run_id.into(),
            run_id: Some(run_id.into()),
            workflow_id: Some(run_id.into()),
            workflow_hash: Some(webhook.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "workflow_name": webhook.workflow_name,
            "workflow_version": webhook.workflow_version,
            "workflow_hash": webhook.workflow_hash,
            "inputs": inputs,
            "parent_run_id": Value::Null,
            "parent_step_id": Value::Null,
            "parent_execution_id": Value::Null,
            "trigger_kind": WorkflowTriggerKind::Webhook,
            "trigger_id": webhook.webhook_id,
            "trigger_occurrence": delivery_id,
            "call_depth": 1,
        }),
    }
}

pub(super) fn valid_subscription_event_type(event_type: &str) -> bool {
    let Some((name, version)) = event_type.rsplit_once(".v") else {
        return false;
    };
    !name.is_empty()
        && !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_digit())
        && event_type.len() <= MAX_SUBSCRIPTION_EVENT_TYPE_BYTES
        && !event_type.starts_with("workflow.")
        && event_type.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

pub(super) fn subscription_matches(
    subscription: &WorkflowSubscription,
    event: &EventEnvelope,
) -> bool {
    event.classification == EventClassification::Domain
        && event.event_type == subscription.event_type
        && subscription
            .stream_prefix
            .as_deref()
            .is_none_or(|prefix| event.stream_id.starts_with(prefix))
}

pub(super) fn subscription_run_id(subscription_id: &str, source_event_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{subscription_id}\0{source_event_id}").as_bytes(),
    ));
    format!(
        "subscription-{}",
        digest.chars().take(32).collect::<String>()
    )
}

pub(super) fn subscription_event(
    subscription: &WorkflowSubscription,
    expected_stream_version: u64,
    event_type: &str,
    payload: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: subscription_stream(&subscription.subscription_id),
        expected_stream_version,
        classification: EventClassification::Workflow,
        event_type: event_type.into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: subscription.subscription_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: subscription.subscription_id.clone(),
            workflow_id: Some(subscription.subscription_id.clone()),
            workflow_hash: Some(subscription.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload,
    }
}

pub(super) fn subscription_delivery_event(
    subscription: &WorkflowSubscription,
    delivery: &WorkflowSubscriptionDelivery,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: subscription_delivery_stream(
            &subscription.subscription_id,
            &delivery.source_event_id,
        ),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.subscription.delivery.accepted.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: subscription.subscription_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: delivery.run_id.clone(),
            run_id: Some(delivery.run_id.clone()),
            workflow_id: Some(subscription.subscription_id.clone()),
            workflow_hash: Some(subscription.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({"record": delivery}),
    }
}

pub(super) fn subscription_run_event(
    subscription: &WorkflowSubscription,
    source_event_id: &str,
    run_id: &str,
    inputs: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("workflow-run:{run_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.run.queued.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: subscription.subscription_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: run_id.into(),
            run_id: Some(run_id.into()),
            workflow_id: Some(run_id.into()),
            workflow_hash: Some(subscription.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "workflow_name": subscription.workflow_name,
            "workflow_version": subscription.workflow_version,
            "workflow_hash": subscription.workflow_hash,
            "inputs": inputs,
            "parent_run_id": Value::Null,
            "parent_step_id": Value::Null,
            "parent_execution_id": Value::Null,
            "trigger_kind": WorkflowTriggerKind::Subscription,
            "trigger_id": subscription.subscription_id,
            "trigger_occurrence": source_event_id,
            "call_depth": 1,
        }),
    }
}
