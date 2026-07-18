use super::*;

pub(super) struct TraceToolExecutor {
    pub(super) journal: Arc<dyn EventJournal>,
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) filesystem: Arc<FilesystemExecutor>,
    pub(super) workspace: PathBuf,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for TraceToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        if !matches!(call.name.as_str(), "trace.show" | "trace.export") {
            return self.inner.execute(call, context).await;
        }
        let run_id = context
            .run_id
            .as_deref()
            .ok_or_else(|| ToolError::Denied("trace tools require an active run".into()))?;
        let default_limit = if call.name == "trace.show" { 200 } else { 500 };
        let limit =
            usize::try_from(optional_tool_u64(&call, "max_events")?.unwrap_or(default_limit))
                .unwrap_or(1_000)
                .clamp(1, 1_000);
        let snapshot = trace_snapshot(self.journal.as_ref(), run_id, limit)?;
        let output = if call.name == "trace.show" {
            serde_json::to_string(&snapshot)
                .map_err(|error| ToolError::Failed(error.to_string()))?
        } else {
            let path = model_workspace_path(&self.workspace, required_tool_string(&call, "path")?)?;
            let display_path = workspace_relative(&self.workspace, &path)?;
            let text = serde_json::to_string_pretty(&snapshot)
                .map_err(|error| ToolError::Failed(error.to_string()))?;
            let mut request = effect_request(
                model_actor(&call, &context),
                "trace.export",
                path.display().to_string(),
                json!({
                    "operation": "write",
                    "display_path": display_path,
                    "text": text,
                    "mode": "overwrite",
                }),
            );
            request.capabilities = vec!["trace.export".into()];
            request.context = context;
            let result = self
                .gateway
                .execute(request, self.filesystem.as_ref())
                .await
                .map_err(tool_gateway_error)?;
            String::from_utf8(result.bytes)
                .map_err(|_| ToolError::Failed("trace export result is non-UTF-8".into()))?
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: bounded_tool_text(&output, 1024 * 1024),
            exit_code: 0,
        })
    }
}

pub(super) fn trace_snapshot(
    journal: &dyn EventJournal,
    run_id: &str,
    limit: usize,
) -> Result<Value, ToolError> {
    let events = journal
        .read_stream(&format!("run:{run_id}"))
        .map_err(|error| ToolError::Failed(error.to_string()))?;
    let truncated = events.len() > limit;
    let start = events.len().saturating_sub(limit);
    let events = events[start..]
        .iter()
        .map(|event| {
            json!({
                "event_id": event.event_id,
                "global_sequence": event.global_sequence,
                "stream_version": event.stream_version,
                "event_type": event.event_type,
                "classification": event.classification,
                "actor": event.actor,
                "context": event.context,
                "occurred_at": event.occurred_at,
                "payload_hash": event.payload.plaintext_hash,
                "record_hash": event.record_hash,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "available": !events.is_empty(),
        "run_id": run_id,
        "events": events,
        "truncated": truncated,
    }))
}
