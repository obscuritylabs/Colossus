use super::*;

impl WorkflowService {
    /// Create one bounded, hash-pinned authenticated workflow webhook.
    #[allow(clippy::too_many_arguments)]
    pub fn create_webhook(
        &self,
        webhook_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        secret_reference: &str,
        replay_window_seconds: u64,
        max_body_bytes: u64,
        enabled: bool,
    ) -> Result<WorkflowWebhook, WorkflowError> {
        if webhook_id.is_empty()
            || webhook_id.len() > MAX_WEBHOOK_ID_BYTES
            || !valid_name(webhook_id)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook id must contain 1..={MAX_WEBHOOK_ID_BYTES} lowercase letters, digits, dots, or hyphens"
            )));
        }
        if !valid_environment_reference(secret_reference) {
            return Err(WorkflowError::InvalidDefinition(
                "webhook secret must use an env:VARIABLE credential reference".into(),
            ));
        }
        if !(MIN_WEBHOOK_REPLAY_WINDOW_SECONDS..=MAX_WEBHOOK_REPLAY_WINDOW_SECONDS)
            .contains(&replay_window_seconds)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook replay window must be between {MIN_WEBHOOK_REPLAY_WINDOW_SECONDS} and {MAX_WEBHOOK_REPLAY_WINDOW_SECONDS} seconds"
            )));
        }
        if !(1..=MAX_WEBHOOK_BODY_BYTES).contains(&max_body_bytes) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook body limit must be between 1 and {MAX_WEBHOOK_BODY_BYTES} bytes"
            )));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self.repository.webhook(webhook_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow webhook {webhook_id} already exists"
            )));
        }
        if self.repository.webhooks(MAX_WORKFLOW_WEBHOOKS)?.len() >= MAX_WORKFLOW_WEBHOOKS {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow webhook limit {MAX_WORKFLOW_WEBHOOKS} is exhausted"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(workflow_name, workflow_version)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("{workflow_name}:{workflow_version}"))
            })?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let webhook = WorkflowWebhook {
            webhook_id: webhook_id.into(),
            workflow_name: workflow_name.into(),
            workflow_version: workflow_version.into(),
            workflow_hash,
            secret_reference: secret_reference.into(),
            enabled,
            replay_window_seconds,
            max_body_bytes,
            blocked_reason: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repository.create_webhook(
            &webhook,
            Actor {
                actor_type: ActorType::User,
                id: "workflow-webhook-registrar".into(),
            },
        )?;
        Ok(webhook)
    }

    /// Reconstruct one canonical workflow webhook.
    pub fn get_webhook(&self, webhook_id: &str) -> Result<WorkflowWebhook, WorkflowError> {
        self.repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))
    }

    /// List bounded workflow webhooks in deterministic identifier order.
    pub fn list_webhooks(&self, limit: usize) -> Result<Vec<WorkflowWebhook>, WorkflowError> {
        self.repository
            .webhooks(limit.min(MAX_WORKFLOW_WEBHOOKS))
            .map_err(Into::into)
    }

    /// Explicitly enable or disable one webhook after rechecking pinned trust.
    pub fn set_webhook_enabled(
        &self,
        webhook_id: &str,
        enabled: bool,
    ) -> Result<WorkflowWebhook, WorkflowError> {
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let webhook = self
            .repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if enabled {
            self.validate_webhook_trust(&webhook)?;
        }
        self.repository
            .set_webhook_enabled(
                webhook_id,
                enabled,
                &now,
                Actor {
                    actor_type: ActorType::User,
                    id: "workflow-webhook-operator".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Authenticate, authorize, and durably queue one webhook delivery.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_webhook(
        &self,
        webhook_id: &str,
        delivery_id: &str,
        timestamp: &str,
        signature: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
        secret: &[u8],
    ) -> Result<WorkflowWebhookDispatch, WorkflowError> {
        let received = OffsetDateTime::now_utc();
        self.ingest_webhook_at(
            webhook_id,
            delivery_id,
            timestamp,
            signature,
            headers,
            body,
            secret,
            received,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn ingest_webhook_at(
        &self,
        webhook_id: &str,
        delivery_id: &str,
        timestamp: &str,
        signature: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
        secret: &[u8],
        received: OffsetDateTime,
    ) -> Result<WorkflowWebhookDispatch, WorkflowError> {
        if delivery_id.is_empty() || delivery_id.len() > MAX_WEBHOOK_DELIVERY_ID_BYTES {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook delivery id must contain 1..={MAX_WEBHOOK_DELIVERY_ID_BYTES} bytes"
            )));
        }
        if delivery_id.chars().any(char::is_control) {
            return Err(WorkflowError::InvalidDefinition(
                "webhook delivery id cannot contain control characters".into(),
            ));
        }
        validate_webhook_headers(&headers)?;
        let webhook = self
            .repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if !webhook.enabled {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow webhook {webhook_id} is disabled"
            )));
        }
        let body_limit = usize::try_from(webhook.max_body_bytes)
            .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
        if body.is_empty() || body.len() > body_limit {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook body must contain 1..={} bytes",
                webhook.max_body_bytes
            )));
        }
        if secret.len() < 32 {
            return Err(WorkflowError::InvalidDefinition(
                "webhook HMAC secret must contain at least 32 bytes".into(),
            ));
        }
        let signed_at = parse_schedule_time(timestamp, "webhook timestamp")?;
        let age_seconds = (received - signed_at).whole_seconds().unsigned_abs();
        if age_seconds > webhook.replay_window_seconds {
            return Err(WorkflowError::InvalidTransition(
                "webhook timestamp is outside the configured replay window".into(),
            ));
        }
        verify_webhook_signature(timestamp, delivery_id, body, signature, secret)?;
        if self
            .repository
            .webhook_delivery(webhook_id, delivery_id)?
            .is_some()
        {
            return Err(WorkflowError::InvalidTransition(format!(
                "webhook delivery {delivery_id} was already accepted"
            )));
        }
        if let Err(error) = self.validate_webhook_trust(&webhook) {
            if !matches!(&error, WorkflowError::Store(_)) {
                self.block_webhook(
                    webhook_id,
                    "pinned workflow definition or call graph is no longer trusted",
                    received,
                )?;
            }
            return Err(error);
        }
        let body_value: Value = serde_json::from_slice(body).map_err(|error| {
            WorkflowError::InvalidDefinition(format!("webhook body must be strict JSON: {error}"))
        })?;
        let inputs = json!({
            "body": body_value,
            "delivery_id": delivery_id,
            "headers": headers,
            "timestamp": timestamp,
        });
        let (definition, _) = self
            .repository
            .definition(&webhook.workflow_name, &webhook.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(webhook.workflow_name.clone()))?;
        validate_instance(&definition.inputs, &inputs, "webhook input")?;
        let run_id = webhook_run_id(webhook_id, delivery_id);
        let body_sha256 = hex::encode(Sha256::digest(body));
        let secret_hash = hex::encode(Sha256::digest(secret));
        self.effects
            .run(WorkflowEffect {
                kind: "workflow".into(),
                action: "workflow.webhook.ingest".into(),
                content: json!({
                    "webhook_id": webhook_id,
                    "delivery_id": delivery_id,
                    "timestamp": timestamp,
                    "headers": inputs["headers"].clone(),
                    "body": inputs["body"].clone(),
                    "body_bytes": body.len(),
                    "body_sha256": body_sha256,
                    "replay_window_seconds": webhook.replay_window_seconds,
                    "workflow_name": webhook.workflow_name,
                    "workflow_version": webhook.workflow_version,
                }),
                idempotency: Some(format!("webhook:{webhook_id}:{delivery_id}")),
                credential_references: vec![CredentialReference {
                    reference: webhook.secret_reference.clone(),
                    value_hash: Some(secret_hash),
                }],
                allowed_tools: Vec::new(),
                run_id: run_id.clone(),
                step_id: "$webhook".into(),
                definition_step_id: "$webhook".into(),
                workflow_hash: webhook.workflow_hash.clone(),
                attempt: 1,
                compensation: false,
            })
            .await?;

        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self
            .repository
            .webhook_delivery(webhook_id, delivery_id)?
            .is_some()
        {
            return Err(WorkflowError::InvalidTransition(format!(
                "webhook delivery {delivery_id} was already accepted"
            )));
        }
        let current = self
            .repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if !current.enabled || current != webhook {
            return Err(WorkflowError::InvalidTransition(
                "webhook configuration changed during authorization; retry with current state"
                    .into(),
            ));
        }
        self.validate_webhook_trust(&current)?;
        if self.repository.run(&run_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "deterministic webhook run {run_id} already exists"
            )));
        }
        let received_at = format_schedule_time(received)?;
        let delivery = WorkflowWebhookDelivery {
            webhook_id: webhook_id.into(),
            delivery_id: delivery_id.into(),
            timestamp: timestamp.into(),
            received_at,
            body_sha256,
            run_id: run_id.clone(),
        };
        self.journal.append_batch(vec![
            webhook_delivery_event(&current, &delivery),
            webhook_run_event(&current, &run_id, delivery_id, inputs),
        ])?;
        Ok(WorkflowWebhookDispatch {
            delivery,
            run: self.get_run(&run_id)?,
        })
    }

    pub(super) fn validate_webhook_trust(
        &self,
        webhook: &WorkflowWebhook,
    ) -> Result<(), WorkflowError> {
        let (definition, current_hash) = self
            .repository
            .definition(&webhook.workflow_name, &webhook.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(webhook.workflow_name.clone()))?;
        if current_hash != webhook.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "webhook pinned workflow definition changed".into(),
            ));
        }
        validate_call_graph(self.repository.as_ref(), &definition, true)
    }

    pub(super) fn block_webhook(
        &self,
        webhook_id: &str,
        reason: &str,
        now: OffsetDateTime,
    ) -> Result<(), WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let Some(mut webhook) = self.repository.webhook(webhook_id)? else {
            return Ok(());
        };
        if !webhook.enabled {
            return Ok(());
        }
        webhook.enabled = false;
        webhook.blocked_reason = Some(reason.into());
        webhook.updated_at = format_schedule_time(now)?;
        let stream_id = webhook_stream(webhook_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: "workflow.webhook.blocked.v1".into(),
            actor: Actor {
                actor_type: ActorType::Workflow,
                id: webhook_id.into(),
            },
            context: ExecutionContext {
                correlation_id: webhook_id.into(),
                workflow_id: Some(webhook_id.into()),
                workflow_hash: Some(webhook.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": webhook, "reason": reason}),
        })?;
        Ok(())
    }
}
