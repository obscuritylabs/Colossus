use super::*;

impl WorkflowService {
    /// Create one bounded, hash-pinned repository-event subscription.
    #[allow(clippy::too_many_arguments)]
    pub fn create_subscription(
        &self,
        subscription_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        event_type: &str,
        stream_prefix: Option<&str>,
        enabled: bool,
        after_sequence: Option<u64>,
    ) -> Result<WorkflowSubscription, WorkflowError> {
        if subscription_id.is_empty()
            || subscription_id.len() > MAX_SUBSCRIPTION_ID_BYTES
            || !valid_name(subscription_id)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription id must contain 1..={MAX_SUBSCRIPTION_ID_BYTES} lowercase letters, digits, dots, or hyphens"
            )));
        }
        if !valid_subscription_event_type(event_type) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription event type must be a versioned name ending in .vN, contain at most {MAX_SUBSCRIPTION_EVENT_TYPE_BYTES} lowercase letters, digits, dots, underscores, or hyphens, and cannot target workflow lifecycle events"
            )));
        }
        if let Some(prefix) = stream_prefix
            && (prefix.is_empty()
                || prefix.len() > MAX_SUBSCRIPTION_STREAM_PREFIX_BYTES
                || prefix.chars().any(char::is_control))
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription stream prefix must contain 1..={MAX_SUBSCRIPTION_STREAM_PREFIX_BYTES} non-control bytes"
            )));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self.repository.subscription(subscription_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow subscription {subscription_id} already exists"
            )));
        }
        if self
            .repository
            .subscriptions(MAX_WORKFLOW_SUBSCRIPTIONS)?
            .len()
            >= MAX_WORKFLOW_SUBSCRIPTIONS
        {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow subscription limit {MAX_WORKFLOW_SUBSCRIPTIONS} is exhausted"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(workflow_name, workflow_version)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("{workflow_name}:{workflow_version}"))
            })?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        let (head, _) = self.journal.head()?;
        let checkpoint = after_sequence.unwrap_or(head);
        if checkpoint > head {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription checkpoint {checkpoint} is beyond journal head {head}"
            )));
        }
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let subscription = WorkflowSubscription {
            subscription_id: subscription_id.into(),
            workflow_name: workflow_name.into(),
            workflow_version: workflow_version.into(),
            workflow_hash,
            event_type: event_type.into(),
            stream_prefix: stream_prefix.map(str::to_owned),
            enabled,
            checkpoint,
            last_event_id: None,
            last_run_id: None,
            blocked_reason: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repository.create_subscription(
            &subscription,
            Actor {
                actor_type: ActorType::User,
                id: "workflow-subscription-registrar".into(),
            },
        )?;
        Ok(subscription)
    }

    /// Reconstruct one canonical repository-event subscription.
    pub fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<WorkflowSubscription, WorkflowError> {
        self.repository
            .subscription(subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow subscription {subscription_id}"))
            })
    }

    /// List bounded subscriptions in deterministic identifier order.
    pub fn list_subscriptions(
        &self,
        limit: usize,
    ) -> Result<Vec<WorkflowSubscription>, WorkflowError> {
        self.repository
            .subscriptions(limit.min(MAX_WORKFLOW_SUBSCRIPTIONS))
            .map_err(Into::into)
    }

    /// Explicitly enable or disable one subscription after rechecking pinned trust.
    pub fn set_subscription_enabled(
        &self,
        subscription_id: &str,
        enabled: bool,
    ) -> Result<WorkflowSubscription, WorkflowError> {
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let subscription = self
            .repository
            .subscription(subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow subscription {subscription_id}"))
            })?;
        if enabled {
            self.validate_subscription_trust(&subscription)?;
        }
        self.repository
            .set_subscription_enabled(
                subscription_id,
                enabled,
                &now,
                Actor {
                    actor_type: ActorType::User,
                    id: "workflow-subscription-operator".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Evaluate persisted subscriptions against bounded canonical journal work.
    pub async fn tick_subscriptions_now(
        &self,
    ) -> Result<Vec<WorkflowSubscriptionDispatch>, WorkflowError> {
        let subscriptions = self.repository.subscriptions(MAX_WORKFLOW_SUBSCRIPTIONS)?;
        let mut dispatches = Vec::new();
        let mut queued = 0_usize;
        for subscription in subscriptions
            .into_iter()
            .filter(|subscription| subscription.enabled)
        {
            if queued >= MAX_SUBSCRIPTION_DISPATCHES_PER_TICK {
                break;
            }
            let subscription_id = subscription.subscription_id.clone();
            let checkpoint = subscription.checkpoint;
            match self.tick_subscription(subscription).await {
                Ok(Some(dispatch)) => {
                    if dispatch.status == WorkflowSubscriptionDispatchStatus::Queued {
                        queued = queued.saturating_add(1);
                    }
                    dispatches.push(dispatch);
                }
                Ok(None) => {}
                Err(WorkflowError::Effect(_) | WorkflowError::OutcomeUnknown(_)) => {
                    dispatches.push(WorkflowSubscriptionDispatch {
                        subscription_id,
                        status: WorkflowSubscriptionDispatchStatus::Deferred,
                        checkpoint,
                        source_event_id: None,
                        run_id: None,
                        reason: Some(
                            "policy-controlled dispatch did not complete; source remains pending"
                                .into(),
                        ),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(dispatches)
    }

    pub(super) async fn tick_subscription(
        &self,
        subscription: WorkflowSubscription,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let events = self.journal.read_global(
            subscription.checkpoint.saturating_add(1),
            MAX_SUBSCRIPTION_SCAN_EVENTS,
        )?;
        if events.is_empty() {
            return Ok(None);
        }
        let matching = events
            .iter()
            .find(|event| subscription_matches(&subscription, event))
            .cloned();
        let Some(source) = matching else {
            let domain_seen = events
                .iter()
                .any(|event| event.classification == EventClassification::Domain);
            if !domain_seen && events.len() < MAX_SUBSCRIPTION_SCAN_EVENTS {
                return Ok(None);
            }
            let checkpoint = events
                .last()
                .map(|event| event.global_sequence)
                .unwrap_or(subscription.checkpoint);
            return self.advance_subscription_checkpoint(&subscription, checkpoint);
        };

        if let Some(delivery) = self
            .repository
            .subscription_delivery(&subscription.subscription_id, &source.event_id)?
        {
            return self.acknowledge_duplicate_subscription(&subscription, &source, &delivery);
        }

        let inputs = self.subscription_inputs(&subscription, &source)?;
        let definition = match self.validate_subscription_trust(&subscription) {
            Ok(definition) => definition,
            Err(WorkflowError::Store(error)) => return Err(error.into()),
            Err(_) => {
                return self.block_subscription(
                    &subscription,
                    &source,
                    "pinned workflow definition or call graph is no longer trusted",
                );
            }
        };
        if let Err(error) = validate_instance(&definition.inputs, &inputs, "subscription input") {
            if let WorkflowError::Store(error) = error {
                return Err(error.into());
            }
            return self.block_subscription(
                &subscription,
                &source,
                "source event does not satisfy the pinned workflow input schema",
            );
        }
        let run_id = subscription_run_id(&subscription.subscription_id, &source.event_id);
        let dispatch = self
            .effects
            .run(WorkflowEffect {
                kind: "workflow".into(),
                action: "workflow.subscription.dispatch".into(),
                content: json!({
                    "subscription_id": subscription.subscription_id,
                    "workflow_name": subscription.workflow_name,
                    "workflow_version": subscription.workflow_version,
                    "event": inputs["event"].clone(),
                    "idempotency_key": inputs["idempotency_key"].clone(),
                }),
                idempotency: Some(format!(
                    "subscription:{}:{}",
                    subscription.subscription_id, source.event_id
                )),
                credential_references: Vec::new(),
                allowed_tools: Vec::new(),
                run_id: run_id.clone(),
                step_id: "$subscription".into(),
                definition_step_id: "$subscription".into(),
                workflow_hash: subscription.workflow_hash.clone(),
                attempt: 1,
                compensation: false,
            })
            .await;
        if let Err(error) = dispatch {
            return match error {
                WorkflowError::Effect(_) | WorkflowError::OutcomeUnknown(_) => {
                    Ok(Some(WorkflowSubscriptionDispatch {
                        subscription_id: subscription.subscription_id,
                        status: WorkflowSubscriptionDispatchStatus::Deferred,
                        checkpoint: subscription.checkpoint,
                        source_event_id: Some(source.event_id),
                        run_id: None,
                        reason: Some(
                            "policy-controlled dispatch did not complete; source remains pending"
                                .into(),
                        ),
                    }))
                }
                error => Err(error),
            };
        }

        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        let persisted = self
            .journal
            .read_global(source.global_sequence, 1)?
            .into_iter()
            .next()
            .filter(|event| event.event_id == source.event_id)
            .ok_or_else(|| {
                WorkflowError::InvalidTransition(
                    "subscription source event changed during authorization".into(),
                )
            })?;
        if !subscription_matches(&current, &persisted) {
            return Err(WorkflowError::InvalidTransition(
                "subscription filter changed during authorization".into(),
            ));
        }
        if let Some(delivery) = self
            .repository
            .subscription_delivery(&current.subscription_id, &persisted.event_id)?
        {
            return self.acknowledge_duplicate_subscription_locked(
                &mut current,
                &persisted,
                &delivery,
            );
        }
        let current_inputs = self.subscription_inputs(&current, &persisted)?;
        let current_definition = self.validate_subscription_trust(&current)?;
        validate_instance(
            &current_definition.inputs,
            &current_inputs,
            "subscription input",
        )?;
        if self.repository.run(&run_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "deterministic subscription run {run_id} already exists without its delivery receipt"
            )));
        }
        let delivered_at = format_schedule_time(OffsetDateTime::now_utc())?;
        current.checkpoint = persisted.global_sequence;
        current.last_event_id = Some(persisted.event_id.clone());
        current.last_run_id = Some(run_id.clone());
        current.blocked_reason = None;
        current.updated_at = delivered_at.clone();
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        let delivery = WorkflowSubscriptionDelivery {
            subscription_id: current.subscription_id.clone(),
            source_event_id: persisted.event_id.clone(),
            source_global_sequence: persisted.global_sequence,
            delivered_at,
            run_id: run_id.clone(),
        };
        self.journal.append_batch(vec![
            subscription_event(
                &current,
                expected_stream_version,
                "workflow.subscription.delivered.v1",
                json!({"record": &current, "delivery": &delivery}),
            ),
            subscription_delivery_event(&current, &delivery),
            subscription_run_event(&current, &persisted.event_id, &run_id, current_inputs),
        ])?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id,
            status: WorkflowSubscriptionDispatchStatus::Queued,
            checkpoint: persisted.global_sequence,
            source_event_id: Some(persisted.event_id),
            run_id: Some(run_id),
            reason: None,
        }))
    }

    pub(super) fn subscription_inputs(
        &self,
        subscription: &WorkflowSubscription,
        event: &EventEnvelope,
    ) -> Result<Value, WorkflowError> {
        let payload = self.journal.decrypt_payload(event)?;
        Ok(json!({
            "subscription_id": subscription.subscription_id,
            "idempotency_key": format!(
                "subscription:{}:{}",
                subscription.subscription_id, event.event_id
            ),
            "event": {
                "event_id": event.event_id,
                "global_sequence": event.global_sequence,
                "stream_id": event.stream_id,
                "stream_version": event.stream_version,
                "classification": event.classification,
                "event_type": event.event_type,
                "actor": event.actor,
                "context": event.context,
                "occurred_at": event.occurred_at,
                "payload": payload,
            },
        }))
    }

    pub(super) fn validate_subscription_trust(
        &self,
        subscription: &WorkflowSubscription,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let (definition, current_hash) = self
            .repository
            .definition(&subscription.workflow_name, &subscription.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(subscription.workflow_name.clone()))?;
        if current_hash != subscription.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "subscription pinned workflow definition changed".into(),
            ));
        }
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        Ok(definition)
    }

    pub(super) fn advance_subscription_checkpoint(
        &self,
        subscription: &WorkflowSubscription,
        checkpoint: u64,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        current.checkpoint = checkpoint;
        current.updated_at = format_schedule_time(OffsetDateTime::now_utc())?;
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        self.journal.append(subscription_event(
            &current,
            expected_stream_version,
            "workflow.subscription.checkpointed.v1",
            json!({"record": &current}),
        ))?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id,
            status: WorkflowSubscriptionDispatchStatus::Checkpointed,
            checkpoint,
            source_event_id: None,
            run_id: None,
            reason: None,
        }))
    }

    pub(super) fn acknowledge_duplicate_subscription(
        &self,
        subscription: &WorkflowSubscription,
        source: &EventEnvelope,
        delivery: &WorkflowSubscriptionDelivery,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        self.acknowledge_duplicate_subscription_locked(&mut current, source, delivery)
    }

    pub(super) fn acknowledge_duplicate_subscription_locked(
        &self,
        current: &mut WorkflowSubscription,
        source: &EventEnvelope,
        delivery: &WorkflowSubscriptionDelivery,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let expected_run_id = subscription_run_id(&current.subscription_id, &source.event_id);
        let run = self.repository.run(&delivery.run_id)?.ok_or_else(|| {
            StoreError::Verification(format!(
                "subscription delivery {}:{} has no queued workflow run",
                current.subscription_id, source.event_id
            ))
        })?;
        if delivery.source_global_sequence != source.global_sequence
            || delivery.run_id != expected_run_id
            || run.trigger_kind != Some(WorkflowTriggerKind::Subscription)
            || run.trigger_id.as_deref() != Some(current.subscription_id.as_str())
            || run.trigger_occurrence.as_deref() != Some(source.event_id.as_str())
        {
            return Err(StoreError::Verification(format!(
                "subscription delivery {}:{} does not match its source event and run",
                current.subscription_id, source.event_id
            ))
            .into());
        }
        current.checkpoint = source.global_sequence;
        current.last_event_id = Some(source.event_id.clone());
        current.last_run_id = Some(delivery.run_id.clone());
        current.updated_at = format_schedule_time(OffsetDateTime::now_utc())?;
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        self.journal.append(subscription_event(
            current,
            expected_stream_version,
            "workflow.subscription.duplicate_acknowledged.v1",
            json!({"record": &current, "delivery": delivery}),
        ))?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id.clone(),
            status: WorkflowSubscriptionDispatchStatus::Duplicate,
            checkpoint: source.global_sequence,
            source_event_id: Some(source.event_id.clone()),
            run_id: Some(delivery.run_id.clone()),
            reason: None,
        }))
    }

    pub(super) fn block_subscription(
        &self,
        subscription: &WorkflowSubscription,
        source: &EventEnvelope,
        reason: &str,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        current.enabled = false;
        current.blocked_reason = Some(reason.into());
        current.updated_at = format_schedule_time(OffsetDateTime::now_utc())?;
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        self.journal.append(subscription_event(
            &current,
            expected_stream_version,
            "workflow.subscription.blocked.v1",
            json!({
                "record": &current,
                "reason": reason,
                "source_event_id": source.event_id,
                "source_global_sequence": source.global_sequence,
            }),
        ))?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id,
            status: WorkflowSubscriptionDispatchStatus::Blocked,
            checkpoint: current.checkpoint,
            source_event_id: Some(source.event_id.clone()),
            run_id: None,
            reason: Some(reason.into()),
        }))
    }

    pub(super) fn subscription_version(&self, subscription_id: &str) -> Result<u64, StoreError> {
        u64::try_from(
            self.journal
                .read_stream(&subscription_stream(subscription_id))?
                .len(),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
    }
}
