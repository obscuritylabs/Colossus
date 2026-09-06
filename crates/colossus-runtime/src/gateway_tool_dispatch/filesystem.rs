use super::*;

impl GatewayToolExecutor {
    pub(super) async fn execute_filesystem(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let exit_code = 0;
        let output = match call.name.as_str() {
            "echo" => bounded_tool_text(required_tool_string(&call, "text")?, 32_768),
            "filesystem.list" => {
                let input = optional_tool_string(&call, "path")?.unwrap_or(".");
                let ambient = self.danger_full_access(&context);
                let path = self.model_read_path(input, &context, ambient)?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "filesystem.list",
                    path.display().to_string(),
                    json!({}),
                );
                request.capabilities = vec!["filesystem.list".into()];
                request.context = context.clone();
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                let value: Value = serde_json::from_slice(&result.bytes)
                    .map_err(|error| ToolError::Failed(error.to_string()))?;
                let entries = value
                    .get("entries")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        ToolError::Failed("filesystem.list returned invalid JSON".into())
                    })?;
                let entries = entries
                    .iter()
                    .filter(|entry| {
                        ambient
                            || !entry
                                .get("name")
                                .and_then(Value::as_str)
                                .is_some_and(|name| matches!(name, ".colossus" | ".git"))
                    })
                    .map(|entry| {
                        let mut entry = entry.clone();
                        let name = entry.get("name").and_then(Value::as_str).ok_or_else(|| {
                            ToolError::Failed("filesystem.list entry name is absent".into())
                        })?;
                        entry["path"] = Value::String(display_resource_path(
                            &self.workspace,
                            &path.join(name),
                            ambient,
                        )?);
                        Ok(entry)
                    })
                    .collect::<Result<Vec<_>, ToolError>>()?;
                serde_json::to_string(&json!({
                        "root": self.display_read_path(&path, &context, ambient)?,
                    "entries": entries,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "filesystem.read" => {
                let ambient = self.danger_full_access(&context);
                let path =
                    self.model_read_path(required_tool_string(&call, "path")?, &context, ambient)?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "filesystem.read",
                    path.display().to_string(),
                    json!({"path": path}),
                );
                request.capabilities = vec!["filesystem.read".into()];
                request.context = context.clone();
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                bounded_tool_text(
                    &String::from_utf8(result.bytes).map_err(|_| {
                        ToolError::Failed("filesystem.read returned non-UTF-8".into())
                    })?,
                    1024 * 1024,
                )
            }
            "filesystem.search" => {
                let input = optional_tool_string(&call, "path")?.unwrap_or(".");
                let ambient = self.danger_full_access(&context);
                let path = self.model_read_path(input, &context, ambient)?;
                let content = json!({
                    "pattern": required_tool_string(&call, "pattern")?,
                    "glob": optional_tool_string(&call, "glob")?,
                    "regex": optional_tool_bool(&call, "regex")?.unwrap_or(true),
                    "case_sensitive": optional_tool_bool(&call, "case_sensitive")?.unwrap_or(true),
                    "max_matches": optional_tool_u64(&call, "max_matches")?.unwrap_or(100),
                });
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "filesystem.search",
                    path.display().to_string(),
                    content,
                );
                request.capabilities = vec!["filesystem.search".into()];
                request.context = context.clone();
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                let mut value: Value = serde_json::from_slice(&result.bytes)
                    .map_err(|error| ToolError::Failed(error.to_string()))?;
                let matches = value
                    .get_mut("matches")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| {
                        ToolError::Failed("filesystem.search returned invalid JSON".into())
                    })?;
                for matched in matches {
                    let relative = matched
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ToolError::Failed("search match path is absent".into()))?;
                    matched["path"] = Value::String(display_resource_path(
                        &self.workspace,
                        &path.join(relative),
                        ambient,
                    )?);
                }
                serde_json::to_string(&value)
                    .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "filesystem.write" => {
                let ambient = self.danger_full_access(&context);
                let path = model_resource_path(
                    &self.workspace,
                    required_tool_string(&call, "path")?,
                    ambient,
                )?;
                let display_path = display_resource_path(&self.workspace, &path, ambient)?;
                self.execute_filesystem_mutation(
                    &call,
                    context,
                    path,
                    json!({
                        "operation": "write",
                        "display_path": display_path,
                        "text": required_tool_string(&call, "content")?,
                        "mode": required_tool_string(&call, "mode")?,
                    }),
                )
                .await?
            }
            "filesystem.replace" => {
                let ambient = self.danger_full_access(&context);
                let path = model_resource_path(
                    &self.workspace,
                    required_tool_string(&call, "path")?,
                    ambient,
                )?;
                let display_path = display_resource_path(&self.workspace, &path, ambient)?;
                self.execute_filesystem_mutation(
                    &call,
                    context,
                    path,
                    json!({
                        "operation": "replace",
                        "display_path": display_path,
                        "old": required_tool_string(&call, "old")?,
                        "new": required_tool_string(&call, "new")?,
                        "replace_all": optional_tool_bool(&call, "replace_all")?.unwrap_or(false),
                    }),
                )
                .await?
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
