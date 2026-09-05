use super::*;

impl GatewayToolExecutor {
    pub(super) async fn execute_network(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let exit_code = 0;
        let output = match call.name.as_str() {
            "web.search" => {
                let query = required_tool_string(&call, "query")?.to_owned();
                let limit = usize::try_from(
                    optional_tool_u64(&call, "limit")?
                        .unwrap_or_else(|| u64::try_from(default_search_limit()).unwrap_or(10)),
                )
                .map_err(|_| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: "limit is too large".into(),
                })?;
                let response = self
                    .search
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("search provider is unavailable".into()))?
                    .search(
                        "agent",
                        model_actor(&call, &context),
                        SearchRequest { query, limit },
                        context,
                    )
                    .await
                    .map_err(search_tool_error)?;
                serde_json::to_string(&response)
                    .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "network.http" | "web.fetch" | "docs.fetch" => {
                let url = required_tool_string(&call, "url")?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "network.http",
                    url,
                    json!({"method": "GET", "headers": {"accept": "*/*"}}),
                );
                request.capabilities = vec!["network.http".into()];
                request.context = context.clone();
                let result = self
                    .gateway
                    .execute(request, self.http.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                bounded_tool_text(
                    &String::from_utf8(result.bytes)
                        .map_err(|_| ToolError::Failed("network.http returned non-UTF-8".into()))?,
                    1024 * 1024,
                )
            }
            name => return Err(ToolError::Unknown(name.into())),
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code,
        })
    }
}
