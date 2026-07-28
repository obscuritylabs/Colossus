use super::*;

pub(super) struct SubagentSchedulingToolExecutor {
    pub(super) notify: Arc<Notify>,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for SubagentSchedulingToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let delegated = call.name == "agent.delegate";
        let result = self.inner.execute(call, context).await?;
        if delegated && result.exit_code == 0 {
            self.notify.notify_one();
            // Give the owning runtime turn a scheduling point before the parent asks for
            // the child result or emits a final answer based only on the queued snapshot.
            tokio::task::yield_now().await;
        }
        Ok(result)
    }
}

pub(super) struct DiscoverableToolExecutor {
    pub(super) registry: Arc<dyn ToolRegistry>,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for DiscoverableToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        if call.name != "tool.search" {
            return self.inner.execute(call, context).await;
        }
        let query = required_tool_string(&call, "query")?
            .trim()
            .to_ascii_lowercase();
        let terms = query.split_whitespace().collect::<Vec<_>>();
        let limit = usize::try_from(optional_tool_u64(&call, "max_results")?.unwrap_or(10))
            .unwrap_or(50)
            .clamp(1, 50);
        let offered = context.offered_tools.iter().collect::<BTreeSet<_>>();
        let mut matches = self
            .registry
            .list_specs()
            .into_iter()
            .filter(|spec| offered.contains(&spec.name))
            .filter_map(|spec| {
                let name = spec.name.to_ascii_lowercase();
                let description = spec.description.to_ascii_lowercase();
                if !terms
                    .iter()
                    .all(|term| name.contains(term) || description.contains(term))
                {
                    return None;
                }
                let score = usize::from(name == query) * 1_000
                    + usize::from(name.contains(&query)) * 500
                    + terms.iter().filter(|term| name.contains(**term)).count() * 50
                    + terms
                        .iter()
                        .filter(|term| description.contains(**term))
                        .count()
                        * 10;
                Some((score, spec))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        let truncated = matches.len() > limit;
        matches.truncate(limit);
        let tools = matches
            .into_iter()
            .map(|(score, spec)| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "effect_action": spec.effect_action,
                    "capability": spec.capability,
                    "score": score,
                })
            })
            .collect::<Vec<_>>();
        let output = serde_json::to_string(&json!({
            "query": query,
            "tools": tools,
            "truncated": truncated,
        }))
        .map_err(|error| ToolError::Failed(error.to_string()))?;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: bounded_tool_text(&output, 256 * 1024),
            exit_code: 0,
        })
    }
}

pub(super) struct InteractiveToolExecutor {
    pub(super) prompts: Arc<dyn UserPromptProvider>,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for InteractiveToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        if call.name != "user.ask" {
            return self.inner.execute(call, context).await;
        }
        let choices = optional_tool_string_array(&call, "choices")?.unwrap_or_default();
        let allow_free_form = optional_tool_bool(&call, "allow_free_form")?.unwrap_or(true);
        if choices.is_empty() && !allow_free_form {
            return Err(ToolError::InvalidArguments {
                tool: call.name,
                message: "user.ask requires choices when free-form answers are disabled".into(),
            });
        }
        let response = self
            .prompts
            .prompt(UserPromptRequest {
                question: required_tool_string(&call, "question")?.into(),
                choices: choices.clone(),
                allow_free_form,
            })
            .await?;
        if response.answer.is_empty()
            || response.answer.len() > 64 * 1024
            || response
                .selected_index
                .is_some_and(|index| choices.get(index) != Some(&response.answer))
            || (!allow_free_form && !choices.iter().any(|choice| choice == &response.answer))
        {
            return Err(ToolError::Failed(
                "interactive prompt returned an invalid or out-of-contract answer".into(),
            ));
        }
        let output = serde_json::to_string(&response)
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: bounded_tool_text(&output, 64 * 1024),
            exit_code: 0,
        })
    }
}
